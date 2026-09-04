//! Optional GPU acceleration for the operations measurement says it helps.
//!
//! # Why this module is small, and why it is optional
//!
//! Phase 11 measured the alternatives on the machine it was built on (NVIDIA
//! RTX A5500, DX12 and Vulkan) before writing any of it. Two results shaped
//! what is here:
//!
//! * Per-pixel work — blends, tone adjustments — is bandwidth bound. A 45 MP
//!   float frame is 720 MB, and the best transfer path measured 5.5 GB/s up
//!   and 4.4 GB/s back, so a round trip costs about 300 ms while the kernel
//!   itself costs 11 ms. Against the eight-thread CPU compositor a document
//!   would need roughly seventeen chained per-pixel operations before the
//!   round trip paid for itself. PhotoForge documents do not have seventeen.
//! * Neighbourhood work is compute bound and does pay. A separable Gaussian
//!   reads `2*ceil(3*sigma)+1` taps per pixel per pass, and even after the CPU
//!   blur was parallelised the GPU is 1.5x to 3.0x faster end to end including
//!   both transfers.
//!
//! So this module accelerates the blur family and nothing else. Claiming more
//! would mean claiming acceleration that was measured not to exist.
//!
//! # It is a second implementation, not the definition
//!
//! `image_processing::high_precision` remains the correctness oracle. This
//! kernel is written to reproduce its arithmetic, and the equivalence tests
//! compare against it rather than against a previous GPU result. Whenever the
//! GPU cannot be used — no adapter, a device that vanished, a frame larger
//! than the device will bind, a driver that rejected the work — the answer is
//! `None` and the caller runs the CPU path, which is always present.
//!
//! # Security
//!
//! The shader is compiled into the binary with `include_str!`. Nothing is
//! downloaded, and no project, preset or user input can reach the shader
//! compiler: the only thing a caller supplies is an image and a sigma.
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::OnceLock;

use crate::color::{FloatImage, FloatRgba};
use crate::error::AppError;

const GAUSSIAN_WGSL: &str = include_str!("shaders/gaussian.wgsl");

/// Smallest frame worth sending to the GPU.
///
/// Below this the CPU blur already finishes in well under a tenth of a second,
/// and involving a second device buys latency risk for no useful gain. It also
/// keeps tiles out of the GPU path entirely: a 256-pixel tile is 65k pixels,
/// so a tiled render never reaches the device, which is deliberate — per-tile
/// transfer costs several times per-tile compute.
pub const MIN_GPU_PIXELS: u64 = 2_000_000;

/// Smallest blur radius worth sending to the GPU, in taps either side.
///
/// Measured at 24 MP on an RTX A5500 over Vulkan. GPU time is nearly flat in
/// the radius because it is transfer bound, while CPU time grows with it, so
/// the two cross at about eight taps:
///
/// | radius | CPU | GPU | gain |
/// | --- | --- | --- | --- |
/// | 3 | 416 ms | 469 ms | 0.89x |
/// | 6 | 460 ms | 482 ms | 0.96x |
/// | 9 | 519 ms | 487 ms | 1.07x |
/// | 18 | 704 ms | 490 ms | 1.44x |
/// | 36 | 1533 ms | 490 ms | 3.13x |
/// | 60 | 2676 ms | 513 ms | 5.22x |
///
/// Below this the GPU is measurably slower, so it is not used.
pub const MIN_GPU_RADIUS: i32 = 8;

/// What the GPU path has actually done, so claims about it can be checked.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct GpuStats {
    /// Operations that ran to completion on the device.
    pub dispatches: u64,
    /// Megapixels processed on the device, summed over operations.
    pub megapixels: u64,
    /// Operations declined before starting: too small, too large, no device.
    pub declined: u64,
    /// Operations that started on the device and failed, falling back to CPU.
    pub failures: u64,
}

static DISPATCHES: AtomicU64 = AtomicU64::new(0);
static MEGAPIXELS: AtomicU64 = AtomicU64::new(0);
static DECLINED: AtomicU64 = AtomicU64::new(0);
static FAILURES: AtomicU64 = AtomicU64::new(0);
/// Latches once the device has failed, so a broken device is not retried on
/// every operation for the rest of the session.
static UNHEALTHY: AtomicBool = AtomicBool::new(false);

