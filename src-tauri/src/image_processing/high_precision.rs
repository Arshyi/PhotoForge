//! Float operations for linear documents. No operation visits RGBA8.
use crate::color::{
    apply_development, srgb_decode, srgb_encode, DevelopmentParameters, FloatImage, FloatRgba,
    WhiteBalance,
};
use crate::domain::{CurvePoint, EditOperation};
use crate::error::AppError;
use std::sync::atomic::{AtomicBool, Ordering};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OperationLocality {
    TileLocal,
    HaloDependent,
    Global,
}

pub fn locality(operation: &EditOperation) -> OperationLocality {
    match operation {
        EditOperation::RawDevelopment { parameters }
            if matches!(parameters.white_balance, WhiteBalance::Auto) =>
        {
            OperationLocality::Global
        }
        EditOperation::AutoWhiteBalance { .. } => OperationLocality::Global,
        EditOperation::GaussianBlur { .. }
        | EditOperation::Sharpen { .. }
        | EditOperation::Denoise { .. }
        | EditOperation::LocalContrast { .. }
        | EditOperation::EdgeAwareSharpen { .. }
        | EditOperation::MildDeblur { .. }
        | EditOperation::UnevenLightingCorrection { .. }
        | EditOperation::Deblock { .. }
        | EditOperation::DecontaminateColors { .. } => OperationLocality::HaloDependent,
        EditOperation::Masked { operation, .. } => locality(operation),
        _ => OperationLocality::TileLocal,
    }
}

pub fn check_cancel(cancel: Option<&AtomicBool>) -> Result<(), AppError> {
    if cancel.is_some_and(|flag| flag.load(Ordering::Acquire)) {
        Err(AppError::RenderCancelled)
    } else {
        Ok(())
    }
}

/// Additional full-frame equivalents held by an operation, excluding input.
pub fn scratch_frames(operation: &EditOperation) -> u64 {
    match operation {
        EditOperation::Masked { operation, .. } => scratch_frames(operation).max(2),
        EditOperation::GaussianBlur { .. }
        | EditOperation::AutoWhiteBalance { .. }
        | EditOperation::LocalContrast { .. }
        | EditOperation::UnevenLightingCorrection { .. } => 2,
        EditOperation::Sharpen { .. }
        | EditOperation::EdgeAwareSharpen { .. }
        | EditOperation::MildDeblur { .. } => 3,
        _ => 1,
    }
}

pub fn pipeline(
    mut image: FloatImage,
    operations: &[EditOperation],
    cancel: Option<&AtomicBool>,
) -> Result<FloatImage, AppError> {
    for operation in operations {
        check_cancel(cancel)?;
        image = apply(&image, operation, cancel)?;
    }
    image.validate()?;
    Ok(image)
}

pub fn pipeline_typed(
    source: crate::pixel::PixelBuffer,
    operations: &[EditOperation],
    cancel: Option<&AtomicBool>,
) -> Result<crate::pixel::PixelBuffer, AppError> {
    check_cancel(cancel)?;
    if operations.is_empty() {
        return Ok(source);
    }
    if matches!(source, crate::pixel::PixelBuffer::LinearRgbaF32(_)) {
        let (w, h) = source.dimensions();
        crate::resources::ResourceEstimate::pipeline(w, h, operations, source.bytes())?;
    }
    match source {
        crate::pixel::PixelBuffer::LinearRgbaF32(image) => {
            // First operation reads the shared source directly. Cloning it into
            // an owned frame first would add an unneeded gigabyte at 61 MP.
            let first = apply(&image, &operations[0], cancel)?;
            drop(image);
            Ok(pipeline(first, &operations[1..], cancel)?.into())
        }
        crate::pixel::PixelBuffer::EncodedSrgba8(image) => {
            let owned =
                std::sync::Arc::try_unwrap(image).unwrap_or_else(|shared| (*shared).clone());
            Ok(
                super::apply_pipeline(&image::DynamicImage::ImageRgba8(owned), operations)?
                    .to_rgba8()
                    .into(),
            )
        }
    }
}

