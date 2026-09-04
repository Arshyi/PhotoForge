use super::blend::{clamp_unit, composite_pixel, BlendMode};
use super::model::{Layer, LayerContent, LayerDocument, MAX_GROUP_DEPTH};
use super::transform::{LayerInterpolation, LayerTransform};
use crate::domain::EditOperation;
use crate::error::AppError;
use crate::image_processing::apply_operation;
use crate::mask::MaskBitmap;
use image::{Rgba, RgbaImage};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

/// Resolves the immutable pixel buffer behind a pixel layer.
///
/// The layer tree never carries pixels; it carries buffer identifiers that the
/// session pixel store resolves. A preview render resolves smaller buffers than
/// a full-resolution render, which is why the compositor reads dimensions from
/// the resolved buffer rather than from the document.
pub trait PixelSource {
    fn resolve(&self, pixel_id: &str) -> Result<Arc<RgbaImage>, AppError>;
}

#[derive(Debug, Clone, Copy)]
pub struct RenderOptions<'a> {
    /// Ratio between the buffers this render will resolve and the document's
    /// declared full-resolution canvas. Layer translations scale with it so a
    /// preview is a faithful miniature of the export.
    pub scale: f64,
    pub cancel: Option<&'a AtomicBool>,
}

impl Default for RenderOptions<'_> {
    fn default() -> Self {
        Self {
            scale: 1.0,
            cancel: None,
        }
    }
}

struct RenderContext<'a> {
    source: &'a dyn PixelSource,
    scale: f64,
    cancel: Option<&'a AtomicBool>,
    canvas_width: u32,
    canvas_height: u32,
}

impl RenderContext<'_> {
    fn check_cancelled(&self) -> Result<(), AppError> {
        match self.cancel {
            Some(flag) if flag.load(Ordering::Acquire) => Err(AppError::RenderCancelled),
            _ => Ok(()),
        }
    }

    fn blank_canvas(&self) -> RgbaImage {
        RgbaImage::from_pixel(self.canvas_width, self.canvas_height, Rgba([0, 0, 0, 0]))
    }
}

/// Renders a document's visible composite, clipped to the document canvas.
///
/// The traversal is deterministic: layers composite strictly bottom to top in
/// a fixed order. Work within one layer may use disjoint parallel row bands;
/// because rows never overlap and there is no reduction, two renders of the
/// same document and buffers still produce byte-identical output.
pub fn render_document(
    document: &LayerDocument,
    source: &dyn PixelSource,
    options: RenderOptions<'_>,
) -> Result<RgbaImage, AppError> {
    document.validate()?;
    render_layers(
        &document.layers,
        document.canvas_width,
        document.canvas_height,
        source,
        options,
    )
}

/// Renders an arbitrary layer stack against a canvas of the given full
/// resolution. Merge and flatten reuse this with a subset of the tree.
pub fn render_layers(
    layers: &[Layer],
    canvas_width: u32,
    canvas_height: u32,
    source: &dyn PixelSource,
    options: RenderOptions<'_>,
) -> Result<RgbaImage, AppError> {
    if !options.scale.is_finite() || options.scale <= 0.0 || options.scale > 1.0 {
        return Err(AppError::InvalidLayerDocument(
            "render scale must be greater than zero and no larger than one".into(),
        ));
    }
    let render_width = scaled_dimension(canvas_width, options.scale);
    let render_height = scaled_dimension(canvas_height, options.scale);
    let context = RenderContext {
        source,
        scale: options.scale,
        cancel: options.cancel,
        canvas_width: render_width,
        canvas_height: render_height,
    };
    composite_stack(layers, &context, 1)
}

fn scaled_dimension(value: u32, scale: f64) -> u32 {
    let scaled = (f64::from(value) * scale).round();
    scaled.clamp(1.0, f64::from(u32::MAX)) as u32
}

/// Composites one stack of siblings onto a transparent backdrop.
///
/// Isolated groups composite children against a transparent buffer of their
/// own before blending the finished result into the parent. Pass-through
/// groups instead let their children operate on the existing backdrop, then
/// apply the group's mask and opacity to that reworked result.
///
/// Recursion is bounded because `LayerDocument::validate` rejects any tree
/// deeper than `MAX_GROUP_DEPTH` before a render begins, and this function
/// re-checks the depth so a caller that skipped validation still fails closed.
fn composite_stack(
    layers: &[Layer],
    context: &RenderContext<'_>,
    depth: usize,
) -> Result<RgbaImage, AppError> {
    context.check_cancelled()?;
    if depth > MAX_GROUP_DEPTH {
        return Err(AppError::LayerDepthExceeded {
            depth,
            limit: MAX_GROUP_DEPTH,
        });
    }
    let mut backdrop = context.blank_canvas();
    composite_onto(&mut backdrop, layers, context, depth)?;
    Ok(backdrop)
}

/// Composites a stack of siblings onto an existing backdrop.
///
/// Splitting this out from `composite_stack` is what makes pass-through groups
/// possible: an isolated group gets a fresh transparent canvas, while a
/// pass-through group is handed a copy of whatever is already beneath it.
fn composite_onto(
    backdrop: &mut RgbaImage,
    layers: &[Layer],
    context: &RenderContext<'_>,
    depth: usize,
) -> Result<(), AppError> {
    if depth > MAX_GROUP_DEPTH {
        return Err(AppError::LayerDepthExceeded {
            depth,
            limit: MAX_GROUP_DEPTH,
        });
    }
    for layer in layers {
        context.check_cancelled()?;
        // A hidden layer and a fully transparent layer contribute nothing in
        // every blend mode, so both are skipped rather than composited.
        if !layer.visible || layer.opacity <= 0.0 {
            continue;
        }
        match &layer.content {
            LayerContent::Pixel { pixel_id, .. } => {
                let buffer = context.source.resolve(pixel_id)?;
                draw_source(backdrop, buffer.as_ref(), layer, context)?;
            }
            LayerContent::Group { children, isolated } => {
                if children.is_empty() {
                    continue;
                }
                if *isolated {
                    let rendered = composite_stack(children, context, depth + 1)?;
                    draw_source(backdrop, &rendered, layer, context)?;
                } else {
                    // Pass-through: the children see the accumulated backdrop,
                    // so an adjustment inside the group also reaches the layers
                    // below it. The group's own opacity and mask then decide how
                    // much of that reworked backdrop replaces the original.
                    let mut reworked = backdrop.clone();
                    composite_onto(&mut reworked, children, context, depth + 1)?;
                    cross_fade(backdrop, &reworked, layer, context)?;
                }
            }
            LayerContent::Adjustment { operation } => {
                apply_adjustment(backdrop, operation, layer, context)?;
            }
        }
    }
    context.check_cancelled()
}

/// Interpolates a pass-through group's reworked backdrop back over the original,
/// weighted by the group's opacity and mask.
///
/// Both sides are complete composites, so the blend happens in premultiplied
/// space and is converted back to straight alpha. Interpolating straight colours
/// directly would darken or lighten wherever the two differ in coverage.
fn cross_fade(
    backdrop: &mut RgbaImage,
    reworked: &RgbaImage,
    layer: &Layer,
    context: &RenderContext<'_>,
) -> Result<(), AppError> {
    context.check_cancelled()?;
    let opacity = clamp_unit(layer.opacity);
    let mask = decoded_mask(layer)?;
    let inverted = mask_inverted(layer);
    let interpolation = layer.transform.interpolation;
    let transform = render_transform(&layer.transform, context.scale);
    let inverse = transform.inverse(context.canvas_width, context.canvas_height)?;

    // A fully opaque, unmasked pass-through group is exactly its reworked
    // backdrop, which is the common case and needs no per-pixel mixing.
    if opacity >= 1.0 && mask.is_none() {
        let row_bytes = context.canvas_width as usize * 4;
        return for_each_row_band(
            backdrop.as_mut(),
            context.canvas_width,
            0,
            context.canvas_height,
            context.cancel,
            |y, row| {
                let start = y as usize * row_bytes;
                row.copy_from_slice(&reworked.as_raw()[start..start + row_bytes]);
            },
        );
    }

    let canvas_width = context.canvas_width;
    let canvas_height = context.canvas_height;
    let mask_reference = mask.as_ref();
    let reworked_raw = reworked.as_raw();

    for_each_row_band(
        backdrop.as_mut(),
        canvas_width,
        0,
        canvas_height,
        context.cancel,
        |y, row| {
            let source_row = y as usize * canvas_width as usize * 4;
            for x in 0..canvas_width {
                let (local_x, local_y) = inverse.apply(x as f32 + 0.5, y as f32 + 0.5);
                let coverage = opacity
                    * mask_coverage(
                        mask_reference,
                        inverted,
                        local_x,
                        local_y,
                        canvas_width,
                        canvas_height,
                        interpolation,
                    );
                if coverage <= 0.0 {
                    continue;
                }
                let index = x as usize * 4;
                let source_index = source_row + index;
                let base_alpha = f32::from(row[index + 3]) / 255.0;
                let next_alpha = f32::from(reworked_raw[source_index + 3]) / 255.0;
                let out_alpha = base_alpha + coverage * (next_alpha - base_alpha);
                if out_alpha <= 0.0 {
                    for channel in 0..4 {
                        row[index + channel] = 0;
                    }
                    continue;
                }
                for channel in 0..3 {
                    let base = f32::from(row[index + channel]) / 255.0 * base_alpha;
                    let next = f32::from(reworked_raw[source_index + channel]) / 255.0 * next_alpha;
                    let mixed = base + coverage * (next - base);
                    row[index + channel] = to_byte(mixed / out_alpha);
                }
                row[index + 3] = to_byte(out_alpha);
            }
        },
    )?;
    Ok(())
}

/// Scales a layer transform into render space. Only the translation depends on
/// the render scale; scale, rotation, and flips are relative to the layer's own
/// dimensions and therefore already scale-free.
fn render_transform(transform: &LayerTransform, scale: f64) -> LayerTransform {
    LayerTransform {
        translate_x: (f64::from(transform.translate_x) * scale) as f32,
        translate_y: (f64::from(transform.translate_y) * scale) as f32,
        ..*transform
    }
}

fn decoded_mask(layer: &Layer) -> Result<Option<MaskBitmap>, AppError> {
    match &layer.mask {
        Some(mask) if mask.enabled => Ok(Some(mask.snapshot.decode()?)),
        _ => Ok(None),
    }
}

fn mask_inverted(layer: &Layer) -> bool {
    layer.mask.as_ref().is_some_and(|mask| mask.inverted)
}