pub fn stats() -> GpuStats {
    GpuStats {
        dispatches: DISPATCHES.load(Ordering::Relaxed),
        megapixels: MEGAPIXELS.load(Ordering::Relaxed),
        declined: DECLINED.load(Ordering::Relaxed),
        failures: FAILURES.load(Ordering::Relaxed),
    }
}

/// Which device the GPU path is using, for the diagnostics panel.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct GpuInfo {
    pub backend: String,
    pub adapter: String,
    pub device_type: String,
    pub driver: String,
    pub max_buffer_bytes: u64,
    /// False once the device has failed; the CPU path is being used instead.
    pub healthy: bool,
}

struct Context {
    device: wgpu::Device,
    queue: wgpu::Queue,
    pipeline: wgpu::ComputePipeline,
    info: wgpu::AdapterInfo,
    max_binding_bytes: u64,
    max_workgroups: u32,
}

// SAFETY-adjacent note: wgpu's Device and Queue are Send + Sync, so one
// context is shared by every caller rather than one per operation. Creating a
// device costs on the order of a hundred milliseconds and must not happen per
// blur.
static CONTEXT: OnceLock<Option<Context>> = OnceLock::new();

fn context() -> Option<&'static Context> {
    CONTEXT.get_or_init(initialise).as_ref()
}

/// Set once from the environment, so a user, a support request or a benchmark
/// can take the GPU out of the picture without a different build.
///
/// Read once: a device cannot be turned on and off underneath a render that is
/// already deciding where its work goes.
static DISABLED: OnceLock<bool> = OnceLock::new();

fn disabled() -> bool {
    *DISABLED.get_or_init(|| {
        std::env::var("PHOTOFORGE_DISABLE_GPU")
            .map(|value| value != "0" && !value.is_empty())
            .unwrap_or(false)
    })
}

fn initialise() -> Option<Context> {
    // Vulkan only, and not by preference. DX12 uploaded at 5.5 GB/s against
    // Vulkan's 2.7 GB/s on this machine, but wgpu's DX12 backend cannot be
    // compiled in this dependency graph — see the note in Cargo.toml — so it is
    // not built and cannot be selected. The measured cost is roughly half the
    // transfer throughput.
    initialise_backend(wgpu::Backends::VULKAN)
}

fn initialise_backend(backends: wgpu::Backends) -> Option<Context> {
    let instance = wgpu::Instance::new(wgpu::InstanceDescriptor {
        backends,
        ..wgpu::InstanceDescriptor::new_without_display_handle()
    });
    let adapter = pollster::block_on(instance.request_adapter(&wgpu::RequestAdapterOptions {
        power_preference: wgpu::PowerPreference::HighPerformance,
        force_fallback_adapter: false,
        compatible_surface: None,
        apply_limit_buckets: false,
    }))
    .ok()?;
    let info = adapter.get_info();
    // A software rasteriser is slower than the CPU path it would replace, and
    // reporting it as GPU acceleration would be a lie.
    if info.device_type == wgpu::DeviceType::Cpu {
        return None;
    }
    let limits = adapter.limits();
    let (device, queue) = pollster::block_on(adapter.request_device(&wgpu::DeviceDescriptor {
        label: Some("photoforge"),
        required_features: wgpu::Features::empty(),
        required_limits: limits.clone(),
        memory_hints: wgpu::MemoryHints::Performance,
        trace: wgpu::Trace::Off,
        experimental_features: Default::default(),
    }))
    .ok()?;
    // Without this a validation error or a lost device aborts the process.
    // An image editor must lose the acceleration, not the user's work.
    device.on_uncaptured_error(std::sync::Arc::new(|error| {
        UNHEALTHY.store(true, Ordering::Release);
        FAILURES.fetch_add(1, Ordering::Relaxed);
        let _ = error;
    }));

    let module = device.create_shader_module(wgpu::ShaderModuleDescriptor {
        label: Some("gaussian"),
        source: wgpu::ShaderSource::Wgsl(GAUSSIAN_WGSL.into()),
    });
    let pipeline = device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
        label: Some("gaussian"),
        layout: None,
        module: &module,
        entry_point: Some("main"),
        compilation_options: Default::default(),
        cache: None,
    });
    if UNHEALTHY.load(Ordering::Acquire) {
        // The shader failed to compile on this driver.
        return None;
    }
    Some(Context {
        device,
        queue,
        pipeline,
        info,
        max_binding_bytes: limits.max_storage_buffer_binding_size,
        max_workgroups: limits.max_compute_workgroups_per_dimension,
    })
}