fn rgb(p: FloatRgba) -> [f32; 3] {
    [p.red, p.green, p.blue]
}
fn with_rgb(c: [f32; 3], alpha: f32) -> FloatRgba {
    FloatRgba::new(c[0], c[1], c[2], alpha)
}
fn mix(a: FloatRgba, b: FloatRgba, t: f32) -> FloatRgba {
    with_rgb(
        std::array::from_fn(|c| rgb(a)[c] + t * (rgb(b)[c] - rgb(a)[c])),
        a.alpha,
    )
}

thread_local! {
    /// Set while this thread is already a worker in an outer parallel render.
    static NESTED: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
}

/// Runs `body` with this operation's own row parallelism suppressed.
///
/// The tiled renderer already runs one worker per core. Without this, a tile
/// big enough to cross the row-splitting threshold would fan out again inside
/// each worker, so eight workers would become sixty-four threads on an
/// eight-core budget. Relying on tiles staying under a pixel count instead
/// would make thread bounds depend on the tile size a caller happened to pick.
pub fn without_nested_parallelism<R>(body: impl FnOnce() -> R) -> R {
    let previous = NESTED.with(|nested| nested.replace(true));
    let result = body();
    NESTED.with(|nested| nested.set(previous));
    result
}

/// Whether this thread is already inside an outer parallel render.
fn nested() -> bool {
    NESTED.with(std::cell::Cell::get)
}

fn mapped(
    image: &FloatImage,
    cancel: Option<&AtomicBool>,
    f: impl Fn(FloatRgba) -> FloatRgba + Sync,
) -> Result<FloatImage, AppError> {
    let mut result = image.clone();
    let width = image.width() as usize;
    let workers = if nested() || image.pixels().len() < 262_144 {
        1
    } else {
        std::thread::available_parallelism()
            .map_or(1, usize::from)
            .min(8)
    };
    let rows_per_worker = (image.height() as usize).div_ceil(workers);
    std::thread::scope(|scope| -> Result<(), AppError> {
        let mut jobs = Vec::with_capacity(workers);
        for band in result.pixels_mut().chunks_mut(width * rows_per_worker) {
            let f = &f;
            jobs.push(scope.spawn(move || -> Result<(), AppError> {
                for row in band.chunks_mut(width) {
                    check_cancel(cancel)?;
                    for pixel in row {
                        *pixel = f(*pixel);
                    }
                }
                Ok(())
            }));
        }
        for job in jobs {
            job.join().map_err(|_| {
                AppError::ProcessingFailure("float operation worker stopped".into())
            })??;
        }
        Ok(())
    })?;
    result.validate()?;
    Ok(result)
}

fn encoded(
    image: &FloatImage,
    cancel: Option<&AtomicBool>,
    f: impl Fn([f32; 3]) -> [f32; 3] + Sync,
) -> Result<FloatImage, AppError> {
    mapped(image, cancel, |p| {
        with_rgb(f(rgb(p).map(srgb_encode)).map(srgb_decode), p.alpha)
    })
}

fn curve(points: &[CurvePoint], value: f32) -> f32 {
    let pair = points
        .windows(2)
        .find(|p| value <= p[1].input)
        .unwrap_or(&points[points.len() - 2..]);
    let t = (value - pair[0].input) / (pair[1].input - pair[0].input).max(f32::EPSILON);
    pair[0].output + (pair[1].output - pair[0].output) * t
}

fn hsl(c: [f32; 3]) -> (f32, f32, f32) {
    let c = c.map(|v| v.clamp(0.0, 1.0));
    let hi = c[0].max(c[1]).max(c[2]);
    let lo = c[0].min(c[1]).min(c[2]);
    let light = (hi + lo) * 0.5;
    let delta = hi - lo;
    if delta <= f32::EPSILON {
        return (0.0, 0.0, light);
    }
    let sat = delta / (1.0 - (2.0 * light - 1.0).abs()).max(f32::EPSILON);
    let hue = if hi == c[0] {
        ((c[1] - c[2]) / delta).rem_euclid(6.0)
    } else if hi == c[1] {
        (c[2] - c[0]) / delta + 2.0
    } else {
        (c[0] - c[1]) / delta + 4.0
    };
    (hue * 60.0, sat, light)
}
fn from_hsl(h: f32, s: f32, l: f32) -> [f32; 3] {
    let c = (1.0 - (2.0 * l - 1.0).abs()) * s;
    let x = c * (1.0 - ((h / 60.0).rem_euclid(2.0) - 1.0).abs());
    let m = l - c * 0.5;
    let base = match (h / 60.0).floor() as i32 {
        0 => [c, x, 0.0],
        1 => [x, c, 0.0],
        2 => [0.0, c, x],
        3 => [0.0, x, c],
        4 => [x, 0.0, c],
        _ => [c, 0.0, x],
    };
    base.map(|v| v + m)
}