/// Draws a source buffer through a layer's transform, mask, opacity, and blend
/// mode, clipped to the canvas.
fn draw_source(
    backdrop: &mut RgbaImage,
    source: &RgbaImage,
    layer: &Layer,
    context: &RenderContext<'_>,
) -> Result<(), AppError> {
    let (source_width, source_height) = source.dimensions();
    if source_width == 0 || source_height == 0 {
        return Ok(());
    }

    let transform = render_transform(&layer.transform, context.scale);
    let bounds = transform.document_bounds(source_width, source_height);
    let Some((x0, y0, x1, y1)) = bounds.clip_to_canvas(context.canvas_width, context.canvas_height)
    else {
        // The layer lies wholly outside the canvas.
        return Ok(());
    };
    let inverse = transform.inverse(source_width, source_height)?;
    let mask = decoded_mask(layer)?;
    let inverted = mask_inverted(layer);
    let opacity = clamp_unit(layer.opacity);
    let blend = layer.blend_mode;
    let interpolation = layer.transform.interpolation;

    // Fast path for the most common placement: a whole-pixel position with no
    // mask, full opacity, and normal blending. Whole-pixel sampling is already
    // an exact copy and the general formula reduces to plain source-over, so
    // this produces byte-identical output without any resampling arithmetic.
    if blend == BlendMode::Normal && mask.is_none() && layer.opacity >= 1.0 {
        if let Some((offset_x, offset_y)) = transform.integer_translation() {
            return draw_source_over(
                backdrop,
                source,
                offset_x,
                offset_y,
                (x0, y0, x1, y1),
                context.cancel,
            );
        }
    }

    context.check_cancelled()?;
    let backdrop_width = backdrop.width();
    let mask_reference = mask.as_ref();

    // One destination row is written by exactly one worker and never read by
    // another, so splitting the region into row bands keeps the result
    // byte-identical to a single-threaded run while using the available cores.
    for_each_row_band(
        backdrop.as_mut(),
        backdrop_width,
        y0,
        y1,
        context.cancel,
        |y, row| {
            for x in x0..x1 {
                let (local_x, local_y) = inverse.apply(x as f32 + 0.5, y as f32 + 0.5);
                let Some(sample) = sample_rgba(source, local_x, local_y, interpolation) else {
                    continue;
                };
                let coverage = opacity
                    * mask_coverage(
                        mask_reference,
                        inverted,
                        local_x,
                        local_y,
                        source_width,
                        source_height,
                        interpolation,
                    );
                if coverage <= 0.0 || sample[3] <= 0.0 {
                    continue;
                }
                let source_pixel = [sample[0], sample[1], sample[2], sample[3] * coverage];
                let index = x as usize * 4;
                let existing = [
                    f32::from(row[index]) / 255.0,
                    f32::from(row[index + 1]) / 255.0,
                    f32::from(row[index + 2]) / 255.0,
                    f32::from(row[index + 3]) / 255.0,
                ];
                let result = composite_pixel(existing, source_pixel, blend);
                for channel in 0..4 {
                    row[index + channel] = to_byte(result[channel]);
                }
            }
        },
    )?;
    context.check_cancelled()?;
    Ok(())
}

/// Number of worker threads a render may use. Bounded so a large document
/// cannot spawn an unreasonable number of threads on a many-core machine.
const MAX_RENDER_THREADS: usize = 8;
/// Below this many rows the coordination cost outweighs the parallelism.
const MIN_ROWS_PER_THREAD: u32 = 24;

/// Runs `apply` over each destination row in `y0..y1`, in parallel row bands
/// where that is worthwhile.
///
/// Bands are disjoint slices of the same buffer, so no row is written twice and
/// no reduction happens. That is what keeps the output deterministic regardless
/// of how the work is divided or how the threads are scheduled.
fn for_each_row_band<F>(
    pixels: &mut [u8],
    width: u32,
    y0: u32,
    y1: u32,
    cancel: Option<&AtomicBool>,
    apply: F,
) -> Result<(), AppError>
where
    F: Fn(u32, &mut [u8]) + Sync,
{
    let available = std::thread::available_parallelism()
        .map(std::num::NonZeroUsize::get)
        .unwrap_or(1)
        .min(MAX_RENDER_THREADS);
    for_each_row_band_with_threads(pixels, width, y0, y1, cancel, available, apply)
}

// Keeping the worker limit explicit here lets tests exercise both schedules,
// including on a one-core CI runner, without relying on timing or sleeps.
fn for_each_row_band_with_threads<F>(
    pixels: &mut [u8],
    width: u32,
    y0: u32,
    y1: u32,
    cancel: Option<&AtomicBool>,
    max_threads: usize,
    apply: F,
) -> Result<(), AppError>
where
    F: Fn(u32, &mut [u8]) + Sync,
{
    if cancel.is_some_and(|flag| flag.load(Ordering::Acquire)) {
        return Err(AppError::RenderCancelled);
    }
    if y1 <= y0 || width == 0 {
        return Ok(());
    }
    let row_bytes = width as usize * 4;
    let rows = y1 - y0;
    let threads = max_threads
        .clamp(1, MAX_RENDER_THREADS)
        .min((rows / MIN_ROWS_PER_THREAD).max(1) as usize);

    let region = &mut pixels[y0 as usize * row_bytes..y1 as usize * row_bytes];
    if threads <= 1 {
        for (offset, row) in region.chunks_mut(row_bytes).enumerate() {
            if cancel.is_some_and(|flag| flag.load(Ordering::Acquire)) {
                return Err(AppError::RenderCancelled);
            }
            apply(y0 + offset as u32, row);
        }
    } else {
        let band_rows = rows.div_ceil(threads as u32) as usize;
        std::thread::scope(|scope| {
            for (band, chunk) in region.chunks_mut(band_rows * row_bytes).enumerate() {
                let first_row = y0 + (band * band_rows) as u32;
                let apply = &apply;
                scope.spawn(move || {
                    for (offset, row) in chunk.chunks_mut(row_bytes).enumerate() {
                        if cancel.is_some_and(|flag| flag.load(Ordering::Acquire)) {
                            break;
                        }
                        apply(first_row + offset as u32, row);
                    }
                });
            }
        });
    }
    if cancel.is_some_and(|flag| flag.load(Ordering::Acquire)) {
        Err(AppError::RenderCancelled)
    } else {
        Ok(())
    }
}

/// Plain source-over of a whole-pixel-aligned buffer.
///
/// Equivalent to the general loop for this case, but it indexes the raw buffers
/// directly and skips fully transparent source pixels and the blend arithmetic
/// for fully opaque ones.
fn draw_source_over(
    backdrop: &mut RgbaImage,
    source: &RgbaImage,
    offset_x: i64,
    offset_y: i64,
    region: (u32, u32, u32, u32),
    cancel: Option<&AtomicBool>,
) -> Result<(), AppError> {
    let (x0, y0, x1, y1) = region;
    let (source_width, source_height) = source.dimensions();
    let backdrop_width = backdrop.width();
    let source_raw = source.as_raw();
    for_each_row_band(
        backdrop.as_mut(),
        backdrop_width,
        y0,
        y1,
        cancel,
        |y, row| {
            let source_y = i64::from(y) - offset_y;
            if source_y < 0 || source_y >= i64::from(source_height) {
                return;
            }
            let source_row = source_y as usize * source_width as usize;
            for x in x0..x1 {
                let source_x = i64::from(x) - offset_x;
                if source_x < 0 || source_x >= i64::from(source_width) {
                    continue;
                }
                let source_index = (source_row + source_x as usize) * 4;
                let alpha = source_raw[source_index + 3];
                if alpha == 0 {
                    continue;
                }
                let backdrop_index = x as usize * 4;
                if alpha == u8::MAX {
                    row[backdrop_index..backdrop_index + 4]
                        .copy_from_slice(&source_raw[source_index..source_index + 4]);
                    continue;
                }
                let source_pixel = [
                    f32::from(source_raw[source_index]) / 255.0,
                    f32::from(source_raw[source_index + 1]) / 255.0,
                    f32::from(source_raw[source_index + 2]) / 255.0,
                    f32::from(alpha) / 255.0,
                ];
                let backdrop_pixel = [
                    f32::from(row[backdrop_index]) / 255.0,
                    f32::from(row[backdrop_index + 1]) / 255.0,
                    f32::from(row[backdrop_index + 2]) / 255.0,
                    f32::from(row[backdrop_index + 3]) / 255.0,
                ];
                let result = composite_pixel(backdrop_pixel, source_pixel, BlendMode::Normal);
                for channel in 0..4 {
                    row[backdrop_index + channel] = to_byte(result[channel]);
                }
            }
        },
    )
}

/// Re-evaluates an adjustment layer against the backdrop beneath it.
///
/// An adjustment layer changes colour without contributing coverage, so the
/// backdrop's alpha is preserved exactly and only the colour channels move,
/// weighted by the layer's opacity and mask. This is why stacking adjustment
/// layers never lightens or darkens a transparent edge.
fn apply_adjustment(
    backdrop: &mut RgbaImage,
    operation: &EditOperation,
    layer: &Layer,
    context: &RenderContext<'_>,
) -> Result<(), AppError> {
    if !operation.supports_adjustment_layer() {
        return Err(AppError::UnsupportedAdjustmentLayer(
            operation.kind().to_string(),
        ));
    }
    context.check_cancelled()?;
    let adjusted = apply_operation(backdrop, operation)?;
    if adjusted.dimensions() != backdrop.dimensions() {
        return Err(AppError::InvalidLayerDocument(
            "an adjustment layer must preserve the canvas dimensions".into(),
        ));
    }

    let transform = render_transform(&layer.transform, context.scale);
    let inverse = transform.inverse(context.canvas_width, context.canvas_height)?;
    let mask = decoded_mask(layer)?;
    let inverted = mask_inverted(layer);
    let opacity = clamp_unit(layer.opacity);
    let blend = layer.blend_mode;
    let interpolation = layer.transform.interpolation;

    context.check_cancelled()?;
    let canvas_width = context.canvas_width;
    let canvas_height = context.canvas_height;
    let mask_reference = mask.as_ref();
    let adjusted_raw = adjusted.as_raw();

    for_each_row_band(
        backdrop.as_mut(),
        canvas_width,
        0,
        canvas_height,
        context.cancel,
        |y, row| {
            let adjusted_row = y as usize * canvas_width as usize * 4;
            for x in 0..canvas_width {
                let (local_x, local_y) = inverse.apply(x as f32 + 0.5, y as f32 + 0.5);
                let coverage = opacity
                    * mask_coverage(
                        mask_reference,
                        inverted,
                        local_x,
                        local_y,
                        canvas_width,
                        canvas_height,
                        interpolation,
                    );
                if coverage <= 0.0 {
                    continue;
                }
                let index = x as usize * 4;
                let source_index = adjusted_row + index;
                let base = [
                    f32::from(row[index]) / 255.0,
                    f32::from(row[index + 1]) / 255.0,
                    f32::from(row[index + 2]) / 255.0,
                ];
                let changed = [
                    f32::from(adjusted_raw[source_index]) / 255.0,
                    f32::from(adjusted_raw[source_index + 1]) / 255.0,
                    f32::from(adjusted_raw[source_index + 2]) / 255.0,
                ];
                let blended = blend.blend(base, changed);
                for channel in 0..3 {
                    row[index + channel] =
                        to_byte(base[channel] + coverage * (blended[channel] - base[channel]));
                }
                // Alpha is deliberately untouched: an adjustment layer changes
                // colour without contributing coverage.
            }
        },
    )?;
    context.check_cancelled()?;
    Ok(())
}