/// Whether a usable device was found. Initialises it on the first call.
pub fn is_available() -> bool {
    !disabled() && context().is_some() && !UNHEALTHY.load(Ordering::Acquire)
}

pub fn info() -> Option<GpuInfo> {
    let context = context()?;
    Some(GpuInfo {
        backend: format!("{:?}", context.info.backend),
        adapter: context.info.name.clone(),
        device_type: format!("{:?}", context.info.device_type),
        driver: format!("{} {}", context.info.driver, context.info.driver_info)
            .trim()
            .to_string(),
        max_buffer_bytes: context.max_binding_bytes,
        healthy: !UNHEALTHY.load(Ordering::Acquire),
    })
}

/// Whether this frame and radius are worth sending to the device.
///
/// Public so the decision can be tested and reported rather than inferred from
/// a timing difference.
pub fn accepts(width: u32, height: u32, radius: i32) -> bool {
    if disabled() {
        return false;
    }
    let pixels = u64::from(width) * u64::from(height);
    pixels >= MIN_GPU_PIXELS && radius >= MIN_GPU_RADIUS
}

/// A separable Gaussian blur on the device.
///
/// `None` means the work was not done and the caller must run the CPU path:
/// no device, a frame the device will not bind, a radius too small to pay for
/// the transfer, or a driver that rejected the work. It is never an error —
/// the CPU path is always available and always correct.
pub fn gaussian(image: &FloatImage, sigma: f32) -> Option<FloatImage> {
    if !sigma.is_finite() || sigma <= 0.0 {
        return None;
    }
    let radius = (sigma * 3.0).ceil() as i32;
    let (width, height) = image.dimensions();
    if !accepts(width, height, radius) {
        DECLINED.fetch_add(1, Ordering::Relaxed);
        return None;
    }
    if UNHEALTHY.load(Ordering::Acquire) {
        DECLINED.fetch_add(1, Ordering::Relaxed);
        return None;
    }
    let context = context()?;
    let bytes = u64::from(width) * u64::from(height) * 16;
    if bytes > context.max_binding_bytes {
        DECLINED.fetch_add(1, Ordering::Relaxed);
        return None;
    }

    match dispatch(context, image, radius, sigma, width, height, bytes) {
        Some(result) if !UNHEALTHY.load(Ordering::Acquire) => {
            DISPATCHES.fetch_add(1, Ordering::Relaxed);
            MEGAPIXELS.fetch_add(
                u64::from(width) * u64::from(height) / 1_000_000,
                Ordering::Relaxed,
            );
            Some(result)
        }
        _ => {
            FAILURES.fetch_add(1, Ordering::Relaxed);
            None
        }
    }
}

#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
struct Params {
    width: u32,
    height: u32,
    radius: i32,
    horizontal: u32,
}