pub fn apply(
    image: &FloatImage,
    operation: &EditOperation,
    cancel: Option<&AtomicBool>,
) -> Result<FloatImage, AppError> {
    operation.validate()?;
    check_cancel(cancel)?;
    crate::resources::ResourceEstimate::pipeline(
        image.width(),
        image.height(),
        std::slice::from_ref(operation),
        u64::from(image.width()) * u64::from(image.height()) * 16,
    )?;
    use EditOperation::*;
    let output = match operation {
        RawDevelopment { parameters } => {
            let mut out = image.clone();
            apply_development(&mut out, parameters)?;
            out
        }
        Masked {
            operation,
            mask,
            invert,
            ..
        } => {
            let mask = mask.decode()?;
            if image.dimensions() != (mask.width(), mask.height()) {
                return Err(AppError::MaskDimensionMismatch {
                    mask_width: mask.width(),
                    mask_height: mask.height(),
                    image_width: image.width(),
                    image_height: image.height(),
                });
            }
            let adjusted = if let DecontaminateColors {
                enabled,
                strength,
                radius,
            } = operation.as_ref()
            {
                decontaminate(image, &mask, *invert, *enabled, *strength, *radius, cancel)?
            } else {
                apply(image, operation, cancel)?
            };
            let mut result = image.clone();
            for (i, p) in result.pixels_mut().iter_mut().enumerate() {
                if i % image.width() as usize == 0 {
                    check_cancel(cancel)?;
                }
                let coverage = f32::from(mask.coverage()[i]) / 255.0;
                *p = mix(
                    *p,
                    adjusted.pixels()[i],
                    if *invert { 1.0 - coverage } else { coverage },
                );
            }
            result
        }
        Brightness { amount } => encoded(image, cancel, |c| c.map(|v| v + amount))?,
        Contrast { amount } => encoded(image, cancel, |c| {
            c.map(|v| (v - 0.5) * (1.0 + amount) + 0.5)
        })?,
        Saturation { amount } => encoded(image, cancel, |c| {
            let y = c[0] * 0.2126 + c[1] * 0.7152 + c[2] * 0.0722;
            c.map(|v| y + (v - y) * (1.0 + amount))
        })?,
        Gamma { value } => encoded(image, cancel, |c| {
            c.map(|v| v.signum() * v.abs().powf(1.0 / value))
        })?,
        Grayscale => mapped(image, cancel, |p| with_rgb([p.luminance(); 3], p.alpha))?,
        Sepia => encoded(image, cancel, |c| {
            [
                c[0] * 0.393 + c[1] * 0.769 + c[2] * 0.189,
                c[0] * 0.349 + c[1] * 0.686 + c[2] * 0.168,
                c[0] * 0.272 + c[1] * 0.534 + c[2] * 0.131,
            ]
        })?,
        Curves { curves } => encoded(image, cancel, |c| {
            let channels = [&curves.red, &curves.green, &curves.blue];
            std::array::from_fn(|i| curve(channels[i], curve(&curves.rgb, c[i])))
        })?,
        Levels {
            input_black,
            input_white,
            gamma,
            output_black,
            output_white,
        } => encoded(image, cancel, |c| {
            let black = f32::from(*input_black) / 255.0;
            let span = f32::from(input_white - input_black) / 255.0;
            c.map(|v| {
                f32::from(*output_black) / 255.0
                    + ((v - black) / span).clamp(0.0, 1.0).powf(1.0 / gamma)
                        * f32::from(output_white - output_black)
                        / 255.0
            })
        })?,
        WhitePoint { red, green, blue } => encoded(image, cancel, |c| {
            let point = [*red, *green, *blue];
            std::array::from_fn(|i| c[i] * 255.0 / f32::from(point[i].max(1)))
        })?,
        BlackPoint { red, green, blue } => encoded(image, cancel, |c| {
            let point = [*red, *green, *blue];
            std::array::from_fn(|i| c[i] - f32::from(point[i]) / 255.0)
        })?,
        TemperatureTint { temperature, tint } => mapped(image, cancel, |p| {
            with_rgb(
                [
                    p.red * (1.0 + temperature * 0.25 + tint * 0.05),
                    p.green * (1.0 - tint * 0.2),
                    p.blue * (1.0 - temperature * 0.25 + tint * 0.05),
                ],
                p.alpha,
            )
        })?,
        Hsl { settings } => encoded(image, cancel, |c| {
            let (h, s, l) = hsl(c);
            let local = super::professional::hue_adjustment(settings, h);
            from_hsl(
                (h + settings.master.hue + local.hue).rem_euclid(360.0),
                (s * (1.0 + settings.master.saturation + local.saturation)).clamp(0.0, 1.0),
                (l + (settings.master.lightness + local.lightness) * 0.5).clamp(0.0, 1.0),
            )
        })?,
        SelectiveColor {
            target_hue,
            width,
            adjustment,
        } => encoded(image, cancel, |c| {
            let (h, _, _) = hsl(c);
            let weight = (1.0 - super::professional::hue_distance(h, *target_hue) / width.max(1.0))
                .clamp(0.0, 1.0);
            [
                c[0] - (adjustment.cyan + adjustment.black) * weight * 0.5,
                c[1] - (adjustment.magenta + adjustment.black) * weight * 0.5,
                c[2] - (adjustment.yellow + adjustment.black) * weight * 0.5,
            ]
        })?,
        AutoWhiteBalance { strength } => {
            let mut balanced = image.clone();
            apply_development(
                &mut balanced,
                &DevelopmentParameters {
                    white_balance: WhiteBalance::Auto,
                    ..DevelopmentParameters::default()
                },
            )?;
            let mut out = image.clone();
            for (p, b) in out.pixels_mut().iter_mut().zip(balanced.pixels()) {
                *p = mix(*p, *b, *strength);
            }
            out
        }
        GaussianBlur { radius } => gaussian(image, *radius, cancel)?,
        Sharpen { strength } => sharpen(image, *strength, 1.2, 0.0, cancel)?,
        EdgeAwareSharpen {
            strength,
            radius,
            threshold,
        } => sharpen(image, *strength, *radius, *threshold, cancel)?,
        MildDeblur { strength, radius } => sharpen(image, *strength * 1.35, *radius, 2.0, cancel)?,
        Denoise {
            strength,
            preserve_edges,
        } => denoise(image, *strength, *preserve_edges, cancel)?,
        LocalContrast {
            strength,
            tile_size,
            clip_limit,
        } => local_luma(image, (*tile_size / 2).max(1), cancel, |value, local| {
            value
                + (value - local).clamp(-12.0 * clip_limit / 255.0, 12.0 * clip_limit / 255.0)
                    * strength
        })?,
        UnevenLightingCorrection { strength, radius } => {
            local_luma(image, (*radius as u32).max(1), cancel, |value, local| {
                value + (0.5 - local) * strength
            })?
        }
        DocumentEnhance {
            strength,
            grayscale,
        } => mapped(image, cancel, |p| {
            let y = p.luminance();
            let target = ((y - 0.5) * (1.0 + 0.75 * strength) + 0.5).clamp(0.0, 1.0);
            if *grayscale {
                with_rgb([target; 3], p.alpha)
            } else {
                with_rgb(rgb(p).map(|v| v + target - y), p.alpha)
            }
        })?,
        Deblock { strength } => deblock(image, *strength, cancel)?,
        ReflectHorizontal
        | Rotate { .. }
        | Crop { .. }
        | Straighten { .. }
        | Perspective { .. }
        | LensCorrection { .. } => geometry(image, operation, cancel)?,
        DecontaminateColors { .. } => {
            return Err(AppError::InvalidOperation(
                "decontaminate_colors requires a selection mask".into(),
            ))
        }
    };
    check_cancel(cancel)?;
    output.validate()?;
    Ok(output)
}