fn unpack(pixel: &Rgba<u8>) -> [f32; 4] {
    [
        f32::from(pixel[0]) / 255.0,
        f32::from(pixel[1]) / 255.0,
        f32::from(pixel[2]) / 255.0,
        f32::from(pixel[3]) / 255.0,
    ]
}

#[cfg(test)]
fn pack(color: [f32; 4]) -> Rgba<u8> {
    Rgba([
        to_byte(color[0]),
        to_byte(color[1]),
        to_byte(color[2]),
        to_byte(color[3]),
    ])
}

fn to_byte(value: f32) -> u8 {
    (clamp_unit(value) * 255.0).round() as u8
}

/// Samples a buffer at a continuous layer-space coordinate.
///
/// Pixel centres sit at `index + 0.5`. Whole-pixel coordinates take an exact
/// copy path so an identity or integer translation is lossless. Bilinear mode
/// interpolates in premultiplied space to prevent dark or light halos around
/// transparent edges; nearest mode selects one pixel. Coordinates outside the
/// buffer return `None`, and neighbours outside the buffer count as transparent
/// so edges fade correctly instead of smearing.
fn sample_rgba(
    source: &RgbaImage,
    x: f32,
    y: f32,
    interpolation: LayerInterpolation,
) -> Option<[f32; 4]> {
    let (width, height) = source.dimensions();
    if !x.is_finite() || !y.is_finite() {
        return None;
    }
    if x < 0.0 || y < 0.0 || x >= width as f32 || y >= height as f32 {
        return None;
    }

    if interpolation == LayerInterpolation::Nearest {
        // The bounds check above already places x and y inside the buffer, so
        // truncating lands on a real pixel without further clamping.
        return Some(unpack(source.get_pixel(x as u32, y as u32)));
    }

    let sample_x = x - 0.5;
    let sample_y = y - 0.5;
    let base_x = sample_x.floor();
    let base_y = sample_y.floor();
    let fraction_x = sample_x - base_x;
    let fraction_y = sample_y - base_y;

    if fraction_x == 0.0 && fraction_y == 0.0 {
        let ix = base_x as i64;
        let iy = base_y as i64;
        if ix >= 0 && iy >= 0 && ix < i64::from(width) && iy < i64::from(height) {
            return Some(unpack(source.get_pixel(ix as u32, iy as u32)));
        }
    }

    let mut accumulated = [0.0_f32; 4];
    for (offset_y, weight_y) in [(0_i64, 1.0 - fraction_y), (1, fraction_y)] {
        if weight_y <= 0.0 {
            continue;
        }
        for (offset_x, weight_x) in [(0_i64, 1.0 - fraction_x), (1, fraction_x)] {
            if weight_x <= 0.0 {
                continue;
            }
            let ix = base_x as i64 + offset_x;
            let iy = base_y as i64 + offset_y;
            if ix < 0 || iy < 0 || ix >= i64::from(width) || iy >= i64::from(height) {
                continue;
            }
            let weight = weight_x * weight_y;
            let pixel = unpack(source.get_pixel(ix as u32, iy as u32));
            let alpha = pixel[3];
            accumulated[0] += pixel[0] * alpha * weight;
            accumulated[1] += pixel[1] * alpha * weight;
            accumulated[2] += pixel[2] * alpha * weight;
            accumulated[3] += alpha * weight;
        }
    }

    if accumulated[3] <= 0.0 {
        return Some([0.0, 0.0, 0.0, 0.0]);
    }
    Some([
        clamp_unit(accumulated[0] / accumulated[3]),
        clamp_unit(accumulated[1] / accumulated[3]),
        clamp_unit(accumulated[2] / accumulated[3]),
        clamp_unit(accumulated[3]),
    ])
}

/// Samples a layer mask at a layer-space coordinate.
///
/// The mask is stored at the layer's full-resolution dimensions, so a preview
/// render samples it normalised against the layer's space rather than
/// resampling the whole coverage bitmap first.
fn mask_coverage(
    mask: Option<&MaskBitmap>,
    inverted: bool,
    local_x: f32,
    local_y: f32,
    space_width: u32,
    space_height: u32,
    interpolation: LayerInterpolation,
) -> f32 {
    let Some(mask) = mask else {
        return 1.0;
    };
    if space_width == 0 || space_height == 0 {
        return 0.0;
    }
    let u = local_x / space_width as f32;
    let v = local_y / space_height as f32;
    let x = (u * mask.width() as f32 - 0.5).clamp(0.0, mask.width().saturating_sub(1) as f32);
    let y = (v * mask.height() as f32 - 0.5).clamp(0.0, mask.height().saturating_sub(1) as f32);
    if !x.is_finite() || !y.is_finite() {
        return 0.0;
    }
    let (x0, y0) = if interpolation == LayerInterpolation::Nearest {
        // x/y are pixel-centre coordinates after subtracting 0.5. Rounding,
        // rather than flooring, keeps the mask on the same texel as artwork.
        (x.round() as u32, y.round() as u32)
    } else {
        (x.floor() as u32, y.floor() as u32)
    };
    // Nearest sampling keeps a mask edge exactly as hard as the artwork it
    // covers, which is the whole point of choosing it.
    let (x1, y1, fx, fy) = if interpolation == LayerInterpolation::Nearest {
        (x0, y0, 0.0, 0.0)
    } else {
        (
            (x0 + 1).min(mask.width() - 1),
            (y0 + 1).min(mask.height() - 1),
            x - x0 as f32,
            y - y0 as f32,
        )
    };
    let top = f32::from(mask.get(x0, y0)) * (1.0 - fx) + f32::from(mask.get(x1, y0)) * fx;
    let bottom = f32::from(mask.get(x0, y1)) * (1.0 - fx) + f32::from(mask.get(x1, y1)) * fx;
    let coverage = (top * (1.0 - fy) + bottom * fy) / 255.0;
    let coverage = clamp_unit(coverage);
    if inverted {
        1.0 - coverage
    } else {
        coverage
    }
}

#[cfg(test)]
pub(crate) mod testing {
    use super::*;
    use std::collections::HashMap;

    #[derive(Default)]
    pub struct MapSource {
        buffers: HashMap<String, Arc<RgbaImage>>,
    }

    impl MapSource {
        pub fn insert(&mut self, id: &str, image: RgbaImage) {
            self.buffers.insert(id.to_string(), Arc::new(image));
        }

        pub fn with(id: &str, image: RgbaImage) -> Self {
            let mut source = Self::default();
            source.insert(id, image);
            source
        }
    }

    impl PixelSource for MapSource {
        fn resolve(&self, pixel_id: &str) -> Result<Arc<RgbaImage>, AppError> {
            self.buffers
                .get(pixel_id)
                .cloned()
                .ok_or_else(|| AppError::LayerPixelsMissing(pixel_id.to_string()))
        }
    }

    pub fn solid(width: u32, height: u32, color: [u8; 4]) -> RgbaImage {
        RgbaImage::from_pixel(width, height, Rgba(color))
    }
}

#[cfg(test)]
mod tests {
    use super::testing::*;
    use super::*;
    use crate::domain::CropOverlay;
    use crate::layers::blend::BlendMode;
    use crate::layers::model::fixtures::*;
    use crate::layers::model::{LayerMask, LayerMetadata};
    use crate::mask::MaskSnapshot;

    const RED: [u8; 4] = [255, 0, 0, 255];
    const BLUE: [u8; 4] = [0, 0, 255, 255];
    const CLEAR: [u8; 4] = [0, 0, 0, 0];

    fn document_with(layers: Vec<Layer>, width: u32, height: u32) -> LayerDocument {
        LayerDocument {
            schema_version: crate::layers::model::LAYER_SCHEMA_VERSION,
            canvas_width: width,
            canvas_height: height,
            layers,
            active_layer_id: None,
        }
    }

    fn two_layer_source() -> MapSource {
        let mut source = MapSource::default();
        source.insert("pxbottom", solid(4, 4, BLUE));
        source.insert("pxtop", solid(4, 4, RED));
        source
    }

    fn stack(mode: BlendMode, top_opacity: f32) -> (LayerDocument, MapSource) {
        let mut top = pixel_layer("top", 4, 4);
        top.blend_mode = mode;
        top.opacity = top_opacity;
        (
            document_with(vec![pixel_layer("bottom", 4, 4), top], 4, 4),
            two_layer_source(),
        )
    }

    fn render(document: &LayerDocument, source: &MapSource) -> RgbaImage {
        render_document(document, source, RenderOptions::default()).unwrap()
    }

    #[test]
    fn an_empty_document_renders_a_transparent_canvas() {
        let document = document_with(vec![], 3, 2);
        let rendered = render(&document, &MapSource::default());
        assert_eq!(rendered.dimensions(), (3, 2));
        for pixel in rendered.pixels() {
            assert_eq!(pixel.0, CLEAR);
        }
    }

    #[test]
    fn a_single_opaque_layer_reproduces_its_pixels_exactly() {
        let document = document_with(vec![pixel_layer("only", 4, 4)], 4, 4);
        let source = MapSource::with("pxonly", solid(4, 4, RED));
        let rendered = render(&document, &source);
        for pixel in rendered.pixels() {
            assert_eq!(pixel.0, RED);
        }
    }

    #[test]
    fn red_over_blue_in_normal_mode_shows_only_red() {
        let (document, source) = stack(BlendMode::Normal, 1.0);
        let rendered = render(&document, &source);
        assert_eq!(rendered.get_pixel(0, 0).0, RED);
        assert_eq!(rendered.get_pixel(3, 3).0, RED);
    }

    #[test]
    fn fifty_percent_red_over_blue_is_the_documented_midpoint() {
        let (document, source) = stack(BlendMode::Normal, 0.5);
        let rendered = render(&document, &source);
        // 255 * 0.5 rounds to 128 on red; blue keeps 255 * 0.5 = 128.
        assert_eq!(rendered.get_pixel(1, 1).0, [128, 0, 128, 255]);
    }

