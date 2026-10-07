//! The planner's prices against what the decoders really hold.
//!
//! A price that is too low is a refusal that should have happened turning into an
//! out-of-memory, so this is the test that keeps the admission planner honest: for
//! each decoder it measures the peak heap the decode allocates, with a counting
//! allocator, and requires it to be **no more** than what the planner priced.
//!
//! It exists because the first version of the planner priced a WebP without alpha at
//! three bytes a pixel, which is the size of its result; the decoder actually holds
//! seven. Nothing failed until a measurement said so.
//!
//! The allocator counts every thread, so everything runs in one test, one scenario at
//! a time, and the fixtures are written before the counting starts. It sees the Rust
//! heap, which is where the decoders allocate; it does not see memory the operating
//! system maps for the process on its own, which the benchmark examples report
//! through the working set.
use photoforge_lib::resources::admission::{
    reduced_transient_bytes, region_cost, OPEN_PREVIEW_BYTES,
};
use photoforge_lib::source::open::{open_selection, OpenSelection};
use photoforge_lib::source::probe::probe_path;
use photoforge_lib::source::{jpeg, png, webp, Rect};
use std::alloc::{GlobalAlloc, Layout, System};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};

struct Counting;
static CURRENT: AtomicUsize = AtomicUsize::new(0);
static PEAK: AtomicUsize = AtomicUsize::new(0);

// SAFETY: forwards every call to the system allocator unchanged and only counts sizes.
unsafe impl GlobalAlloc for Counting {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        let pointer = System.alloc(layout);
        if !pointer.is_null() {
            let now = CURRENT.fetch_add(layout.size(), Ordering::SeqCst) + layout.size();
            PEAK.fetch_max(now, Ordering::SeqCst);
        }
        pointer
    }
    unsafe fn alloc_zeroed(&self, layout: Layout) -> *mut u8 {
        let pointer = System.alloc_zeroed(layout);
        if !pointer.is_null() {
            let now = CURRENT.fetch_add(layout.size(), Ordering::SeqCst) + layout.size();
            PEAK.fetch_max(now, Ordering::SeqCst);
        }
        pointer
    }
    unsafe fn dealloc(&self, pointer: *mut u8, layout: Layout) {
        System.dealloc(pointer, layout);
        CURRENT.fetch_sub(layout.size(), Ordering::SeqCst);
    }
}

#[global_allocator]
static ALLOCATOR: Counting = Counting;

/// Runs `work` and returns its result with the most heap it held above where it began.
fn peak_of<T>(work: impl FnOnce() -> T) -> (T, u64) {
    let base = CURRENT.load(Ordering::SeqCst);
    PEAK.store(base, Ordering::SeqCst);
    let result = work();
    let peak = PEAK.load(Ordering::SeqCst);
    (result, peak.saturating_sub(base) as u64)
}

const WIDTH: u32 = 2400;
const HEIGHT: u32 = 1800;

fn picture(alpha: bool) -> image::RgbaImage {
    let mut image = image::RgbaImage::new(WIDTH, HEIGHT);
    let mut state = 0x9e37_79b9u32;
    for (x, y, pixel) in image.enumerate_pixels_mut() {
        state = state.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
        let noise = (state >> 28) as u8;
        *pixel = image::Rgba([
            ((x / 7 + y / 5) % 180) as u8 + noise,
            ((x * 255 / WIDTH) as u8).saturating_add(noise),
            ((y * 255 / HEIGHT) as u8).wrapping_add(noise),
            if alpha {
                128 + ((x ^ y) % 127) as u8
            } else {
                255
            },
        ]);
    }
    image
}

struct Fixture {
    label: &'static str,
    path: PathBuf,
}

fn fixtures(directory: &Path) -> Vec<Fixture> {
    let rgba = picture(true);
    let rgb = image::DynamicImage::ImageRgba8(picture(false)).to_rgb8();
    let mut out = Vec::new();
    let mut add = |label: &'static str, name: &str, write: &dyn Fn(&Path)| {
        let path = directory.join(name);
        write(&path);
        out.push(Fixture { label, path });
    };
    add("png rgb8", "a.png", &|path| rgb.save(path).unwrap());
    add("png rgba8", "b.png", &|path| rgba.save(path).unwrap());
    add("png rgb16", "c.png", &|path| {
        image::DynamicImage::ImageRgb8(rgb.clone())
            .to_rgb16()
            .save(path)
            .unwrap()
    });
    add("jpeg", "d.jpg", &|path| {
        let file = std::fs::File::create(path).unwrap();
        image::codecs::jpeg::JpegEncoder::new_with_quality(std::io::BufWriter::new(file), 90)
            .encode_image(&rgb)
            .unwrap();
    });
    add("webp rgb", "e.webp", &|path| {
        let file = std::fs::File::create(path).unwrap();
        image::codecs::webp::WebPEncoder::new_lossless(std::io::BufWriter::new(file))
            .encode(rgb.as_raw(), WIDTH, HEIGHT, image::ExtendedColorType::Rgb8)
            .unwrap();
    });
    add("webp rgba", "f.webp", &|path| {
        let file = std::fs::File::create(path).unwrap();
        image::codecs::webp::WebPEncoder::new_lossless(std::io::BufWriter::new(file))
            .encode(
                rgba.as_raw(),
                WIDTH,
                HEIGHT,
                image::ExtendedColorType::Rgba8,
            )
            .unwrap();
    });
    out
}