fn gaussian(
    image: &FloatImage,
    sigma: f32,
    cancel: Option<&AtomicBool>,
) -> Result<FloatImage, AppError> {
    if sigma <= 0.0 {
        return Ok(image.clone());
    }
    let radius = (sigma * 3.0).ceil() as i64;
    let mut kernel: Vec<f32> = (-radius..=radius)
        .map(|i| (-(i * i) as f32 / (2.0 * sigma * sigma)).exp())
        .collect();
    let sum: f32 = kernel.iter().sum();
    for weight in &mut kernel {
        *weight /= sum;
    }
    let mut current = image.clone();
    for horizontal in [true, false] {
        let mut out = FloatImage::blank(image.width(), image.height(), FloatRgba::TRANSPARENT)?;
        for y in 0..image.height() {
            check_cancel(cancel)?;
            for x in 0..image.width() {
                let mut sums = [0.0_f32; 4];
                for (k, w) in kernel.iter().enumerate() {
                    let offset = k as i64 - radius;
                    let sx = (i64::from(x) + if horizontal { offset } else { 0 })
                        .clamp(0, i64::from(image.width()) - 1) as u32;
                    let sy = (i64::from(y) + if horizontal { 0 } else { offset })
                        .clamp(0, i64::from(image.height()) - 1)
                        as u32;
                    let p = current.get(sx, sy).unwrap();
                    let weight = w * p.alpha;
                    sums[0] += p.red * weight;
                    sums[1] += p.green * weight;
                    sums[2] += p.blue * weight;
                    sums[3] += weight;
                }
                out.pixels_mut()[(y * image.width() + x) as usize] = if sums[3] > 0.0 {
                    FloatRgba::new(
                        sums[0] / sums[3],
                        sums[1] / sums[3],
                        sums[2] / sums[3],
                        sums[3].clamp(0.0, 1.0),
                    )
                } else {
                    FloatRgba::TRANSPARENT
                };
            }
        }
        current = out;
    }
    Ok(current)
}