    #[test]
    fn multiply_screen_and_overlay_match_hand_computed_values() {
        let mut source = MapSource::default();
        source.insert("pxbottom", solid(2, 2, [200, 100, 50, 255]));
        source.insert("pxtop", solid(2, 2, [128, 128, 128, 255]));

        let cases = [
            // multiply: round(200/255 * 128/255 * 255) = 100
            (BlendMode::Multiply, [100, 50, 25, 255]),
            // screen: b + s - b*s
            (BlendMode::Screen, [228, 178, 153, 255]),
            // overlay with a 0.5 source is an identity on the backdrop
            (BlendMode::Overlay, [200, 100, 50, 255]),
            (BlendMode::Darken, [128, 100, 50, 255]),
            (BlendMode::Lighten, [200, 128, 128, 255]),
            // difference: |b - s|
            (BlendMode::Difference, [72, 28, 78, 255]),
        ];
        for (mode, expected) in cases {
            let mut top = pixel_layer("top", 2, 2);
            top.blend_mode = mode;
            let document = document_with(vec![pixel_layer("bottom", 2, 2), top], 2, 2);
            let rendered = render(&document, &source);
            let actual = rendered.get_pixel(0, 0).0;
            for channel in 0..4 {
                assert!(
                    actual[channel].abs_diff(expected[channel]) <= 1,
                    "{}: {actual:?} != {expected:?}",
                    mode.id()
                );
            }
        }
    }

    #[test]
    fn rendering_is_deterministic_across_repeated_runs() {
        let (document, source) = stack(BlendMode::Overlay, 0.63);
        let first = render(&document, &source);
        let second = render(&document, &source);
        let third = render_document(&document, &source, RenderOptions::default()).unwrap();
        assert_eq!(first.as_raw(), second.as_raw());
        assert_eq!(first.as_raw(), third.as_raw());
    }

    #[test]
    fn hidden_and_fully_transparent_layers_contribute_nothing() {
        let mut hidden = pixel_layer("top", 4, 4);
        hidden.visible = false;
        let document = document_with(vec![pixel_layer("bottom", 4, 4), hidden], 4, 4);
        let source = two_layer_source();
        assert_eq!(render(&document, &source).get_pixel(0, 0).0, BLUE);

        let mut transparent = pixel_layer("top", 4, 4);
        transparent.opacity = 0.0;
        let document = document_with(vec![pixel_layer("bottom", 4, 4), transparent], 4, 4);
        assert_eq!(render(&document, &source).get_pixel(0, 0).0, BLUE);
    }

    #[test]
    fn a_locked_layer_still_renders_because_locking_only_blocks_editing() {
        let mut locked = pixel_layer("only", 4, 4);
        locked.locked = true;
        let document = document_with(vec![locked], 4, 4);
        let source = MapSource::with("pxonly", solid(4, 4, RED));
        assert_eq!(render(&document, &source).get_pixel(0, 0).0, RED);
    }

    #[test]
    fn a_missing_pixel_buffer_fails_closed() {
        let document = document_with(vec![pixel_layer("only", 4, 4)], 4, 4);
        assert!(matches!(
            render_document(&document, &MapSource::default(), RenderOptions::default()),
            Err(AppError::LayerPixelsMissing(_))
        ));
    }

    #[test]
    fn a_layer_mask_produces_partial_coverage() {
        let mut mask = MaskBitmap::empty(4, 4).unwrap();
        for y in 0..4 {
            mask.set(0, y, 255);
            mask.set(1, y, 128);
        }
        let mut top = pixel_layer("top", 4, 4);
        top.mask = Some(LayerMask {
            snapshot: MaskSnapshot::encode(&mask),
            enabled: true,
            inverted: false,
        });
        let document = document_with(vec![pixel_layer("bottom", 4, 4), top], 4, 4);
        let source = two_layer_source();
        let rendered = render(&document, &source);
        assert_eq!(rendered.get_pixel(0, 0).0, RED);
        let partial = rendered.get_pixel(1, 0).0;
        assert!(partial[0].abs_diff(128) <= 1 && partial[2].abs_diff(127) <= 1);
        assert_eq!(rendered.get_pixel(3, 0).0, BLUE);
    }

    #[test]
    fn a_disabled_mask_is_ignored_and_an_inverted_mask_swaps_coverage() {
        let mask = MaskBitmap::empty(4, 4).unwrap();
        let mut top = pixel_layer("top", 4, 4);
        top.mask = Some(LayerMask {
            snapshot: MaskSnapshot::encode(&mask),
            enabled: false,
            inverted: false,
        });
        let document = document_with(vec![pixel_layer("bottom", 4, 4), top.clone()], 4, 4);
        let source = two_layer_source();
        assert_eq!(render(&document, &source).get_pixel(0, 0).0, RED);

        if let Some(mask) = top.mask.as_mut() {
            mask.enabled = true;
        }
        let document = document_with(vec![pixel_layer("bottom", 4, 4), top.clone()], 4, 4);
        assert_eq!(render(&document, &source).get_pixel(0, 0).0, BLUE);

        if let Some(mask) = top.mask.as_mut() {
            mask.inverted = true;
        }
        let document = document_with(vec![pixel_layer("bottom", 4, 4), top], 4, 4);
        assert_eq!(render(&document, &source).get_pixel(0, 0).0, RED);
    }

    #[test]
    fn masks_and_opacity_multiply_together() {
        let mut mask = MaskBitmap::empty(2, 2).unwrap();
        for y in 0..2 {
            for x in 0..2 {
                mask.set(x, y, 128);
            }
        }
        let mut top = pixel_layer("top", 2, 2);
        top.opacity = 0.5;
        top.mask = Some(LayerMask {
            snapshot: MaskSnapshot::encode(&mask),
            enabled: true,
            inverted: false,
        });
        let mut source = MapSource::default();
        source.insert("pxbottom", solid(2, 2, BLUE));
        source.insert("pxtop", solid(2, 2, RED));
        let document = document_with(vec![pixel_layer("bottom", 2, 2), top], 2, 2);
        let rendered = render(&document, &source);
        // coverage = 0.5 * (128/255) ~= 0.251
        let pixel = rendered.get_pixel(0, 0).0;
        assert!(pixel[0].abs_diff(64) <= 2, "{pixel:?}");
        assert!(pixel[2].abs_diff(191) <= 2, "{pixel:?}");
        assert_eq!(pixel[3], 255);
    }

    #[test]
    fn transparent_layers_compose_without_halos_at_their_edges() {
        let mut top_image = solid(4, 4, CLEAR);
        top_image.put_pixel(1, 1, Rgba([255, 255, 255, 255]));
        let mut source = MapSource::default();
        source.insert("pxbottom", solid(4, 4, [0, 0, 0, 255]));
        source.insert("pxtop", top_image);
        let document = document_with(
            vec![pixel_layer("bottom", 4, 4), pixel_layer("top", 4, 4)],
            4,
            4,
        );
        let rendered = render(&document, &source);
        assert_eq!(rendered.get_pixel(1, 1).0, [255, 255, 255, 255]);
        // Neighbours of the opaque pixel keep the backdrop exactly; a halo would
        // show up as a non-zero colour channel here.
        assert_eq!(rendered.get_pixel(0, 1).0, [0, 0, 0, 255]);
        assert_eq!(rendered.get_pixel(2, 1).0, [0, 0, 0, 255]);
    }

    #[test]
    fn opaque_over_transparent_keeps_the_source_alpha() {
        let document = document_with(vec![pixel_layer("only", 2, 2)], 2, 2);
        let source = MapSource::with("pxonly", solid(2, 2, [10, 20, 30, 255]));
        let rendered = render(&document, &source);
        assert_eq!(rendered.get_pixel(0, 0).0, [10, 20, 30, 255]);
    }

    #[test]
    fn translucent_over_transparent_preserves_straight_alpha_colour() {
        let document = document_with(vec![pixel_layer("only", 2, 2)], 2, 2);
        let source = MapSource::with("pxonly", solid(2, 2, [200, 100, 50, 128]));
        let rendered = render(&document, &source);
        assert_eq!(rendered.get_pixel(0, 0).0, [200, 100, 50, 128]);
    }

    #[test]
    fn translucent_over_translucent_matches_the_alpha_formula() {
        let mut source = MapSource::default();
        source.insert("pxbottom", solid(2, 2, [255, 0, 0, 128]));
        source.insert("pxtop", solid(2, 2, [0, 0, 255, 128]));
        let document = document_with(
            vec![pixel_layer("bottom", 2, 2), pixel_layer("top", 2, 2)],
            2,
            2,
        );
        let rendered = render(&document, &source);
        let pixel = rendered.get_pixel(0, 0).0;
        // ao = a + a(1-a) with a = 128/255 gives roughly 0.753 -> 192.
        assert!(pixel[3].abs_diff(192) <= 1, "{pixel:?}");
        assert!(pixel[2].abs_diff(170) <= 2, "{pixel:?}");
        assert!(pixel[0].abs_diff(85) <= 2, "{pixel:?}");
    }

    #[test]
    fn a_layer_smaller_than_the_canvas_only_covers_its_own_area() {
        let mut small = pixel_layer("small", 2, 2);
        small.transform = LayerTransform {
            translate_x: 1.0,
            translate_y: 1.0,
            ..LayerTransform::default()
        };
        let document = document_with(vec![small], 4, 4);
        let source = MapSource::with("pxsmall", solid(2, 2, RED));
        let rendered = render(&document, &source);
        assert_eq!(rendered.get_pixel(1, 1).0, RED);
        assert_eq!(rendered.get_pixel(2, 2).0, RED);
        assert_eq!(rendered.get_pixel(0, 0).0, CLEAR);
        assert_eq!(rendered.get_pixel(3, 3).0, CLEAR);
    }

    #[test]
    fn a_layer_larger_than_the_canvas_is_clipped_without_error() {
        let document = document_with(vec![pixel_layer("big", 8, 8)], 4, 4);
        let source = MapSource::with("pxbig", solid(8, 8, RED));
        let rendered = render(&document, &source);
        assert_eq!(rendered.dimensions(), (4, 4));
        for pixel in rendered.pixels() {
            assert_eq!(pixel.0, RED);
        }
    }

    #[test]
    fn a_layer_entirely_outside_the_canvas_contributes_nothing() {
        let mut offscreen = pixel_layer("gone", 2, 2);
        offscreen.transform = LayerTransform {
            translate_x: 500.0,
            translate_y: 500.0,
            ..LayerTransform::default()
        };
        let document = document_with(vec![offscreen], 4, 4);
        let source = MapSource::with("pxgone", solid(2, 2, RED));
        let rendered = render(&document, &source);
        for pixel in rendered.pixels() {
            assert_eq!(pixel.0, CLEAR);
        }
    }

