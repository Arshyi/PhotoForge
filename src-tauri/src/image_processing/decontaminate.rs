use crate::error::AppError;
use crate::mask::MaskBitmap;
use image::{Rgba, RgbaImage};

/// Coverage at or above this value is treated as confidently selected foreground.
const CONFIDENT_FOREGROUND_COVERAGE: u8 = 224;
/// Bounds the worst-case neighborhood work independently of image and mask shape.
const MAX_NEIGHBORHOOD_PIXEL_VISITS: u64 = 128_000_000;
const STRENGTH_SCALE: u64 = u16::MAX as u64;

pub(crate) fn apply(
    source: &RgbaImage,
    mask: &MaskBitmap,
    invert: bool,
    enabled: bool,
    strength: f32,
    radius: u32,
) -> Result<RgbaImage, AppError> {
    if source.dimensions() != (mask.width(), mask.height()) {
        return Err(AppError::MaskDimensionMismatch {
            mask_width: mask.width(),
            mask_height: mask.height(),
            image_width: source.width(),
            image_height: source.height(),
        });
    }
    if !enabled || strength == 0.0 {
        return Ok(source.clone());
    }

    let edge_pixels = mask
        .coverage()
        .iter()
        .filter(|coverage| {
            let effective = effective_coverage(**coverage, invert);
            effective > 0 && effective < CONFIDENT_FOREGROUND_COVERAGE
        })
        .count() as u64;
    let diameter = u64::from(radius)
        .checked_mul(2)
        .and_then(|value| value.checked_add(1))
        .ok_or(AppError::RestorationResourceLimit)?;
    let conservative_visits = edge_pixels
        .checked_mul(diameter)
        .and_then(|value| value.checked_mul(diameter))
        .ok_or(AppError::RestorationResourceLimit)?;
    if conservative_visits > MAX_NEIGHBORHOOD_PIXEL_VISITS {
        return Err(AppError::RestorationResourceLimit);
    }

    let strength_weight = (strength * STRENGTH_SCALE as f32).round() as u64;
    let radius_squared = u64::from(radius) * u64::from(radius);
    let mut output = source.clone();

    for y in 0..source.height() {
        for x in 0..source.width() {
            let coverage = effective_coverage(mask.get(x, y), invert);
            if coverage == 0 || coverage >= CONFIDENT_FOREGROUND_COVERAGE {
                continue;
            }

            let x_min = x.saturating_sub(radius);
            let y_min = y.saturating_sub(radius);
            let x_max = x.saturating_add(radius).min(source.width() - 1);
            let y_max = y.saturating_add(radius).min(source.height() - 1);
            let mut channel_sums = [0_u64; 3];
            let mut total_weight = 0_u64;

            for sample_y in y_min..=y_max {
                for sample_x in x_min..=x_max {
                    let dx = sample_x.abs_diff(x);
                    let dy = sample_y.abs_diff(y);
                    let distance_squared =
                        u64::from(dx) * u64::from(dx) + u64::from(dy) * u64::from(dy);
                    if distance_squared > radius_squared {
                        continue;
                    }

                    let sample_coverage = effective_coverage(mask.get(sample_x, sample_y), invert);
                    let sample = source.get_pixel(sample_x, sample_y);
                    if sample_coverage < CONFIDENT_FOREGROUND_COVERAGE || sample[3] == 0 {
                        continue;
                    }

                    let distance_weight = radius_squared + 1 - distance_squared;
                    let weight = u64::from(sample_coverage) * distance_weight;
                    for channel in 0..3 {
                        channel_sums[channel] += u64::from(sample[channel]) * weight;
                    }
                    total_weight += weight;
                }
            }

            if total_weight == 0 {
                continue;
            }

            let original = source.get_pixel(x, y);
            let mut changed = original.0;
            for channel in 0..3 {
                let foreground = (channel_sums[channel] + total_weight / 2) / total_weight;
                changed[channel] =
                    blend_channel(u64::from(original[channel]), foreground, strength_weight);
            }
            output.put_pixel(x, y, Rgba(changed));
        }
    }

    Ok(output)
}