fn sharpen(
    image: &FloatImage,
    strength: f32,
    radius: f32,
    threshold: f32,
    cancel: Option<&AtomicBool>,
) -> Result<FloatImage, AppError> {
    if strength == 0.0 {
        return Ok(image.clone());
    }
    let blurred = gaussian(image, radius, cancel)?;
    let mut out = image.clone();
    for (i, (p, b)) in out
        .pixels_mut()
        .iter_mut()
        .zip(blurred.pixels())
        .enumerate()
    {
        if i % image.width() as usize == 0 {
            check_cancel(cancel)?;
        }
        let original = *p;
        if (p.luminance() - b.luminance()).abs() >= threshold / 255.0 {
            *p = with_rgb(
                std::array::from_fn(|c| {
                    rgb(original)[c] + (rgb(original)[c] - rgb(*b)[c]) * strength
                }),
                p.alpha,
            );
        }
    }
    Ok(out)
}

fn denoise(
    image: &FloatImage,
    strength: f32,
    preserve: f32,
    cancel: Option<&AtomicBool>,
) -> Result<FloatImage, AppError> {
    if strength == 0.0 {
        return Ok(image.clone());
    }
    let mut out = image.clone();
    let radius = if strength > 0.65 { 2_i64 } else { 1 };
    let range = (8.0 + (1.0 - preserve) * 64.0) / 255.0;
    for y in 0..image.height() {
        check_cancel(cancel)?;
        for x in 0..image.width() {
            let center = image.get(x, y).unwrap();
            if center.alpha == 0.0 {
                continue;
            }
            let mut sum = [0.0_f32; 3];
            let mut weights = 0.0;
            for dy in -radius..=radius {
                for dx in -radius..=radius {
                    let sx = (i64::from(x) + dx).clamp(0, i64::from(image.width()) - 1) as u32;
                    let sy = (i64::from(y) + dy).clamp(0, i64::from(image.height()) - 1) as u32;
                    let p = image.get(sx, sy).unwrap();
                    let distance = (0..3)
                        .map(|c| (rgb(center)[c] - rgb(p)[c]).abs())
                        .sum::<f32>()
                        / 3.0;
                    let w = p.alpha / (1.0 + (dx * dx + dy * dy) as f32) / (1.0 + distance / range);
                    for (c, value) in sum.iter_mut().enumerate() {
                        *value += rgb(p)[c] * w;
                    }
                    weights += w;
                }
            }
            if weights > 0.0 {
                out.pixels_mut()[(y * image.width() + x) as usize] = mix(
                    center,
                    with_rgb(sum.map(|v| v / weights), center.alpha),
                    strength,
                );
            }
        }
    }
    Ok(out)
}