    #[test]
    fn a_partially_offscreen_layer_keeps_the_visible_part() {
        let mut half = pixel_layer("half", 4, 4);
        half.transform = LayerTransform {
            translate_x: -2.0,
            translate_y: 0.0,
            ..LayerTransform::default()
        };
        let document = document_with(vec![half], 4, 4);
        let source = MapSource::with("pxhalf", solid(4, 4, RED));
        let rendered = render(&document, &source);
        assert_eq!(rendered.get_pixel(0, 0).0, RED);
        assert_eq!(rendered.get_pixel(1, 0).0, RED);
        assert_eq!(rendered.get_pixel(2, 0).0, CLEAR);
    }

    #[test]
    fn integer_translation_moves_pixels_without_resampling() {
        let mut image = solid(4, 4, CLEAR);
        image.put_pixel(0, 0, Rgba([12, 34, 56, 255]));
        let mut layer = pixel_layer("shift", 4, 4);
        layer.transform = LayerTransform {
            translate_x: 2.0,
            translate_y: 1.0,
            ..LayerTransform::default()
        };
        let document = document_with(vec![layer], 4, 4);
        let source = MapSource::with("pxshift", image);
        let rendered = render(&document, &source);
        assert_eq!(rendered.get_pixel(2, 1).0, [12, 34, 56, 255]);
        assert_eq!(rendered.get_pixel(0, 0).0, CLEAR);
    }

    #[test]
    fn flips_mirror_layer_content_in_place() {
        let mut image = solid(2, 1, CLEAR);
        image.put_pixel(0, 0, Rgba(RED));
        let mut layer = pixel_layer("flip", 2, 1);
        layer.transform = LayerTransform {
            flip_horizontal: true,
            ..LayerTransform::default()
        };
        let document = document_with(vec![layer], 2, 1);
        let source = MapSource::with("pxflip", image);
        let rendered = render(&document, &source);
        assert_eq!(rendered.get_pixel(1, 0).0, RED);
        assert_eq!(rendered.get_pixel(0, 0).0, CLEAR);
    }

    #[test]
    fn a_layer_mask_travels_with_its_layer_transform() {
        let mut mask = MaskBitmap::empty(2, 2).unwrap();
        mask.set(0, 0, 255);
        let mut layer = pixel_layer("masked", 2, 2);
        layer.mask = Some(LayerMask {
            snapshot: MaskSnapshot::encode(&mask),
            enabled: true,
            inverted: false,
        });
        layer.transform = LayerTransform {
            translate_x: 2.0,
            translate_y: 2.0,
            ..LayerTransform::default()
        };
        let document = document_with(vec![layer], 4, 4);
        let source = MapSource::with("pxmasked", solid(2, 2, RED));
        let rendered = render(&document, &source);
        // The mask hole started at layer pixel (0,0) and moved with the layer.
        assert_eq!(rendered.get_pixel(2, 2).0, RED);
        assert_eq!(rendered.get_pixel(3, 3).0, CLEAR);
    }

    #[test]
    fn an_adjustment_layer_recomputes_from_parameters_and_keeps_alpha() {
        let adjustment = adjustment_layer("adj", EditOperation::Brightness { amount: 0.2 });
        let document = document_with(vec![pixel_layer("base", 2, 2), adjustment], 2, 2);
        let source = MapSource::with("pxbase", solid(2, 2, [100, 100, 100, 255]));
        let rendered = render(&document, &source);
        // 0.2 * 255 = 51 added to each channel.
        assert_eq!(rendered.get_pixel(0, 0).0, [151, 151, 151, 255]);
    }

    #[test]
    fn an_adjustment_layer_never_adds_coverage_to_transparent_pixels() {
        let adjustment = adjustment_layer("adj", EditOperation::Brightness { amount: 0.5 });
        let document = document_with(vec![pixel_layer("base", 2, 2), adjustment], 2, 2);
        let source = MapSource::with("pxbase", solid(2, 2, CLEAR));
        let rendered = render(&document, &source);
        for pixel in rendered.pixels() {
            assert_eq!(pixel.0[3], 0, "adjustment layers must not create coverage");
        }
    }

    #[test]
    fn adjustment_opacity_and_masks_scale_the_effect() {
        let mut adjustment = adjustment_layer("adj", EditOperation::Brightness { amount: 0.4 });
        adjustment.opacity = 0.5;
        let document = document_with(vec![pixel_layer("base", 2, 2), adjustment], 2, 2);
        let source = MapSource::with("pxbase", solid(2, 2, [100, 100, 100, 255]));
        let rendered = render(&document, &source);
        // Full strength adds 102; half opacity adds 51.
        let pixel = rendered.get_pixel(0, 0).0;
        assert!(pixel[0].abs_diff(151) <= 1, "{pixel:?}");
    }

    #[test]
    fn an_adjustment_layer_only_affects_layers_below_it_in_its_own_group() {
        let adjustment = adjustment_layer("adj", EditOperation::Grayscale);
        let mut source = MapSource::default();
        source.insert("pxbottom", solid(2, 2, RED));
        source.insert("pxtop", solid(2, 2, [0, 0, 255, 255]));
        let document = document_with(
            vec![
                pixel_layer("bottom", 2, 2),
                adjustment,
                pixel_layer("top", 2, 2),
            ],
            2,
            2,
        );
        let rendered = render(&document, &source);
        // The top layer sits above the adjustment and keeps its colour.
        assert_eq!(rendered.get_pixel(0, 0).0, [0, 0, 255, 255]);
    }

    #[test]
    fn adjustment_layers_reject_geometry_operations_at_render_time() {
        let mut layer = pixel_layer("adj", 2, 2);
        layer.content = LayerContent::Adjustment {
            operation: Box::new(EditOperation::Crop {
                x: 0.0,
                y: 0.0,
                width: 0.5,
                height: 0.5,
                aspect_ratio: None,
                overlay: CropOverlay::None,
            }),
        };
        let document = document_with(vec![layer], 2, 2);
        assert!(matches!(
            render_document(&document, &MapSource::default(), RenderOptions::default()),
            Err(AppError::UnsupportedAdjustmentLayer(_))
        ));
    }

    #[test]
    fn groups_composite_their_children_in_order() {
        let group = group_layer(
            "g",
            vec![pixel_layer("bottom", 4, 4), pixel_layer("top", 4, 4)],
        );
        let document = document_with(vec![group], 4, 4);
        let source = two_layer_source();
        assert_eq!(render(&document, &source).get_pixel(0, 0).0, RED);
    }

    #[test]
    fn group_opacity_applies_to_the_finished_group_not_each_child() {
        // Two stacked opaque children inside a 50% group must read as a single
        // 50% result, which is what isolated group compositing guarantees.
        let mut group = group_layer(
            "g",
            vec![pixel_layer("bottom", 2, 2), pixel_layer("top", 2, 2)],
        );
        group.opacity = 0.5;
        let mut source = two_layer_source();
        source.insert("pxback", solid(2, 2, [0, 0, 0, 255]));
        let document = document_with(vec![pixel_layer("back", 2, 2), group], 2, 2);
        let rendered = render(&document, &source);
        let pixel = rendered.get_pixel(0, 0).0;
        assert!(pixel[0].abs_diff(128) <= 1, "{pixel:?}");
        assert_eq!(pixel[2], 0);
    }

    #[test]
    fn nested_group_transparency_accumulates_correctly() {
        let inner = group_layer("inner", vec![pixel_layer("top", 2, 2)]);
        let mut inner_with_opacity = inner;
        inner_with_opacity.opacity = 0.5;
        let mut outer = group_layer("outer", vec![inner_with_opacity]);
        outer.opacity = 0.5;
        let mut source = two_layer_source();
        source.insert("pxback", solid(2, 2, [0, 0, 0, 255]));
        let document = document_with(vec![pixel_layer("back", 2, 2), outer], 2, 2);
        let rendered = render(&document, &source);
        // 0.5 * 0.5 = 0.25 of red over black.
        let pixel = rendered.get_pixel(0, 0).0;
        assert!(pixel[0].abs_diff(64) <= 2, "{pixel:?}");
        assert_eq!(pixel[3], 255);
    }

    #[test]
    fn an_empty_group_is_a_no_op() {
        let document = document_with(
            vec![pixel_layer("base", 2, 2), group_layer("g", vec![])],
            2,
            2,
        );
        let source = MapSource::with("pxbase", solid(2, 2, RED));
        assert_eq!(render(&document, &source).get_pixel(0, 0).0, RED);
    }

    #[test]
    fn a_group_mask_and_blend_mode_apply_to_the_whole_group() {
        let mut group = group_layer("g", vec![pixel_layer("top", 2, 2)]);
        group.blend_mode = BlendMode::Multiply;
        let mut source = two_layer_source();
        source.insert("pxback", solid(2, 2, [128, 128, 128, 255]));
        let document = document_with(vec![pixel_layer("back", 2, 2), group], 2, 2);
        let rendered = render(&document, &source);
        // multiply(128, red) keeps the red channel and zeroes the others.
        let pixel = rendered.get_pixel(0, 0).0;
        assert_eq!(pixel[0], 128);
        assert_eq!(pixel[1], 0);
        assert_eq!(pixel[2], 0);
    }

    #[test]
    fn an_adjustment_inside_a_group_does_not_reach_outside_it() {
        let group = group_layer(
            "g",
            vec![
                pixel_layer("top", 2, 2),
                adjustment_layer("adj", EditOperation::Grayscale),
            ],
        );
        let mut source = two_layer_source();
        source.insert("pxback", solid(2, 2, BLUE));
        let document = document_with(vec![pixel_layer("back", 2, 2), group], 2, 2);
        let rendered = render(&document, &source);
        // Red inside the group became grey; the blue backdrop outside is intact
        // beneath an opaque grey, so the result is the grey value.
        let pixel = rendered.get_pixel(0, 0).0;
        assert_eq!(pixel[0], pixel[1]);
        assert_eq!(pixel[1], pixel[2]);
        assert_ne!(pixel[0], 0);
    }

    #[test]
    fn group_recursion_is_rejected_beyond_the_depth_limit() {
        let mut layer = pixel_layer("leaf", 2, 2);
        for index in 0..MAX_GROUP_DEPTH {
            layer = group_layer(&format!("g{index}"), vec![layer]);
        }
        let document = document_with(vec![layer], 2, 2);
        assert!(matches!(
            render_document(&document, &MapSource::default(), RenderOptions::default()),
            Err(AppError::LayerDepthExceeded { .. })
        ));
    }

    #[test]
    fn a_preview_scale_produces_a_proportional_canvas() {
        let document = document_with(vec![pixel_layer("only", 8, 8)], 8, 8);
        let mut source = MapSource::default();
        source.insert("pxonly", solid(4, 4, RED));
        let rendered = render_document(
            &document,
            &source,
            RenderOptions {
                scale: 0.5,
                cancel: None,
            },
        )
        .unwrap();
        assert_eq!(rendered.dimensions(), (4, 4));
        assert_eq!(rendered.get_pixel(0, 0).0, RED);
    }