fn effective_coverage(coverage: u8, invert: bool) -> u8 {
    if invert {
        u8::MAX - coverage
    } else {
        coverage
    }
}

fn blend_channel(original: u64, foreground: u64, strength: u64) -> u8 {
    ((original * (STRENGTH_SCALE - strength) + foreground * strength + STRENGTH_SCALE / 2)
        / STRENGTH_SCALE) as u8
}

#[cfg(test)]
mod tests {
    use super::*;

    fn image(pixels: &[[u8; 4]]) -> RgbaImage {
        RgbaImage::from_fn(pixels.len() as u32, 1, |x, _| Rgba(pixels[x as usize]))
    }

    fn mask(coverage: Vec<u8>) -> MaskBitmap {
        MaskBitmap::from_coverage(coverage.len() as u32, 1, coverage).unwrap()
    }

    #[test]
    fn disabled_and_zero_strength_are_pixel_exact() {
        let source = image(&[[220, 20, 20, 17], [20, 220, 20, 29]]);
        let selection = mask(vec![255, 128]);
        assert_eq!(
            apply(&source, &selection, false, false, 1.0, 1).unwrap(),
            source
        );
        assert_eq!(
            apply(&source, &selection, false, true, 0.0, 1).unwrap(),
            source
        );
    }

    #[test]
    fn removes_edge_spill_using_only_confident_foreground() {
        let source = image(&[[220, 20, 20, 255], [20, 220, 20, 91], [20, 20, 220, 255]]);
        let selection = mask(vec![255, 128, 0]);
        let result = apply(&source, &selection, false, true, 1.0, 1).unwrap();
        assert_eq!(result.get_pixel(0, 0), source.get_pixel(0, 0));
        assert_eq!(result.get_pixel(1, 0).0, [220, 20, 20, 91]);
        assert_eq!(result.get_pixel(2, 0), source.get_pixel(2, 0));
    }

    #[test]
    fn preserves_alpha_and_is_deterministic() {
        let source = image(&[[180, 30, 10, 11], [10, 180, 30, 22], [10, 30, 180, 33]]);
        let selection = mask(vec![255, 128, 0]);
        let first = apply(&source, &selection, false, true, 0.63, 2).unwrap();
        let second = apply(&source, &selection, false, true, 0.63, 2).unwrap();
        assert_eq!(first, second);
        assert_eq!(
            first.pixels().map(|pixel| pixel[3]).collect::<Vec<_>>(),
            [11, 22, 33]
        );
    }

    #[test]
    fn inverted_selection_samples_the_other_side() {
        let source = image(&[[220, 20, 20, 255], [20, 220, 20, 255], [20, 20, 220, 255]]);
        let selection = mask(vec![255, 128, 0]);
        let inside = apply(&source, &selection, false, true, 1.0, 1).unwrap();
        let outside = apply(&source, &selection, true, true, 1.0, 1).unwrap();
        assert_eq!(inside.get_pixel(1, 0).0, [220, 20, 20, 255]);
        assert_eq!(outside.get_pixel(1, 0).0, [20, 20, 220, 255]);
    }

    #[test]
    fn leaves_edges_without_confident_foreground_unchanged() {
        let source = image(&[[10, 20, 30, 255], [40, 50, 60, 255]]);
        let selection = mask(vec![128, 64]);
        assert_eq!(
            apply(&source, &selection, false, true, 1.0, 1).unwrap(),
            source
        );
    }

    #[test]
    fn rejects_pathological_neighborhood_work_before_sampling() {
        let width = 31_000;
        let source = RgbaImage::from_pixel(width, 1, Rgba([10, 20, 30, 255]));
        let selection = MaskBitmap::from_coverage(width, 1, vec![128; width as usize]).unwrap();
        assert!(matches!(
            apply(&source, &selection, false, true, 1.0, 32),
            Err(AppError::RestorationResourceLimit)
        ));
    }
}