fn dispatch(
    context: &Context,
    image: &FloatImage,
    radius: i32,
    sigma: f32,
    width: u32,
    height: u32,
    bytes: u64,
) -> Option<FloatImage> {
    let device = &context.device;
    // Normalised on the host, exactly as the CPU reference does, so the two
    // sum the same numbers in the same order.
    let mut weights: Vec<f32> = (-i64::from(radius)..=i64::from(radius))
        .map(|i| (-(i * i) as f32 / (2.0 * sigma * sigma)).exp())
        .collect();
    let total: f32 = weights.iter().sum();
    if !total.is_finite() || total <= 0.0 {
        return None;
    }
    for weight in &mut weights {
        *weight /= total;
    }

    let source = device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("blur-source"),
        size: bytes,
        usage: wgpu::BufferUsages::STORAGE,
        mapped_at_creation: true,
    });
    // Writing straight into a mapped buffer. On the DX12 backend this measured
    // about twice the throughput of Queue::write_buffer; on Vulkan the two are
    // closer, and mapped is still the faster of the pair.
    source
        .slice(..)
        .get_mapped_range_mut()
        .ok()?
        .copy_from_slice(bytemuck::cast_slice(image.pixels()));
    source.unmap();

    let intermediate = device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("blur-intermediate"),
        size: bytes,
        usage: wgpu::BufferUsages::STORAGE,
        mapped_at_creation: false,
    });
    let target = device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("blur-target"),
        size: bytes,
        usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_SRC,
        mapped_at_creation: false,
    });
    let staging = device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("blur-staging"),
        size: bytes,
        usage: wgpu::BufferUsages::MAP_READ | wgpu::BufferUsages::COPY_DST,
        mapped_at_creation: false,
    });
    let weight_bytes = (weights.len() * 4) as u64;
    let weight_buffer = device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("blur-weights"),
        size: weight_bytes,
        usage: wgpu::BufferUsages::STORAGE,
        mapped_at_creation: true,
    });
    weight_buffer
        .slice(..)
        .get_mapped_range_mut()
        .ok()?
        .copy_from_slice(bytemuck::cast_slice(&weights));
    weight_buffer.unmap();

    let params = |horizontal: u32| {
        let buffer = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("blur-params"),
            size: std::mem::size_of::<Params>() as u64,
            usage: wgpu::BufferUsages::UNIFORM,
            mapped_at_creation: true,
        });
        buffer
            .slice(..)
            .get_mapped_range_mut()
            .ok()?
            .copy_from_slice(bytemuck::bytes_of(&Params {
                width,
                height,
                radius,
                horizontal,
            }));
        buffer.unmap();
        Some(buffer)
    };
    let horizontal_params = params(1)?;
    let vertical_params = params(0)?;

    let layout = context.pipeline.get_bind_group_layout(0);
    let bind = |read: &wgpu::Buffer, write: &wgpu::Buffer, params: &wgpu::Buffer| {
        device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("blur"),
            layout: &layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: read.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: write.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 2,
                    resource: params.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 3,
                    resource: weight_buffer.as_entire_binding(),
                },
            ],
        })
    };
    let first = bind(&source, &intermediate, &horizontal_params);
    let second = bind(&intermediate, &target, &vertical_params);

    let groups_x = width.div_ceil(64);
    if groups_x > context.max_workgroups || height > context.max_workgroups {
        return None;
    }
    let mut encoder = device.create_command_encoder(&Default::default());
    for group in [&first, &second] {
        let mut pass = encoder.begin_compute_pass(&Default::default());
        pass.set_pipeline(&context.pipeline);
        pass.set_bind_group(0, group, &[]);
        pass.dispatch_workgroups(groups_x, height, 1);
    }
    encoder.copy_buffer_to_buffer(&target, 0, &staging, 0, bytes);
    context.queue.submit([encoder.finish()]);

    let slice = staging.slice(..);
    let (sender, receiver) = std::sync::mpsc::channel();
    slice.map_async(wgpu::MapMode::Read, move |result| {
        let _ = sender.send(result);
    });
    if context
        .device
        .poll(wgpu::PollType::wait_indefinitely())
        .is_err()
    {
        UNHEALTHY.store(true, Ordering::Release);
        return None;
    }
    receiver.recv().ok()?.ok()?;
    let view = slice.get_mapped_range().ok()?;
    let pixels: &[FloatRgba] = bytemuck::cast_slice(&view);
    let mut result = FloatImage::blank(width, height, FloatRgba::TRANSPARENT).ok()?;
    result.pixels_mut().copy_from_slice(pixels);
    drop(view);
    staging.unmap();
    // A device that returned rubbish must not reach the document.
    result.validate().ok()?;
    Some(result)
}