    #[test]
    fn preview_scale_moves_translations_proportionally() {
        let mut layer = pixel_layer("only", 8, 8);
        layer.transform = LayerTransform {
            translate_x: 4.0,
            translate_y: 0.0,
            ..LayerTransform::default()
        };
        let document = document_with(vec![layer], 8, 8);
        let mut source = MapSource::default();
        source.insert("pxonly", solid(4, 4, RED));
        let rendered = render_document(
            &document,
            &source,
            RenderOptions {
                scale: 0.5,
                cancel: None,
            },
        )
        .unwrap();
        // A 4px translation at half scale lands at 2px in the preview.
        assert_eq!(rendered.get_pixel(2, 0).0, RED);
        assert_eq!(rendered.get_pixel(1, 0).0, CLEAR);
    }

    #[test]
    fn invalid_render_scales_are_rejected() {
        let document = document_with(vec![], 4, 4);
        for scale in [0.0, -1.0, 2.0, f64::NAN] {
            assert!(render_document(
                &document,
                &MapSource::default(),
                RenderOptions {
                    scale,
                    cancel: None
                }
            )
            .is_err());
        }
    }

    #[test]
    fn a_render_can_be_cancelled() {
        let cancel = AtomicBool::new(true);
        let document = document_with(vec![pixel_layer("only", 4, 4)], 4, 4);
        let source = MapSource::with("pxonly", solid(4, 4, RED));
        assert!(matches!(
            render_document(
                &document,
                &source,
                RenderOptions {
                    scale: 1.0,
                    cancel: Some(&cancel)
                }
            ),
            Err(AppError::RenderCancelled)
        ));
    }

    #[test]
    fn a_cancelled_empty_document_does_not_report_a_successful_render() {
        let cancel = AtomicBool::new(true);
        assert!(matches!(
            render_document(
                &document_with(vec![], 4, 4),
                &MapSource::default(),
                RenderOptions {
                    cancel: Some(&cancel),
                    ..RenderOptions::default()
                }
            ),
            Err(AppError::RenderCancelled)
        ));
    }

    #[test]
    fn serial_row_bands_stop_between_rows_and_detect_cancellation_on_the_last_row() {
        for height in [1, 4] {
            let cancel = AtomicBool::new(false);
            let count = std::sync::atomic::AtomicUsize::new(0);
            let mut pixels = vec![0; height as usize * 4];
            let result = for_each_row_band_with_threads(
                &mut pixels,
                1,
                0,
                height,
                Some(&cancel),
                1,
                |_, row| {
                    row.fill(255);
                    count.fetch_add(1, Ordering::Relaxed);
                    cancel.store(true, Ordering::Release);
                },
            );
            assert!(matches!(result, Err(AppError::RenderCancelled)));
            assert_eq!(count.load(Ordering::Relaxed), 1);
            assert!(pixels[4..].iter().all(|value| *value == 0));
        }
    }

    #[test]
    fn parallel_row_bands_stop_between_rows_without_timing_assumptions() {
        let height = 2 * MIN_ROWS_PER_THREAD;
        let cancel = AtomicBool::new(false);
        let count = std::sync::atomic::AtomicUsize::new(0);
        let started = std::sync::Barrier::new(2);
        let mut pixels = vec![0; height as usize * 4];
        let result = for_each_row_band_with_threads(
            &mut pixels,
            1,
            0,
            height,
            Some(&cancel),
            2,
            |y, row| {
                row.fill(255);
                count.fetch_add(1, Ordering::Relaxed);
                // Both workers enter their first row before either cancels.
                // Only those two in-flight rows may finish after the signal.
                if y == 0 || y == MIN_ROWS_PER_THREAD {
                    started.wait();
                }
                cancel.store(true, Ordering::Release);
            },
        );
        assert!(matches!(result, Err(AppError::RenderCancelled)));
        assert_eq!(count.load(Ordering::Relaxed), 2);
        assert_eq!(
            pixels.chunks_exact(4).filter(|row| row[0] == 255).count(),
            2
        );
    }

    #[test]
    fn row_bands_preserve_nonzero_region_offsets_and_match_serial_output() {
        let width = 7;
        let height = 211;
        let original = vec![17; width as usize * height as usize * 4];
        let draw = |y: u32, row: &mut [u8]| {
            for (index, value) in row.iter_mut().enumerate() {
                *value = ((y * 13 + index as u32 * 7) % 256) as u8;
            }
        };
        let mut serial = original.clone();
        for_each_row_band_with_threads(&mut serial, width, 5, 204, None, 1, draw).unwrap();
        for threads in [2, 3, 8] {
            let mut parallel = original.clone();
            for_each_row_band_with_threads(&mut parallel, width, 5, 204, None, threads, draw)
                .unwrap();
            assert_eq!(parallel, serial, "{threads} workers changed the result");
        }
        assert_eq!(
            &serial[..5 * width as usize * 4],
            &original[..5 * width as usize * 4]
        );
        assert_eq!(
            &serial[204 * width as usize * 4..],
            &original[204 * width as usize * 4..]
        );
    }

    #[test]
    fn source_over_fast_path_honours_cancellation_before_writing() {
        let cancel = AtomicBool::new(true);
        let source = solid(4, 4, RED);
        let mut backdrop = solid(4, 4, BLUE);
        assert!(matches!(
            draw_source_over(&mut backdrop, &source, 0, 0, (0, 0, 4, 4), Some(&cancel)),
            Err(AppError::RenderCancelled)
        ));
        assert_eq!(backdrop, solid(4, 4, BLUE));
    }

    #[test]
    fn an_invalid_document_is_rejected_before_any_pixel_work() {
        let mut layer = pixel_layer("a", 4, 4);
        layer.opacity = 5.0;
        let document = document_with(vec![layer], 4, 4);
        assert!(
            render_document(&document, &MapSource::default(), RenderOptions::default()).is_err()
        );
    }

    #[test]
    fn sampling_outside_a_buffer_returns_nothing_rather_than_wrapping() {
        let image = solid(4, 4, RED);
        assert!(sample_rgba(&image, -0.1, 1.0, LayerInterpolation::Bilinear).is_none());
        assert!(sample_rgba(&image, 4.0, 1.0, LayerInterpolation::Bilinear).is_none());
        assert!(sample_rgba(&image, f32::NAN, 1.0, LayerInterpolation::Bilinear).is_none());
        assert!(sample_rgba(&image, 3.9, 3.9, LayerInterpolation::Bilinear).is_some());
    }

    #[test]
    fn whole_pixel_sampling_is_an_exact_copy() {
        let mut image = solid(2, 2, CLEAR);
        image.put_pixel(1, 1, Rgba([3, 5, 7, 199]));
        let sample = sample_rgba(&image, 1.5, 1.5, LayerInterpolation::Bilinear).unwrap();
        assert_eq!(pack(sample).0, [3, 5, 7, 199]);
    }

    #[test]
    fn mask_coverage_is_neutral_without_a_mask_and_clamped_with_one() {
        assert_eq!(
            mask_coverage(None, false, 0.0, 0.0, 4, 4, LayerInterpolation::Bilinear),
            1.0
        );
        let mut mask = MaskBitmap::empty(4, 4).unwrap();
        mask.set(0, 0, 255);
        let coverage = mask_coverage(
            Some(&mask),
            false,
            0.5,
            0.5,
            4,
            4,
            LayerInterpolation::Bilinear,
        );
        assert!((coverage - 1.0).abs() < 1e-5);
        let inverted = mask_coverage(
            Some(&mask),
            true,
            0.5,
            0.5,
            4,
            4,
            LayerInterpolation::Bilinear,
        );
        assert!(inverted.abs() < 1e-5);
    }

    #[test]
    fn fifty_layers_composite_without_error_and_stay_deterministic() {
        let mut source = MapSource::default();
        let mut layers = Vec::new();
        for index in 0..50 {
            let id = format!("l{index}");
            source.insert(&format!("px{id}"), solid(8, 8, [index as u8, 10, 20, 128]));
            let mut layer = pixel_layer(&id, 8, 8);
            layer.opacity = 0.5;
            layers.push(layer);
        }
        let document = document_with(layers, 8, 8);
        let first = render(&document, &source);
        let second = render(&document, &source);
        assert_eq!(first.as_raw(), second.as_raw());
        // Alpha accumulates towards opaque. It settles one step short of 255
        // because each layer's result is quantized back to 8 bits, which is an
        // inherent property of an 8-bit compositor rather than a lost layer.
        assert!(
            first.get_pixel(0, 0).0[3] >= 254,
            "{:?}",
            first.get_pixel(0, 0).0
        );
    }

    /// The behaviour every existing PhotoForge workflow depends on: a document
    /// that is one ordinary opaque layer must composite to exactly the pixels
    /// that were opened, so the document pipeline on top of it produces the
    /// same result the destructive Phase 7.1 path produced.
    #[test]
    fn a_single_layer_document_matches_the_destructive_pipeline_byte_for_byte() {
        let mut buffer = RgbaImage::new(6, 5);
        for (index, pixel) in buffer.pixels_mut().enumerate() {
            let value = (index * 7 % 256) as u8;
            *pixel = Rgba([value, 255 - value, value / 2, 255]);
        }

        let document = document_with(vec![pixel_layer("background", 6, 5)], 6, 5);
        let source = MapSource::with("pxbackground", buffer.clone());
        let composited = render(&document, &source);
        assert_eq!(composited.as_raw(), buffer.as_raw());

        let operations = vec![
            EditOperation::Brightness { amount: 0.1 },
            EditOperation::Contrast { amount: 0.2 },
            EditOperation::Grayscale,
        ];
        let through_layers = crate::image_processing::apply_pipeline(
            &image::DynamicImage::ImageRgba8(composited),
            &operations,
        )
        .unwrap();
        let directly = crate::image_processing::apply_pipeline(
            &image::DynamicImage::ImageRgba8(buffer),
            &operations,
        )
        .unwrap();
        assert_eq!(
            through_layers.to_rgba8().as_raw(),
            directly.to_rgba8().as_raw()
        );
    }