/// A float image costs 16 bytes a pixel.
const FLOAT_PIXEL: u64 = 16;

struct Check {
    label: String,
    measured: u64,
    priced: u64,
}

impl Check {
    fn line(&self) -> String {
        format!(
            "{:<34} measured {:>10} B   priced {:>10} B   ({:>3.0}% of the price)",
            self.label,
            self.measured,
            self.priced,
            100.0 * self.measured as f64 / self.priced as f64
        )
    }
}

#[test]
fn no_decoder_holds_more_than_the_planner_priced() {
    let directory = tempfile::tempdir().unwrap();
    let fixtures = fixtures(directory.path());
    let rect = Rect {
        x: 300,
        y: 400,
        width: 1200,
        height: 900,
    };
    let region_pixels = u64::from(rect.width) * u64::from(rect.height);
    // Half of each side: a result large enough that a second copy of it could not hide in
    // the allowance for the decoder's own buffers.
    let (reduced_w, reduced_h) = (WIDTH / 2, HEIGHT / 2);
    let reduced_pixels = u64::from(reduced_w) * u64::from(reduced_h);
    let mut checks = Vec::new();

    for fixture in &fixtures {
        let probed = probe_path(&fixture.path).unwrap();
        let native = probed.probe.native_bytes_per_pixel;
        let name = fixture.label;

        // ---- a region ---------------------------------------------------------------
        let cost = region_cost(&probed.probe).expect("these formats can open a region");
        // The decode phase: the file and the decoder's fixed part, and what the decoder
        // holds per pixel of the window, and the window's own copy at its native depth.
        let decode_price = cost.opening_fixed - OPEN_PREVIEW_BYTES as u64
            + (cost.opening_per_pixel - FLOAT_PIXEL + native) * region_pixels;
        let (image, measured) = peak_of(|| match name {
            n if n.starts_with("png") => png::decode_region(&fixture.path, rect, None)
                .map(|r| r.image)
                .unwrap(),
            "jpeg" => jpeg::decode_region(&fixture.path, rect, None)
                .map(|r| r.image)
                .unwrap(),
            _ => webp::decode_region(&fixture.path, rect, None)
                .map(|r| r.image)
                .unwrap(),
        });
        assert_eq!((image.width(), image.height()), (rect.width, rect.height));
        drop(image);
        checks.push(Check {
            label: format!("{name} region, decode"),
            measured,
            priced: decode_price,
        });

        // The whole open, as the application does it: decode, the working copy, the
        // preview and the file's hash, against the planner's price for opening.
        let opening_price = cost.opening_fixed + cost.opening_per_pixel * region_pixels;
        let (loaded, measured) = peak_of(|| {
            open_selection(&fixture.path, OpenSelection::Region(rect), 0, None).unwrap()
        });
        drop(loaded);
        checks.push(Check {
            label: format!("{name} region, whole open"),
            measured,
            priced: opening_price,
        });

        // ---- a reduced copy, at half of each side ---------------------------------
        let scale = f64::from(reduced_w) / f64::from(WIDTH);
        let transient = reduced_transient_bytes(&probed.probe, scale)
            .expect("these formats can be reduced") as u64;
        let decode_price = transient + reduced_pixels * FLOAT_PIXEL;
        let (reduced, measured) = peak_of(|| match name {
            n if n.starts_with("png") => {
                png::decode_reduced(&fixture.path, reduced_w, reduced_h, None).unwrap()
            }
            "jpeg" => jpeg::decode_reduced(&fixture.path, reduced_w, reduced_h, None).unwrap(),
            _ => webp::decode_reduced(&fixture.path, reduced_w, reduced_h, None).unwrap(),
        });
        assert_eq!((reduced.width(), reduced.height()), (reduced_w, reduced_h));
        drop(reduced);
        checks.push(Check {
            label: format!("{name} reduced, decode"),
            measured,
            priced: decode_price,
        });

        let opening_price = probed.file_bytes
            + transient
            + OPEN_PREVIEW_BYTES as u64
            + reduced_pixels * FLOAT_PIXEL;
        let (loaded, measured) = peak_of(|| {
            open_selection(
                &fixture.path,
                OpenSelection::Reduced {
                    width: reduced_w,
                    height: reduced_h,
                },
                0,
                None,
            )
            .unwrap()
        });
        drop(loaded);
        checks.push(Check {
            label: format!("{name} reduced, whole open"),
            measured,
            priced: opening_price,
        });
    }

    let report: Vec<String> = checks.iter().map(Check::line).collect();
    println!(
        "{}",
        report.join(
            "
"
        )
    );
    let under: Vec<String> = checks
        .iter()
        .filter(|check| check.measured > check.priced)
        .map(Check::line)
        .collect();
    assert!(
        under.is_empty(),
        "the planner under-priced:
{}

all checks:
{}",
        under.join(
            "
"
        ),
        report.join(
            "
"
        )
    );
    // A counter that read nothing would pass everything above, and a price wildly above
    // the measurement would refuse files that fit.
    for check in &checks {
        assert!(check.measured > 0, "{}: nothing was counted", check.label);
        assert!(
            check.measured * 40 > check.priced,
            "{}: priced {} against a measurement of {}",
            check.label,
            check.priced,
            check.measured
        );
    }
}
