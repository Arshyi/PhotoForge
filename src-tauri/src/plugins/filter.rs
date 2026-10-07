//! Running a plugin's filter over an image.
//!
//! This is where a declaration becomes a guarantee. A filter says how far each
//! output pixel reaches into the input (`locality`); this file gives the module
//! exactly that much and no more, one tile at a time, so memory follows the tile
//! and not the image, and the result does not depend on where the tile edges fell.
//!
//! * A **pointwise** filter is given each tile alone.
//! * A **local** filter with radius `r` is given each tile grown by `r` on every
//!   side (clipped to the image), and told where its output sits inside it.
//! * A **global** filter is given the whole image in one call, which is only
//!   possible if the whole image and its result fit in the memory a plugin may
//!   use. If they do not, the filter is refused *before it starts*, with the size
//!   it would have needed — not run until it runs out.
//!
//! The declaration is not taken on trust: [`verify_locality`] runs a filter whole
//! and in tiles and compares, which is what the plugin manager's test does, and
//! what finds a filter that says `pointwise` and reads its neighbours.
use super::job::TileJob;
use super::limits::{self, BYTES_PER_PIXEL, TILE_EDGE};
use super::manifest::{FilterDecl, Locality};
use super::{Compiled, PluginError, Runtime};
use crate::color::{FloatImage, FloatRgba};
use crate::source::Rect;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

/// Most log lines kept from one whole filter application.
const MAX_LOG_LINES: usize = 64;

/// The outcome of applying a filter.
pub struct FilterRun {
    pub image: FloatImage,
    pub tiles: usize,
    pub fuel_used: u64,
    pub logs: Vec<String>,
}

/// Calls for a filter over a `width` x `height` image: `(input window, output rect)`.
pub fn plan_tiles(locality: Locality, width: u32, height: u32, tile: u32) -> Vec<(Rect, Rect)> {
    let whole = Rect {
        x: 0,
        y: 0,
        width,
        height,
    };
    let radius = match locality {
        Locality::Global => return vec![(whole, whole)],
        Locality::Pointwise => 0,
        Locality::Local { radius } => radius,
    };
    let tile = tile.max(1);
    let mut calls = Vec::new();
    let mut y = 0;
    while y < height {
        let h = tile.min(height - y);
        let mut x = 0;
        while x < width {
            let w = tile.min(width - x);
            let output = Rect {
                x,
                y,
                width: w,
                height: h,
            };
            let left = x.saturating_sub(radius);
            let top = y.saturating_sub(radius);
            let right = (x + w).saturating_add(radius).min(width);
            let bottom = (y + h).saturating_add(radius).min(height);
            let input = Rect {
                x: left,
                y: top,
                width: right - left,
                height: bottom - top,
            };
            calls.push((input, output));
            x += w;
        }
        y += h;
    }
    calls
}

/// The largest guest memory any call of this plan needs.
pub fn plan_memory(calls: &[(Rect, Rect)], parameters: usize) -> u64 {
    calls
        .iter()
        .map(|(input, output)| {
            limits::call_memory_required(input.pixels(), output.pixels(), parameters)
        })
        .max()
        .unwrap_or(0)
}

fn window_bytes(image: &FloatImage, rect: &Rect) -> Vec<u8> {
    let mut bytes = Vec::with_capacity((rect.pixels() * BYTES_PER_PIXEL) as usize);
    let width = image.width() as usize;
    let pixels = image.pixels();
    for row in rect.y..rect.y + rect.height {
        let start = row as usize * width + rect.x as usize;
        for pixel in &pixels[start..start + rect.width as usize] {
            bytes.extend_from_slice(&pixel.red.to_le_bytes());
            bytes.extend_from_slice(&pixel.green.to_le_bytes());
            bytes.extend_from_slice(&pixel.blue.to_le_bytes());
            bytes.extend_from_slice(&pixel.alpha.to_le_bytes());
        }
    }
    bytes
}

/// Alpha may drift this far outside 0..1 through rounding; further is a defect.
const ALPHA_TOLERANCE: f32 = 1e-4;

fn store_window(image: &mut FloatImage, rect: &Rect, bytes: &[u8]) -> Result<(), PluginError> {
    if bytes.len() as u64 != rect.pixels() * BYTES_PER_PIXEL {
        return Err(PluginError::BadOutput(
            "the output is not the size of its rectangle".into(),
        ));
    }
    let width = image.width() as usize;
    let pixels = image.pixels_mut();
    let mut at = 0usize;
    let read =
        |at: usize| f32::from_le_bytes([bytes[at], bytes[at + 1], bytes[at + 2], bytes[at + 3]]);
    for row in rect.y..rect.y + rect.height {
        let start = row as usize * width + rect.x as usize;
        for slot in &mut pixels[start..start + rect.width as usize] {
            let (red, green, blue, alpha) = (read(at), read(at + 4), read(at + 8), read(at + 12));
            at += 16;
            // Refused, not repaired: a NaN or infinity written into a document
            // would travel through every later edit and export.
            if !(red.is_finite() && green.is_finite() && blue.is_finite() && alpha.is_finite()) {
                return Err(PluginError::BadOutput(
                    "it produced a value that is not a number".into(),
                ));
            }
            if !(-ALPHA_TOLERANCE..=1.0 + ALPHA_TOLERANCE).contains(&alpha) {
                return Err(PluginError::BadOutput(format!(
                    "it produced an alpha of {alpha}, outside 0 to 1"
                )));
            }
            *slot = FloatRgba::new(red, green, blue, alpha.clamp(0.0, 1.0));
        }
    }
    Ok(())
}