    /// The opaque whole-pixel fast path must be indistinguishable from the
    /// general sampling path. A fully revealing mask forces the general path
    /// while leaving coverage at 1.0, so the two results have to match exactly.
    #[test]
    fn the_opaque_fast_path_matches_the_general_path_byte_for_byte() {
        for (width, height) in [(6, 5), (11, 201)] {
            let mut top = RgbaImage::new(width, height);
            let mut bottom = RgbaImage::new(width, height);
            for (index, (upper, lower)) in top.pixels_mut().zip(bottom.pixels_mut()).enumerate() {
                let value = (index * 11 % 256) as u8;
                let alpha = [0, 1, 64, 128, 254, 255][index % 6];
                *upper = Rgba([value, 255 - value, value / 3, alpha]);
                *lower = Rgba([255 - value, value / 2, value, 255 - alpha]);
            }
            let mut source = MapSource::default();
            source.insert("pxbottom", bottom);
            source.insert("pxtop", top);

            for interpolation in [LayerInterpolation::Bilinear, LayerInterpolation::Nearest] {
                for translation in [(0.0, 0.0), (2.0, 1.0), (-1.0, 3.0)] {
                    let mut fast = pixel_layer("top", width, height);
                    fast.transform = LayerTransform {
                        translate_x: translation.0,
                        translate_y: translation.1,
                        interpolation,
                        ..LayerTransform::default()
                    };
                    let mut general = fast.clone();
                    general.mask = Some(LayerMask {
                        snapshot: MaskSnapshot::encode(&MaskBitmap::full(width, height).unwrap()),
                        enabled: true,
                        inverted: false,
                    });

                    let fast_result = render(
                        &document_with(
                            vec![pixel_layer("bottom", width, height), fast],
                            width,
                            height,
                        ),
                        &source,
                    );
                    let general_result = render(
                        &document_with(
                            vec![pixel_layer("bottom", width, height), general],
                            width,
                            height,
                        ),
                        &source,
                    );
                    assert_eq!(
                        fast_result.as_raw(),
                        general_result.as_raw(),
                        "paths disagreed for {width}x{height} {interpolation:?} at {translation:?}"
                    );
                }
            }
        }
    }

    #[test]
    fn the_fast_path_still_honours_partially_transparent_source_pixels() {
        let mut source = MapSource::default();
        source.insert("pxbottom", solid(2, 2, [0, 0, 0, 255]));
        source.insert("pxtop", solid(2, 2, [255, 255, 255, 128]));
        let document = document_with(
            vec![pixel_layer("bottom", 2, 2), pixel_layer("top", 2, 2)],
            2,
            2,
        );
        let pixel = render(&document, &source).get_pixel(0, 0).0;
        assert!(pixel[0].abs_diff(128) <= 1, "{pixel:?}");
        assert_eq!(pixel[3], 255);
    }

    /// The compositor splits tall regions across worker threads. Row bands are
    /// disjoint and nothing is reduced, so the result must not depend on how
    /// the work was divided or scheduled.
    #[test]
    fn a_canvas_tall_enough_to_be_split_across_threads_stays_deterministic() {
        let (width, height) = (40_u32, 400_u32);
        let mut bottom = RgbaImage::new(width, height);
        let mut top = RgbaImage::new(width, height);
        for y in 0..height {
            for x in 0..width {
                let value = ((x * 7 + y * 13) % 256) as u8;
                bottom.put_pixel(x, y, Rgba([value, 255 - value, value / 2, 255]));
                top.put_pixel(x, y, Rgba([255 - value, value, value / 3, 200]));
            }
        }
        let mut source = MapSource::default();
        source.insert("pxbottom", bottom);
        source.insert("pxtop", top);

        let mut upper = pixel_layer("top", width, height);
        upper.blend_mode = BlendMode::Overlay;
        upper.opacity = 0.63;
        let mut mask = MaskBitmap::empty(width, height).unwrap();
        for y in 0..height {
            for x in 0..width {
                mask.set(x, y, ((x + y) % 256) as u8);
            }
        }
        upper.mask = Some(LayerMask {
            snapshot: MaskSnapshot::encode(&mask),
            enabled: true,
            inverted: false,
        });

        let document = document_with(
            vec![
                pixel_layer("bottom", width, height),
                upper,
                adjustment_layer("adj", EditOperation::Contrast { amount: 0.2 }),
            ],
            width,
            height,
        );

        let first = render(&document, &source);
        for _ in 0..4 {
            assert_eq!(render(&document, &source).as_raw(), first.as_raw());
        }
        // A pixel from the middle of the canvas actually changed, so the test
        // is not comparing two blank buffers.
        assert_ne!(first.get_pixel(20, 200).0, [0, 0, 0, 0]);
    }

    // -- Interpolation ----------------------------------------------------

    /// A two-tone strip scaled up: bilinear blends across the boundary, nearest
    /// must not. Nothing in the nearest result may be a colour the source never
    /// contained, which is exactly what interpolation would introduce.
    #[test]
    fn nearest_sampling_keeps_a_hard_edge_that_bilinear_softens() {
        let mut strip = RgbaImage::new(2, 1);
        strip.put_pixel(0, 0, Rgba([0, 0, 0, 255]));
        strip.put_pixel(1, 0, Rgba([255, 255, 255, 255]));
        let source = MapSource::with("pxart", strip);

        let scaled = |interpolation: LayerInterpolation| {
            let mut layer = pixel_layer("art", 2, 1);
            layer.transform = LayerTransform {
                scale_x: 8.0,
                scale_y: 8.0,
                interpolation,
                ..LayerTransform::default()
            };
            render(&document_with(vec![layer], 16, 8), &source)
        };

        let smooth = scaled(LayerInterpolation::Bilinear);
        let hard = scaled(LayerInterpolation::Nearest);
        for pixel in hard.pixels().filter(|pixel| pixel.0[3] > 0) {
            assert!(
                pixel.0[0] == 0 || pixel.0[0] == 255,
                "nearest produced {pixel:?}"
            );
        }
        assert!(
            smooth
                .pixels()
                .any(|pixel| pixel.0[3] > 0 && pixel.0[0] > 0 && pixel.0[0] < 255),
            "bilinear produced no blended pixel"
        );
    }

    #[test]
    fn the_sampling_mode_changes_nothing_when_no_resampling_happens() {
        let source = MapSource::with("pxart", solid(4, 4, RED));
        let render_with = |interpolation: LayerInterpolation| {
            let mut layer = pixel_layer("art", 4, 4);
            layer.transform.interpolation = interpolation;
            // A whole-pixel move is an exact copy under either mode.
            layer.transform.translate_x = 1.0;
            render(&document_with(vec![layer], 4, 4), &source)
        };
        assert_eq!(
            render_with(LayerInterpolation::Bilinear).as_raw(),
            render_with(LayerInterpolation::Nearest).as_raw()
        );
    }

    #[test]
    fn nearest_sampling_applies_to_the_layer_mask_as_well() {
        let source = MapSource::with("pxart", solid(2, 2, RED));
        let mut bitmap = MaskBitmap::empty(2, 2).unwrap();
        bitmap.set(0, 0, 255);
        bitmap.set(0, 1, 255);
        let render_with = |interpolation: LayerInterpolation| {
            let mut layer = pixel_layer("art", 2, 2);
            layer.mask = Some(LayerMask {
                snapshot: MaskSnapshot::encode(&bitmap),
                enabled: true,
                inverted: false,
            });
            layer.transform = LayerTransform {
                scale_x: 6.0,
                scale_y: 6.0,
                interpolation,
                ..LayerTransform::default()
            };
            render(&document_with(vec![layer], 12, 12), &source)
        };
        let hard = render_with(LayerInterpolation::Nearest);
        assert!(
            hard.pixels()
                .all(|pixel| pixel.0[3] == 0 || pixel.0[3] == 255),
            "nearest left a partly covered pixel"
        );
        assert!(render_with(LayerInterpolation::Bilinear)
            .pixels()
            .any(|pixel| pixel.0[3] > 0 && pixel.0[3] < 255));
    }

    #[test]
    fn nearest_mask_samples_the_same_texel_as_nearest_artwork() {
        let mut bitmap = MaskBitmap::empty(2, 2).unwrap();
        bitmap.set(0, 0, 255);
        bitmap.set(1, 1, 128);
        let mut artwork = solid(2, 2, CLEAR);
        artwork.put_pixel(0, 0, Rgba([255, 255, 255, 255]));
        artwork.put_pixel(1, 1, Rgba([255, 255, 255, 128]));
        for x in [0.1, 0.5, 0.9, 1.0, 1.1, 1.5, 1.9] {
            for y in [0.1, 0.5, 0.9, 1.0, 1.1, 1.5, 1.9] {
                let alpha = sample_rgba(&artwork, x, y, LayerInterpolation::Nearest).unwrap()[3];
                for scale in [1, 2, 4] {
                    let coverage = mask_coverage(
                        Some(&bitmap),
                        false,
                        x * scale as f32,
                        y * scale as f32,
                        2 * scale,
                        2 * scale,
                        LayerInterpolation::Nearest,
                    );
                    let inverted = mask_coverage(
                        Some(&bitmap),
                        true,
                        x * scale as f32,
                        y * scale as f32,
                        2 * scale,
                        2 * scale,
                        LayerInterpolation::Nearest,
                    );
                    assert_eq!(
                        coverage, alpha,
                        "misaligned mask at ({x}, {y}), scale {scale}"
                    );
                    assert_eq!(inverted, 1.0 - alpha);
                }
            }
        }
    }

    // -- Transforms inside groups -----------------------------------------

    /// A child transform is applied inside the group, and the group opacity to
    /// the finished result, rather than the other way round.
    #[test]
    fn a_transformed_child_composites_inside_its_group() {
        let mut source = MapSource::default();
        source.insert("pxback", solid(8, 8, [0, 0, 0, 255]));
        source.insert("pxchild", solid(8, 8, RED));

        let mut child = pixel_layer("child", 8, 8);
        child.transform.translate_x = 4.0;
        let mut group = group_layer("g", vec![child]);
        group.opacity = 0.5;
        let document = document_with(vec![pixel_layer("back", 8, 8), group], 8, 8);
        let rendered = render(&document, &source);
        // Left of the moved child: untouched backdrop. Right: half-strength red.
        assert_eq!(rendered.get_pixel(0, 0).0, [0, 0, 0, 255]);
        assert!(rendered.get_pixel(6, 0).0[0].abs_diff(128) <= 1);
    }

    #[test]
    fn a_transformed_child_of_a_pass_through_group_matches_the_same_child_in_the_parent() {
        let mut source = MapSource::default();
        source.insert("pxback", solid(8, 8, [20, 30, 40, 255]));
        source.insert("pxchild", solid(8, 8, RED));

        let transformed = || {
            let mut layer = pixel_layer("child", 8, 8);
            layer.transform = LayerTransform {
                translate_x: 2.0,
                translate_y: -1.0,
                scale_x: 0.75,
                rotation_degrees: 15.0,
                ..LayerTransform::default()
            };
            layer
        };

        let grouped = document_with(
            vec![
                pixel_layer("back", 8, 8),
                pass_through_group("g", vec![transformed()]),
            ],
            8,
            8,
        );
        let direct = document_with(vec![pixel_layer("back", 8, 8), transformed()], 8, 8);
        assert_eq!(
            render(&grouped, &source).as_raw(),
            render(&direct, &source).as_raw()
        );
    }