/// Sliding sums make large-radius local controls O(pixels), not O(radius*pixels).
fn local_luma(
    image: &FloatImage,
    radius: u32,
    cancel: Option<&AtomicBool>,
    f: impl Fn(f32, f32) -> f32,
) -> Result<FloatImage, AppError> {
    let w = image.width() as usize;
    let h = image.height() as usize;
    let r = radius as usize;
    let mut values: Vec<f32> = image
        .pixels()
        .iter()
        .map(|p| p.luminance() * p.alpha)
        .collect();
    let mut weights: Vec<f32> = image.pixels().iter().map(|p| p.alpha).collect();
    let mut scratch = vec![0.0_f32; values.len()];
    let mut weight_scratch = vec![0.0_f32; values.len()];
    for horizontal in [true, false] {
        let lines = if horizontal { h } else { w };
        let len = if horizontal { w } else { h };
        for line in 0..lines {
            check_cancel(cancel)?;
            let at = |i: usize| {
                if horizontal {
                    line * w + i
                } else {
                    i * w + line
                }
            };
            let mut sum = 0.0_f64;
            let mut weight_sum = 0.0_f64;
            let mut left = 0;
            let mut right = 0;
            for i in 0..len {
                let end = (i + r + 1).min(len);
                while right < end {
                    sum += f64::from(values[at(right)]);
                    weight_sum += f64::from(weights[at(right)]);
                    right += 1;
                }
                let start = i.saturating_sub(r);
                while left < start {
                    sum -= f64::from(values[at(left)]);
                    weight_sum -= f64::from(weights[at(left)]);
                    left += 1;
                }
                scratch[at(i)] = (sum / (right - left) as f64) as f32;
                weight_scratch[at(i)] = (weight_sum / (right - left) as f64) as f32;
            }
        }
        std::mem::swap(&mut values, &mut scratch);
        std::mem::swap(&mut weights, &mut weight_scratch);
    }
    let mut out = image.clone();
    for ((p, local), weight) in out.pixels_mut().iter_mut().zip(values).zip(weights) {
        if weight <= 0.0 || p.alpha <= 0.0 {
            continue;
        }
        let delta = f(p.luminance(), local / weight) - p.luminance();
        p.red += delta;
        p.green += delta;
        p.blue += delta;
    }
    Ok(out)
}

fn deblock(
    image: &FloatImage,
    strength: f32,
    cancel: Option<&AtomicBool>,
) -> Result<FloatImage, AppError> {
    let mut out = image.clone();
    for vertical in [true, false] {
        let end = if vertical {
            image.width()
        } else {
            image.height()
        };
        let rows = if vertical {
            image.height()
        } else {
            image.width()
        };
        for edge in (8..end).step_by(8) {
            check_cancel(cancel)?;
            for row in 0..rows {
                let (a, b) = if vertical {
                    ((edge - 1, row), (edge, row))
                } else {
                    ((row, edge - 1), (row, edge))
                };
                let p = image.get(a.0, a.1).unwrap();
                let q = image.get(b.0, b.1).unwrap();
                if p.alpha == 0.0
                    || q.alpha == 0.0
                    || !(4.0 / 255.0..=80.0 / 255.0)
                        .contains(&(p.luminance() - q.luminance()).abs())
                {
                    continue;
                }
                let avg = with_rgb(std::array::from_fn(|c| (rgb(p)[c] + rgb(q)[c]) * 0.5), 1.0);
                out.pixels_mut()[(a.1 * image.width() + a.0) as usize] =
                    mix(p, avg, strength * 0.38);
                out.pixels_mut()[(b.1 * image.width() + b.0) as usize] =
                    mix(q, avg, strength * 0.38);
            }
        }
    }
    Ok(out)
}