/// Marks the device unusable. Test-only: production sets this from a real
/// device error, and there is no supported way for a user to disable the GPU
/// mid-session other than the settings toggle, which is checked before this.
#[cfg(test)]
pub(crate) fn force_unhealthy_for_test(unhealthy: bool) {
    UNHEALTHY.store(unhealthy, Ordering::Release);
}

/// Turns a GPU result into the shape `high_precision` wants, or nothing.
pub(crate) fn try_gaussian(image: &FloatImage, sigma: f32) -> Option<Result<FloatImage, AppError>> {
    gaussian(image, sigma).map(Ok)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn frame(width: u32, height: u32) -> FloatImage {
        let mut image = FloatImage::blank(width, height, FloatRgba::TRANSPARENT).unwrap();
        for (index, pixel) in image.pixels_mut().iter_mut().enumerate() {
            let n = (index % 251) as f32 / 251.0;
            let m = (index % 97) as f32 / 97.0;
            *pixel = FloatRgba::new(n, 1.0 - n, m, 0.2 + m * 0.8);
        }
        image
    }

    /// The acceptance rule is the whole selection policy, so it is tested
    /// directly rather than inferred from whether something got faster.
    #[test]
    fn small_frames_and_small_radii_are_declined() {
        assert!(!accepts(100, 100, 12), "a tiny frame was accepted");
        assert!(!accepts(8000, 6000, 1), "a one-tap radius was accepted");
        // Radius 6 measured 0.96x: slower on the device than on the CPU.
        assert!(
            !accepts(8000, 6000, 6),
            "a radius the GPU loses at was accepted"
        );
        assert!(accepts(4000, 3000, 12), "a 12 MP wide blur was declined");
        // The tiled renderer's unit of work must never reach the device.
        assert!(
            !accepts(256, 256, 36),
            "a render tile was accepted; per-tile transfer costs more than per-tile compute"
        );
        assert!(!accepts(384, 384, 60), "a haloed render tile was accepted");
    }

    /// Nothing in this module may panic when there is no usable device: an
    /// image editor loses the acceleration, never the user's work.
    #[test]
    fn a_missing_device_is_not_an_error() {
        // Whether or not this machine has a GPU, both paths must be safe.
        let image = frame(64, 64);
        assert!(
            gaussian(&image, 4.0).is_none(),
            "a frame below the threshold should never be sent to the device"
        );
    }

    /// The equivalence gate. The CPU implementation is the definition; this
    /// asserts the GPU reproduces it, not that two GPU runs agree.
    #[test]
    fn a_gpu_blur_matches_the_cpu_reference() {
        if !is_available() {
            eprintln!("skipped: no usable GPU on this machine");
            return;
        }
        // Large enough to be accepted, small enough to keep the test quick.
        let image = frame(2000, 1100);
        for sigma in [3.0f32, 4.0, 9.0] {
            let Some(actual) = gaussian(&image, sigma) else {
                panic!("the device declined a frame the acceptance rule allows");
            };
            let expected = crate::image_processing::high_precision::apply(
                &image,
                &crate::domain::EditOperation::GaussianBlur { radius: sigma },
                None,
            )
            .expect("cpu reference");
            let worst = expected
                .pixels()
                .iter()
                .zip(actual.pixels())
                .map(|(a, b)| {
                    (a.red - b.red)
                        .abs()
                        .max((a.green - b.green).abs())
                        .max((a.blue - b.blue).abs())
                        .max((a.alpha - b.alpha).abs())
                })
                .fold(0.0f32, f32::max);
            // One step of a 16-bit channel is 1.5e-5. The bound is an order of
            // magnitude below that, so no exported pixel can differ.
            assert!(
                worst < 1.5e-6,
                "sigma {sigma} differed from the CPU reference by {worst}"
            );
        }
    }

    /// A device marked unusable must stop being asked, and the caller must
    /// still get a correct picture from the CPU.
    #[test]
    fn an_unhealthy_device_declines_instead_of_failing() {
        let image = frame(2000, 1100);
        force_unhealthy_for_test(true);
        let result = gaussian(&image, 4.0);
        force_unhealthy_for_test(false);
        assert!(result.is_none(), "an unhealthy device returned pixels");
    }
}