    /// The invariant pass-through exists to satisfy, stated over a deliberately
    /// awkward tree so a future renderer change cannot quietly break it.
    #[test]
    fn an_open_pass_through_group_equals_its_children_placed_directly() {
        let mut source = MapSource::default();
        source.insert("pxback", solid(8, 8, [90, 40, 10, 255]));
        source.insert("pxone", solid(8, 8, RED));
        source.insert("pxtwo", solid(8, 8, BLUE));
        source.insert("pxthree", solid(8, 8, CLEAR));
        source.insert("pxfour", solid(8, 8, [30, 200, 120, 255]));

        let children = || {
            let mut moved = pixel_layer("one", 8, 8);
            moved.transform.translate_y = 3.0;
            moved.opacity = 0.6;
            let mut blended = pixel_layer("two", 8, 8);
            blended.blend_mode = BlendMode::Multiply;
            let mut transparent = pixel_layer("three", 8, 8);
            transparent.opacity = 0.25;
            let mut hidden = pixel_layer("two", 8, 8);
            hidden.id = "hidden".into();
            hidden.visible = false;
            let mut isolated_inner = group_layer("inner", vec![pixel_layer("four", 8, 8)]);
            isolated_inner.opacity = 0.4;
            vec![
                adjustment_layer("adj", EditOperation::Brightness { amount: 0.3 }),
                moved,
                blended,
                transparent,
                hidden,
                isolated_inner,
            ]
        };

        let grouped = document_with(
            vec![
                pixel_layer("back", 8, 8),
                pass_through_group("g", children()),
            ],
            8,
            8,
        );
        let mut flat = vec![pixel_layer("back", 8, 8)];
        flat.extend(children());
        let direct = document_with(flat, 8, 8);
        assert_eq!(
            render(&grouped, &source).as_raw(),
            render(&direct, &source).as_raw(),
            "a fully open pass-through group changed the result"
        );
    }

    #[test]
    fn an_isolated_group_inside_a_pass_through_group_still_contains_its_adjustment() {
        let mut source = MapSource::default();
        source.insert("pxback", solid(4, 4, [10, 10, 10, 255]));

        let inner = group_layer(
            "inner",
            vec![adjustment_layer(
                "adj",
                EditOperation::Brightness { amount: 1.0 },
            )],
        );
        let document = document_with(
            vec![
                pixel_layer("back", 4, 4),
                pass_through_group("g", vec![inner]),
            ],
            4,
            4,
        );
        // The isolated group has no pixels of its own, so its adjustment reaches
        // nothing and the backdrop below the pass-through group is untouched.
        assert_eq!(
            render(&document, &source).get_pixel(0, 0).0,
            [10, 10, 10, 255]
        );
    }

    #[test]
    fn a_transformed_layer_dragged_off_canvas_contributes_nothing_and_does_not_error() {
        let source = MapSource::with("pxart", solid(4, 4, RED));
        for translate in [-1_000.0_f32, 1_000.0, 500_000.0] {
            let mut layer = pixel_layer("art", 4, 4);
            layer.transform.translate_x = translate;
            let rendered = render(&document_with(vec![layer], 4, 4), &source);
            assert!(
                rendered.pixels().all(|pixel| pixel.0 == CLEAR),
                "{translate}"
            );
        }
    }

    #[test]
    fn a_partly_off_canvas_transform_draws_only_the_visible_part() {
        let source = MapSource::with("pxart", solid(4, 4, RED));
        let mut layer = pixel_layer("art", 4, 4);
        layer.transform.translate_x = -2.0;
        let rendered = render(&document_with(vec![layer], 4, 4), &source);
        assert_eq!(rendered.get_pixel(0, 0).0, RED);
        assert_eq!(rendered.get_pixel(3, 0).0, CLEAR);
    }

    /// The defining difference between the two group models: an adjustment
    /// inside an isolated group cannot touch the backdrop beneath it, and the
    /// same adjustment inside a pass-through group can.
    #[test]
    fn a_pass_through_group_lets_its_adjustment_reach_the_backdrop() {
        let mut source = MapSource::default();
        source.insert("pxback", solid(2, 2, [200, 40, 40, 255]));

        let build = |group: Layer| document_with(vec![pixel_layer("back", 2, 2), group], 2, 2);
        let adjustment = || adjustment_layer("adj", EditOperation::Grayscale);

        let isolated = render(&build(group_layer("g", vec![adjustment()])), &source);
        let pass_through = render(&build(pass_through_group("g", vec![adjustment()])), &source);

        // Isolated: the group's own buffer is empty, so the backdrop shows through
        // untouched.
        assert_eq!(isolated.get_pixel(0, 0).0, [200, 40, 40, 255]);
        // Pass-through: the adjustment greyed the backdrop beneath the group.
        let mixed = pass_through.get_pixel(0, 0).0;
        assert_eq!(mixed[0], mixed[1]);
        assert_eq!(mixed[1], mixed[2]);
        assert_eq!(mixed[3], 255);
        assert_ne!(mixed[0], 200);
    }

    #[test]
    fn an_opaque_unmasked_pass_through_group_is_exactly_its_reworked_backdrop() {
        let mut source = MapSource::default();
        source.insert("pxback", solid(3, 3, [120, 180, 60, 255]));
        source.insert("pxtop", solid(3, 3, [10, 20, 30, 255]));

        // Placing the same layers inside a fully open pass-through group must
        // read identically to placing them in the root stack.
        let flat = document_with(
            vec![
                pixel_layer("back", 3, 3),
                adjustment_layer("adj", EditOperation::Brightness { amount: 0.2 }),
                pixel_layer("top", 3, 3),
            ],
            3,
            3,
        );
        let grouped = document_with(
            vec![
                pixel_layer("back", 3, 3),
                pass_through_group(
                    "g",
                    vec![
                        adjustment_layer("adj", EditOperation::Brightness { amount: 0.2 }),
                        pixel_layer("top", 3, 3),
                    ],
                ),
            ],
            3,
            3,
        );
        assert_eq!(
            render(&flat, &source).as_raw(),
            render(&grouped, &source).as_raw()
        );
    }

    #[test]
    fn pass_through_group_opacity_fades_between_the_two_backdrops() {
        let mut source = MapSource::default();
        source.insert("pxback", solid(2, 2, [0, 0, 0, 255]));

        let mut group = pass_through_group(
            "g",
            vec![adjustment_layer(
                "adj",
                EditOperation::Brightness { amount: 1.0 },
            )],
        );
        group.opacity = 0.5;
        let document = document_with(vec![pixel_layer("back", 2, 2), group], 2, 2);
        let pixel = render(&document, &source).get_pixel(0, 0).0;
        // Full strength would take black to white; half opacity lands midway.
        assert!(pixel[0].abs_diff(128) <= 2, "{pixel:?}");
        assert_eq!(pixel[3], 255);
    }

    #[test]
    fn a_pass_through_group_mask_limits_where_the_rework_applies() {
        let mut source = MapSource::default();
        source.insert("pxback", solid(2, 1, [0, 0, 0, 255]));

        let mut mask = MaskBitmap::empty(2, 1).unwrap();
        mask.set(0, 0, 255);
        let mut group = pass_through_group(
            "g",
            vec![adjustment_layer(
                "adj",
                EditOperation::Brightness { amount: 1.0 },
            )],
        );
        group.mask = Some(LayerMask {
            snapshot: MaskSnapshot::encode(&mask),
            enabled: true,
            inverted: false,
        });
        let document = document_with(vec![pixel_layer("back", 2, 1), group], 2, 1);
        let rendered = render(&document, &source);
        assert_eq!(rendered.get_pixel(0, 0).0, [255, 255, 255, 255]);
        assert_eq!(rendered.get_pixel(1, 0).0, [0, 0, 0, 255]);
    }

    #[test]
    fn pass_through_preserves_alpha_where_the_backdrop_is_transparent() {
        let mut source = MapSource::default();
        source.insert("pxback", solid(2, 2, CLEAR));
        let group = pass_through_group(
            "g",
            vec![adjustment_layer(
                "adj",
                EditOperation::Brightness { amount: 0.5 },
            )],
        );
        let document = document_with(vec![pixel_layer("back", 2, 2), group], 2, 2);
        for pixel in render(&document, &source).pixels() {
            assert_eq!(pixel.0[3], 0, "pass-through must not create coverage");
        }
    }

    #[test]
    fn a_pass_through_group_must_use_the_normal_blend_mode() {
        let mut group = pass_through_group("g", vec![pixel_layer("child", 2, 2)]);
        group.blend_mode = BlendMode::Multiply;
        let document = document_with(vec![group], 2, 2);
        assert!(matches!(
            render_document(&document, &MapSource::default(), RenderOptions::default()),
            Err(AppError::InvalidLayerDocument(_))
        ));
    }

    #[test]
    fn a_group_without_the_isolated_field_restores_as_isolated() {
        // Every project written before pass-through existed omits the field, so
        // the default has to keep those documents looking the same.
        let json = r#"{"type":"group","children":[]}"#;
        let content: LayerContent = serde_json::from_str(json).unwrap();
        assert!(matches!(
            content,
            LayerContent::Group { isolated: true, .. }
        ));
    }

    #[test]
    fn nested_pass_through_groups_compose_through_both_levels() {
        let mut source = MapSource::default();
        source.insert("pxback", solid(2, 2, [40, 40, 40, 255]));
        let inner = pass_through_group(
            "inner",
            vec![adjustment_layer(
                "adj",
                EditOperation::Brightness { amount: 0.2 },
            )],
        );
        let outer = pass_through_group("outer", vec![inner]);
        let document = document_with(vec![pixel_layer("back", 2, 2), outer], 2, 2);
        // 0.2 * 255 = 51 reaches the backdrop through two pass-through levels.
        assert_eq!(
            render(&document, &source).get_pixel(0, 0).0,
            [91, 91, 91, 255]
        );
    }

    #[test]
    fn metadata_does_not_influence_the_rendered_result() {
        let mut layer = pixel_layer("only", 2, 2);
        layer.metadata = LayerMetadata {
            created_at: "2026-08-24T00:00:00Z".into(),
            modified_at: "2026-08-24T00:00:00Z".into(),
            custom: Default::default(),
        };
        let plain = document_with(vec![pixel_layer("only", 2, 2)], 2, 2);
        let annotated = document_with(vec![layer], 2, 2);
        let source = MapSource::with("pxonly", solid(2, 2, RED));
        assert_eq!(
            render(&plain, &source).as_raw(),
            render(&annotated, &source).as_raw()
        );
    }
}