fn decontaminate(
    image: &FloatImage,
    mask: &crate::mask::MaskBitmap,
    invert: bool,
    enabled: bool,
    strength: f32,
    radius: u32,
    cancel: Option<&AtomicBool>,
) -> Result<FloatImage, AppError> {
    if !enabled || strength == 0.0 {
        return Ok(image.clone());
    }
    let mut out = image.clone();
    let coverage = |x: u32, y: u32| {
        let c = mask.get(x, y);
        if invert {
            255 - c
        } else {
            c
        }
    };
    for y in 0..image.height() {
        check_cancel(cancel)?;
        for x in 0..image.width() {
            let c = coverage(x, y);
            if c == 0 || c == 255 {
                continue;
            }
            let mut sum = [0.0_f32; 3];
            let mut weights = 0.0;
            for sy in y.saturating_sub(radius)..=(y + radius).min(image.height() - 1) {
                for sx in x.saturating_sub(radius)..=(x + radius).min(image.width() - 1) {
                    if coverage(sx, sy) < 250 {
                        continue;
                    }
                    let p = image.get(sx, sy).unwrap();
                    let d = (sx as f32 - x as f32).powi(2) + (sy as f32 - y as f32).powi(2);
                    let weight = p.alpha / (1.0 + d);
                    for (i, value) in sum.iter_mut().enumerate() {
                        *value += rgb(p)[i] * weight;
                    }
                    weights += weight;
                }
            }
            if weights > 0.0 {
                let i = (y * image.width() + x) as usize;
                out.pixels_mut()[i] = mix(
                    image.pixels()[i],
                    with_rgb(sum.map(|v| v / weights), 1.0),
                    strength,
                );
            }
        }
    }
    Ok(out)
}