/// Bridges a borrowed cancel flag to the owned one the epoch callback needs, for the
/// duration of `work`. One thread per application, not per tile.
fn with_cancel<R>(
    cancel: Option<&AtomicBool>,
    work: impl FnOnce(Option<&Arc<AtomicBool>>) -> R,
) -> R {
    let Some(flag) = cancel else {
        return work(None);
    };
    let shared = Arc::new(AtomicBool::new(flag.load(Ordering::Acquire)));
    let done = AtomicBool::new(false);
    std::thread::scope(|scope| {
        scope.spawn(|| {
            while !done.load(Ordering::Acquire) {
                if flag.load(Ordering::Acquire) {
                    shared.store(true, Ordering::Release);
                }
                std::thread::sleep(limits::EPOCH_TICK);
            }
        });
        let result = work(Some(&shared));
        done.store(true, Ordering::Release);
        result
    })
}

/// Checks `parameters` against the filter's declaration.
pub fn check_parameters(decl: &FilterDecl, parameters: &[f64]) -> Result<(), PluginError> {
    if parameters.len() != decl.parameters.len() {
        return Err(PluginError::Refused(format!(
            "the filter takes {} parameters and was given {}",
            decl.parameters.len(),
            parameters.len()
        )));
    }
    for (declared, value) in decl.parameters.iter().zip(parameters) {
        if !declared.accepts(*value) {
            return Err(PluginError::Refused(format!(
                "{} is not a value the parameter {:?} accepts",
                value, declared.id
            )));
        }
    }
    Ok(())
}

/// Applies filter number `index` to `source`.
///
/// `memory_allowance` is what the plugin may use for one call (see
/// [`limits::memory_allowance`]); `tile` is the tile edge, `TILE_EDGE` in the editor.
#[allow(clippy::too_many_arguments)]
pub fn apply(
    runtime: &Runtime,
    compiled: &Compiled,
    decl: &FilterDecl,
    index: usize,
    parameters: &[f64],
    source: &FloatImage,
    memory_allowance: u64,
    tile: u32,
    cancel: Option<&AtomicBool>,
) -> Result<FilterRun, PluginError> {
    check_parameters(decl, parameters)?;
    let (width, height) = (source.width(), source.height());
    let calls = plan_tiles(decl.locality, width, height, tile);

    // Admission, before any pixel is copied or any instruction run.
    let needed = plan_memory(&calls, parameters.len());
    if needed > memory_allowance {
        let what = match decl.locality {
            Locality::Global => format!(
                "it looks at the whole image at once, which needs about {} MiB for the plugin to \
                 work in, and it may use {} MiB. A smaller image or region would work",
                needed.div_ceil(limits::MIB),
                memory_allowance / limits::MIB
            ),
            _ => format!(
                "even one tile needs about {} MiB and it may use {} MiB",
                needed.div_ceil(limits::MIB),
                memory_allowance / limits::MIB
            ),
        };
        return Err(PluginError::Refused(what));
    }

    let mut result = FloatImage::blank(width, height, FloatRgba::TRANSPARENT)
        .map_err(|error| PluginError::Refused(error.to_string()))?;
    let mut fuel_used = 0u64;
    let mut logs = Vec::new();
    with_cancel(cancel, |shared| {
        for (input_rect, output_rect) in &calls {
            if cancel.is_some_and(|flag| flag.load(Ordering::Acquire)) {
                return Err(PluginError::Cancelled);
            }
            let input = window_bytes(source, input_rect);
            let pixels = input_rect.pixels().max(output_rect.pixels());
            let tile_limits = limits::call_limits(memory_allowance, pixels);
            let output = runtime.call(
                compiled,
                &TileJob {
                    filter_index: index as u32,
                    parameters,
                    image_width: width,
                    image_height: height,
                    input_rect: *input_rect,
                    input: &input,
                    output_rect: *output_rect,
                },
                tile_limits,
                shared,
            )?;
            fuel_used = fuel_used.saturating_add(output.fuel_used);
            for line in output.logs {
                if logs.len() < MAX_LOG_LINES {
                    logs.push(line);
                }
            }
            store_window(&mut result, output_rect, &output.output)?;
        }
        Ok(())
    })?;
    Ok(FilterRun {
        image: result,
        tiles: calls.len(),
        fuel_used,
        logs,
    })
}