fn geometry(
    image: &FloatImage,
    op: &EditOperation,
    cancel: Option<&AtomicBool>,
) -> Result<FloatImage, AppError> {
    let (w, h) = image.dimensions();
    let mut dimensions = (w, h);
    let mut crop = (0, 0);
    match op {
        EditOperation::Rotate { degrees } if degrees.rem_euclid(180) != 0 => dimensions = (h, w),
        EditOperation::Crop {
            x,
            y,
            width,
            height,
            ..
        } => {
            crop = ((x * w as f32).floor() as u32, (y * h as f32).floor() as u32);
            dimensions = (
                ((width * w as f32).round() as u32).max(1).min(w - crop.0),
                ((height * h as f32).round() as u32).max(1).min(h - crop.1),
            );
        }
        _ => {}
    }
    let mut out = FloatImage::blank(dimensions.0, dimensions.1, FloatRgba::TRANSPARENT)?;
    for y in 0..dimensions.1 {
        check_cancel(cancel)?;
        for x in 0..dimensions.0 {
            let mut light = 1.0;
            let mut shift = (0.0, 0.0);
            let (sx, sy) = match op {
                EditOperation::ReflectHorizontal => (w as f32 - x as f32 - 1.0, y as f32),
                EditOperation::Rotate { degrees } => match degrees.rem_euclid(360) {
                    90 => (y as f32, h as f32 - x as f32 - 1.0),
                    180 => (w as f32 - x as f32 - 1.0, h as f32 - y as f32 - 1.0),
                    270 => (w as f32 - y as f32 - 1.0, x as f32),
                    _ => (x as f32, y as f32),
                },
                EditOperation::Crop { .. } => ((x + crop.0) as f32, (y + crop.1) as f32),
                EditOperation::Straighten { degrees } => {
                    let (sin, cos) = degrees.to_radians().sin_cos();
                    let cx = (w as f32 - 1.0) * 0.5;
                    let cy = (h as f32 - 1.0) * 0.5;
                    let dx = x as f32 - cx;
                    let dy = y as f32 - cy;
                    (cx + dx * cos + dy * sin, cy - dx * sin + dy * cos)
                }
                EditOperation::Perspective { corners } => {
                    let u = x as f32 / (w - 1).max(1) as f32;
                    let v = y as f32 / (h - 1).max(1) as f32;
                    let p = |i: usize| {
                        let top =
                            corners.top_left[i] + u * (corners.top_right[i] - corners.top_left[i]);
                        let bottom = corners.bottom_left[i]
                            + u * (corners.bottom_right[i] - corners.bottom_left[i]);
                        top + v * (bottom - top)
                    };
                    (p(0) * (w - 1) as f32, p(1) * (h - 1) as f32)
                }
                EditOperation::LensCorrection {
                    distortion,
                    vignetting,
                    chromatic_aberration,
                } => {
                    let cx = (w as f32 - 1.0) * 0.5;
                    let cy = (h as f32 - 1.0) * 0.5;
                    let nx = (x as f32 - cx) / cx.max(1.0);
                    let ny = (y as f32 - cy) / cy.max(1.0);
                    let r = nx * nx + ny * ny;
                    shift = (
                        nx * chromatic_aberration * r * 2.0,
                        ny * chromatic_aberration * r * 2.0,
                    );
                    light = (1.0 + vignetting * r).max(0.0);
                    (
                        cx + nx * (1.0 + distortion * r) * cx.max(1.0),
                        cy + ny * (1.0 + distortion * r) * cy.max(1.0),
                    )
                }
                _ => unreachable!("geometry dispatch is exhaustive"),
            };
            let mut p = image.sample(sx + 0.5, sy + 0.5, false);
            if shift != (0.0, 0.0) {
                p.red = image
                    .sample(sx + shift.0 + 0.5, sy + shift.1 + 0.5, false)
                    .red;
                p.blue = image
                    .sample(sx - shift.0 + 0.5, sy - shift.1 + 0.5, false)
                    .blue;
            }
            p.red *= light;
            p.green *= light;
            p.blue *= light;
            out.pixels_mut()[(y * dimensions.0 + x) as usize] = p;
        }
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashSet;
    use std::sync::Mutex;
    use std::thread::ThreadId;

    /// Counts the distinct threads an operation actually runs on.
    fn threads_used(nest: bool) -> usize {
        // Comfortably over the row-splitting threshold, so the unnested case
        // has a real reason to fan out.
        let image = FloatImage::blank(600, 600, FloatRgba::TRANSPARENT).unwrap();
        let seen: Mutex<HashSet<ThreadId>> = Mutex::new(HashSet::new());
        let record = |pixel: FloatRgba| {
            seen.lock()
                .unwrap_or_else(|error| error.into_inner())
                .insert(std::thread::current().id());
            pixel
        };
        let run = || mapped(&image, None, record).unwrap();
        if nest {
            without_nested_parallelism(run);
        } else {
            run();
        }
        let count = seen.lock().unwrap_or_else(|e| e.into_inner()).len();
        count
    }

    /// The bound that stops eight tiled workers becoming sixty-four threads.
    #[test]
    fn nested_parallelism_is_suppressed_inside_an_outer_worker() {
        assert_eq!(
            threads_used(true),
            1,
            "an operation fanned out while already inside a parallel render"
        );
        // Only meaningful where the machine actually has cores to use; on a
        // single-core runner the unnested case is legitimately 1 as well.
        if std::thread::available_parallelism().map_or(1, usize::from) > 1 {
            assert!(
                threads_used(false) > 1,
                "the guard suppressed parallelism outside a parallel render too"
            );
        }
    }

    /// The flag must not leak into later work on the same thread, or one tiled
    /// render would silently make every subsequent render single-threaded.
    #[test]
    fn the_nesting_flag_is_restored_afterwards() {
        assert!(!nested());
        without_nested_parallelism(|| {
            assert!(nested());
            without_nested_parallelism(|| assert!(nested()));
            assert!(nested(), "the inner guard cleared the outer one");
        });
        assert!(!nested(), "the guard leaked past its scope");
    }
}