/// How the whole-image and tiled results of a filter compare.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LocalityReport {
    pub honest: bool,
    /// Pixels at which the two results differ.
    pub differing_pixels: u64,
    /// The first such pixel, if any.
    pub first_difference: Option<(u32, u32)>,
}

/// Runs a filter over `source` as one call and again in `tile`-sized tiles, and
/// compares them bit for bit.
///
/// A filter that tiles correctly gives identical results either way. One that
/// declares less reach than it uses gives results that differ along tile edges.
/// The comparison is exact: a tolerance would forgive a one-pixel seam, which is
/// exactly the defect being looked for.
pub fn verify_locality(
    runtime: &Runtime,
    compiled: &Compiled,
    decl: &FilterDecl,
    index: usize,
    source: &FloatImage,
    memory_allowance: u64,
    tile: u32,
) -> Result<LocalityReport, PluginError> {
    let parameters = decl.default_parameters();
    let whole_decl = FilterDecl {
        locality: Locality::Global,
        ..decl.clone()
    };
    let whole = apply(
        runtime,
        compiled,
        &whole_decl,
        index,
        &parameters,
        source,
        memory_allowance,
        tile,
        None,
    )?;
    let tiled = apply(
        runtime,
        compiled,
        decl,
        index,
        &parameters,
        source,
        memory_allowance,
        tile,
        None,
    )?;
    let width = source.width();
    let mut differing = 0u64;
    let mut first = None;
    for (position, (a, b)) in whole
        .image
        .pixels()
        .iter()
        .zip(tiled.image.pixels())
        .enumerate()
    {
        let same = a.red.to_bits() == b.red.to_bits()
            && a.green.to_bits() == b.green.to_bits()
            && a.blue.to_bits() == b.blue.to_bits()
            && a.alpha.to_bits() == b.alpha.to_bits();
        if !same {
            differing += 1;
            first.get_or_insert(((position as u32) % width, (position as u32) / width));
        }
    }
    Ok(LocalityReport {
        honest: differing == 0,
        differing_pixels: differing,
        first_difference: first,
    })
}

pub const DEFAULT_TILE: u32 = TILE_EDGE;

#[cfg(test)]
mod tests {
    use super::*;

    fn covered(calls: &[(Rect, Rect)], width: u32, height: u32) -> bool {
        let mut seen = vec![0u8; (width * height) as usize];
        for (_, output) in calls {
            for y in output.y..output.y + output.height {
                for x in output.x..output.x + output.width {
                    seen[(y * width + x) as usize] += 1;
                }
            }
        }
        seen.iter().all(|count| *count == 1)
    }

    #[test]
    fn output_rectangles_tile_the_image_exactly_once() {
        for locality in [
            Locality::Pointwise,
            Locality::Local { radius: 3 },
            Locality::Global,
        ] {
            for (w, h, tile) in [
                (1, 1, 256),
                (255, 257, 256),
                (512, 512, 256),
                (100, 7, 16),
                (17, 300, 64),
            ] {
                let calls = plan_tiles(locality, w, h, tile);
                assert!(covered(&calls, w, h), "{locality:?} {w}x{h} tile {tile}");
            }
        }
    }

    #[test]
    fn a_local_filter_is_given_exactly_its_radius_and_never_outside_the_image() {
        let calls = plan_tiles(Locality::Local { radius: 5 }, 100, 80, 32);
        for (input, output) in &calls {
            assert!(input.right() <= 100 && input.bottom() <= 80);
            // The window contains the output grown by 5, clipped, and nothing more.
            assert_eq!(input.x, output.x.saturating_sub(5));
            assert_eq!(input.y, output.y.saturating_sub(5));
            assert_eq!(input.right(), (output.right() + 5).min(100));
            assert_eq!(input.bottom(), (output.bottom() + 5).min(80));
        }
        // A pointwise filter gets no more than its own tile.
        for (input, output) in plan_tiles(Locality::Pointwise, 100, 80, 32) {
            assert_eq!(input, output);
        }
    }

    #[test]
    fn a_global_filter_is_one_call_over_the_whole_image() {
        let calls = plan_tiles(Locality::Global, 300, 200, 16);
        assert_eq!(calls.len(), 1);
        assert_eq!(calls[0].0, calls[0].1);
        assert_eq!((calls[0].0.width, calls[0].0.height), (300, 200));
    }

    #[test]
    fn memory_follows_the_tile_for_local_filters_and_the_image_for_global_ones() {
        let local = plan_memory(&plan_tiles(Locality::Pointwise, 8000, 6000, 256), 2);
        let global = plan_memory(&plan_tiles(Locality::Global, 8000, 6000, 256), 2);
        assert_eq!(local, 2 * 256 * 256 * 16 + 16 + limits::MEMORY_OVERHEAD);
        assert_eq!(global, 2 * 8000 * 6000 * 16 + 16 + limits::MEMORY_OVERHEAD);
        assert!(global > 50 * local);
    }
}
