use super::processor::clamp;
use image::{GrayImage, Luma, Rgba, RgbaImage};

pub fn auto_white_balance(source: &RgbaImage, strength: f32) -> RgbaImage {
    if strength == 0.0 {
        return source.clone();
    }

    let mut histograms = [[0_u32; 256]; 3];
    let mut count = 0_u64;
    for pixel in source.pixels().filter(|pixel| pixel[3] != 0) {
        count += 1;
        for channel in 0..3 {
            histograms[channel][pixel[channel] as usize] += 1;
        }
    }
    if count == 0 {
        return source.clone();
    }

    let means = histograms.map(|histogram| trimmed_mean(&histogram, count));
    let target = 0.2126 * means[0] + 0.7152 * means[1] + 0.0722 * means[2];
    if target <= f32::EPSILON {
        return source.clone();
    }
    let mut gains = means.map(|mean| {
        if mean <= 1.0 {
            1.0
        } else {
            (target / mean).clamp(0.67, 1.5)
        }
    });
    let weighted_gain = 0.2126 * gains[0] + 0.7152 * gains[1] + 0.0722 * gains[2];
    if weighted_gain > f32::EPSILON {
        for gain in &mut gains {
            *gain = (*gain / weighted_gain).clamp(0.67, 1.5);
        }
    }

    RgbaImage::from_fn(source.width(), source.height(), |x, y| {
        let pixel = source.get_pixel(x, y);
        if pixel[3] == 0 {
            return *pixel;
        }
        Rgba([
            clamp(pixel[0] as f32 * (1.0 + (gains[0] - 1.0) * strength)),
            clamp(pixel[1] as f32 * (1.0 + (gains[1] - 1.0) * strength)),
            clamp(pixel[2] as f32 * (1.0 + (gains[2] - 1.0) * strength)),
            pixel[3],
        ])
    })
}

fn trimmed_mean(histogram: &[u32; 256], total: u64) -> f32 {
    let trim = total / 20;
    let start = trim;
    let end = total.saturating_sub(trim);
    let mut seen = 0_u64;
    let mut retained = 0_u64;
    let mut weighted = 0_u64;
    for (value, count) in histogram.iter().enumerate() {
        let bin_start = seen;
        let bin_end = seen + u64::from(*count);
        let overlap = bin_end.min(end).saturating_sub(bin_start.max(start));
        retained += overlap;
        weighted += overlap * value as u64;
        seen = bin_end;
    }
    if retained == 0 {
        0.0
    } else {
        weighted as f32 / retained as f32
    }
}

pub fn local_contrast(
    source: &RgbaImage,
    strength: f32,
    tile_size: u32,
    clip_limit: f32,
) -> RgbaImage {
    if strength == 0.0 || source.width() == 0 || source.height() == 0 {
        return source.clone();
    }
    let luma = luma_image(source);
    let background = box_blur_luma(&luma, (tile_size / 2).max(1));
    let maximum = 12.0 * clip_limit;
    adjust_luma(source, |x, y, current| {
        let local = background.get_pixel(x, y)[0] as f32;
        current + (current - local).clamp(-maximum, maximum) * strength
    })
}

pub fn denoise(source: &RgbaImage, strength: f32, preserve_edges: f32) -> RgbaImage {
    if strength == 0.0 || source.width() == 0 || source.height() == 0 {
        return source.clone();
    }
    let radius = if strength > 0.65 { 2_i32 } else { 1_i32 };
    let range_scale = 8.0 + (1.0 - preserve_edges) * 64.0;
    let mut output = source.clone();
    for y in 0..source.height() {
        for x in 0..source.width() {
            let center = source.get_pixel(x, y);
            if center[3] == 0 {
                continue;
            }
            let mut sums = [0.0_f32; 3];
            let mut weight_sum = 0.0_f32;
            for offset_y in -radius..=radius {
                for offset_x in -radius..=radius {
                    let sample_x = (x as i32 + offset_x).clamp(0, source.width() as i32 - 1) as u32;
                    let sample_y =
                        (y as i32 + offset_y).clamp(0, source.height() as i32 - 1) as u32;
                    let sample = source.get_pixel(sample_x, sample_y);
                    if sample[3] == 0 {
                        continue;
                    }
                    let distance = (0..3)
                        .map(|channel| (center[channel] as f32 - sample[channel] as f32).abs())
                        .sum::<f32>()
                        / 3.0;
                    let spatial = 1.0 / (1.0 + (offset_x * offset_x + offset_y * offset_y) as f32);
                    let range = 1.0 / (1.0 + distance / range_scale);
                    let weight = spatial * range;
                    for channel in 0..3 {
                        sums[channel] += sample[channel] as f32 * weight;
                    }
                    weight_sum += weight;
                }
            }
            if weight_sum > f32::EPSILON {
                let filtered = sums.map(|sum| sum / weight_sum);
                output.put_pixel(
                    x,
                    y,
                    Rgba([
                        clamp(center[0] as f32 + (filtered[0] - center[0] as f32) * strength),
                        clamp(center[1] as f32 + (filtered[1] - center[1] as f32) * strength),
                        clamp(center[2] as f32 + (filtered[2] - center[2] as f32) * strength),
                        center[3],
                    ]),
                );
            }
        }
    }
    output
}

pub fn deblock(source: &RgbaImage, strength: f32) -> RgbaImage {
    if strength == 0.0 {
        return source.clone();
    }
    let mut output = source.clone();
    let blend = strength * 0.38;
    for boundary in (8..source.width()).step_by(8) {
        for y in 0..source.height() {
            soften_boundary(source, &mut output, boundary - 1, y, boundary, y, blend);
        }
    }
    for boundary in (8..source.height()).step_by(8) {
        for x in 0..source.width() {
            soften_boundary(source, &mut output, x, boundary - 1, x, boundary, blend);
        }
    }
    output
}

fn soften_boundary(
    source: &RgbaImage,
    output: &mut RgbaImage,
    first_x: u32,
    first_y: u32,
    second_x: u32,
    second_y: u32,
    blend: f32,
) {
    let first = source.get_pixel(first_x, first_y);
    let second = source.get_pixel(second_x, second_y);
    if first[3] == 0 || second[3] == 0 {
        return;
    }
    let difference = (luminance(first) - luminance(second)).abs();
    if !(4.0..=80.0).contains(&difference) {
        return;
    }
    let average = [
        (u16::from(first[0]) + u16::from(second[0])) as f32 / 2.0,
        (u16::from(first[1]) + u16::from(second[1])) as f32 / 2.0,
        (u16::from(first[2]) + u16::from(second[2])) as f32 / 2.0,
    ];
    output.put_pixel(
        first_x,
        first_y,
        Rgba([
            clamp(first[0] as f32 + (average[0] - first[0] as f32) * blend),
            clamp(first[1] as f32 + (average[1] - first[1] as f32) * blend),
            clamp(first[2] as f32 + (average[2] - first[2] as f32) * blend),
            first[3],
        ]),
    );
    output.put_pixel(
        second_x,
        second_y,
        Rgba([
            clamp(second[0] as f32 + (average[0] - second[0] as f32) * blend),
            clamp(second[1] as f32 + (average[1] - second[1] as f32) * blend),
            clamp(second[2] as f32 + (average[2] - second[2] as f32) * blend),
            second[3],
        ]),
    );
}

pub fn edge_aware_sharpen(
    source: &RgbaImage,
    strength: f32,
    radius: f32,
    threshold: f32,
) -> RgbaImage {
    if strength == 0.0 {
        return source.clone();
    }
    let luma = luma_image(source);
    let blurred = box_blur_luma(&luma, radius.round().clamp(1.0, 4.0) as u32);
    let threshold = threshold * 255.0;
    let correction_limit = 24.0 + strength * 20.0;
    adjust_luma(source, |x, y, current| {
        let detail = current - blurred.get_pixel(x, y)[0] as f32;
        let correction = if detail.abs() >= threshold {
            (detail * strength).clamp(-correction_limit, correction_limit)
        } else {
            0.0
        };
        current + correction
    })
}

pub fn mild_deblur(source: &RgbaImage, strength: f32, radius: f32) -> RgbaImage {
    if strength == 0.0 {
        return source.clone();
    }
    let first = edge_aware_sharpen(source, strength * 0.72, radius, 0.025);
    edge_aware_sharpen(&first, strength * 0.28, (radius * 0.6).max(0.5), 0.04)
}

pub fn uneven_lighting(source: &RgbaImage, strength: f32, radius: f32) -> RgbaImage {
    if strength == 0.0 || source.width() == 0 || source.height() == 0 {
        return source.clone();
    }
    let luma = luma_image(source);
    let background = box_blur_luma(&luma, radius.round().clamp(4.0, 96.0) as u32);
    let mut total = 0.0_f64;
    let mut count = 0_u64;
    for (pixel, original) in luma.pixels().zip(source.pixels()) {
        if original[3] != 0 {
            total += f64::from(pixel[0]);
            count += 1;
        }
    }
    if count == 0 {
        return source.clone();
    }
    let target = (total / count as f64) as f32;
    adjust_luma(source, |x, y, current| {
        let estimated = background.get_pixel(x, y)[0] as f32;
        current + ((target - estimated) * strength).clamp(-72.0, 72.0)
    })
}

pub fn document_enhance(source: &RgbaImage, strength: f32, grayscale: bool) -> RgbaImage {
    if strength == 0.0 {
        return source.clone();
    }
    let balanced = auto_white_balance(source, strength * 0.45);
    let lit = uneven_lighting(&balanced, strength * 0.78, 32.0);
    let contrasted = local_contrast(&lit, strength * 0.72, 32, 1.6);
    let clean = denoise(&contrasted, strength * 0.22, 0.85);
    let sharp = edge_aware_sharpen(&clean, strength * 0.55, 1.0, 0.025);
    if grayscale {
        RgbaImage::from_fn(sharp.width(), sharp.height(), |x, y| {
            let pixel = sharp.get_pixel(x, y);
            let value = clamp(luminance(pixel));
            Rgba([value, value, value, pixel[3]])
        })
    } else {
        sharp
    }
}

fn adjust_luma<F>(source: &RgbaImage, mut transform: F) -> RgbaImage
where
    F: FnMut(u32, u32, f32) -> f32,
{
    RgbaImage::from_fn(source.width(), source.height(), |x, y| {
        let pixel = source.get_pixel(x, y);
        if pixel[3] == 0 {
            return *pixel;
        }
        let current = luminance(pixel);
        let desired = transform(x, y, current).clamp(0.0, 255.0);
        let delta = desired - current;
        Rgba([
            clamp(pixel[0] as f32 + delta),
            clamp(pixel[1] as f32 + delta),
            clamp(pixel[2] as f32 + delta),
            pixel[3],
        ])
    })
}

pub(crate) fn luma_image(source: &RgbaImage) -> GrayImage {
    GrayImage::from_fn(source.width(), source.height(), |x, y| {
        Luma([clamp(luminance(source.get_pixel(x, y)))])
    })
}

pub(crate) fn box_blur_luma(source: &GrayImage, radius: u32) -> GrayImage {
    if radius == 0 || source.width() == 0 || source.height() == 0 {
        return source.clone();
    }
    let width = source.width();
    let height = source.height();
    let window = radius * 2 + 1;
    let mut horizontal = GrayImage::new(width, height);
    for y in 0..height {
        let mut sum = 0_u32;
        for offset in -(radius as i32)..=radius as i32 {
            let x = offset.clamp(0, width as i32 - 1) as u32;
            sum += u32::from(source.get_pixel(x, y)[0]);
        }
        for x in 0..width {
            horizontal.put_pixel(x, y, Luma([(sum / window) as u8]));
            let remove = (x as i32 - radius as i32).clamp(0, width as i32 - 1) as u32;
            let add = (x as i32 + radius as i32 + 1).clamp(0, width as i32 - 1) as u32;
            sum = sum + u32::from(source.get_pixel(add, y)[0])
                - u32::from(source.get_pixel(remove, y)[0]);
        }
    }

    let mut output = GrayImage::new(width, height);
    for x in 0..width {
        let mut sum = 0_u32;
        for offset in -(radius as i32)..=radius as i32 {
            let y = offset.clamp(0, height as i32 - 1) as u32;
            sum += u32::from(horizontal.get_pixel(x, y)[0]);
        }
        for y in 0..height {
            output.put_pixel(x, y, Luma([(sum / window) as u8]));
            let remove = (y as i32 - radius as i32).clamp(0, height as i32 - 1) as u32;
            let add = (y as i32 + radius as i32 + 1).clamp(0, height as i32 - 1) as u32;
            sum = sum + u32::from(horizontal.get_pixel(x, add)[0])
                - u32::from(horizontal.get_pixel(x, remove)[0]);
        }
    }
    output
}

pub(crate) fn luminance(pixel: &Rgba<u8>) -> f32 {
    0.2126 * pixel[0] as f32 + 0.7152 * pixel[1] as f32 + 0.0722 * pixel[2] as f32
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rgba(width: u32, height: u32, build: impl Fn(u32, u32) -> Rgba<u8>) -> RgbaImage {
        RgbaImage::from_fn(width, height, build)
    }

    fn variance(image: &RgbaImage) -> f32 {
        let values = image.pixels().map(luminance).collect::<Vec<_>>();
        let mean = values.iter().sum::<f32>() / values.len() as f32;
        values
            .iter()
            .map(|value| (value - mean).powi(2))
            .sum::<f32>()
            / values.len() as f32
    }

    #[test]
    fn white_balance_neutral_and_zero_strength_are_stable() {
        let neutral = RgbaImage::from_pixel(4, 4, Rgba([120, 120, 120, 73]));
        assert_eq!(auto_white_balance(&neutral, 1.0), neutral);
        let warm = RgbaImage::from_pixel(4, 4, Rgba([180, 110, 80, 73]));
        assert_eq!(auto_white_balance(&warm, 0.0), warm);
    }

    #[test]
    fn white_balance_reduces_warm_and_cool_casts() {
        for source in [Rgba([190, 120, 80, 255]), Rgba([80, 120, 190, 255])] {
            let image = RgbaImage::from_pixel(8, 8, source);
            let corrected = auto_white_balance(&image, 1.0);
            let before = source[0].abs_diff(source[2]);
            let after = corrected.get_pixel(0, 0)[0].abs_diff(corrected.get_pixel(0, 0)[2]);
            assert!(after < before);
        }
    }

    #[test]
    fn white_balance_handles_black_and_transparency() {
        let source = rgba(2, 1, |x, _| {
            if x == 0 {
                Rgba([0, 0, 0, 255])
            } else {
                Rgba([9, 8, 7, 0])
            }
        });
        let result = auto_white_balance(&source, 1.0);
        assert_eq!(result.get_pixel(0, 0), &Rgba([0, 0, 0, 255]));
        assert_eq!(result.get_pixel(1, 0), &Rgba([9, 8, 7, 0]));
    }

    #[test]
    fn local_contrast_keeps_flat_images_and_improves_a_soft_edge() {
        let flat = RgbaImage::from_pixel(12, 12, Rgba([100, 100, 100, 41]));
        assert_eq!(local_contrast(&flat, 1.0, 8, 2.0), flat);
        let soft = rgba(16, 8, |x, _| {
            let value = if x < 8 { 105 } else { 135 };
            Rgba([value, value, value, 255])
        });
        let result = local_contrast(&soft, 1.0, 8, 2.0);
        assert!(result.get_pixel(8, 4)[0] - result.get_pixel(7, 4)[0] > 30);
    }

    #[test]
    fn denoise_reduces_noise_while_retaining_a_major_edge() {
        let noisy = rgba(20, 12, |x, y| {
            let base: i16 = if x < 10 { 45 } else { 205 };
            let noise: i16 = if (x + y) % 2 == 0 { 18 } else { -18 };
            let value = (base + noise).clamp(0, 255) as u8;
            Rgba([value, value, value, 77])
        });
        let result = denoise(&noisy, 1.0, 0.9);
        assert!(variance(&result) < variance(&noisy));
        assert!(result.get_pixel(12, 6)[0] as i16 - result.get_pixel(7, 6)[0] as i16 > 100);
        assert!(result.pixels().all(|pixel| pixel[3] == 77));
    }

    #[test]
    fn deblock_softens_only_moderate_block_boundaries() {
        let blocks = rgba(16, 8, |x, _| {
            let value = if x < 8 { 90 } else { 120 };
            Rgba([value, value, value, 55])
        });
        let result = deblock(&blocks, 1.0);
        assert!(result.get_pixel(7, 4)[0] > 90);
        assert!(result.get_pixel(8, 4)[0] < 120);
        assert!(result.pixels().all(|pixel| pixel[3] == 55));
    }

    #[test]
    fn edge_aware_sharpen_respects_flat_regions_and_threshold() {
        let flat = RgbaImage::from_pixel(9, 9, Rgba([100, 100, 100, 66]));
        assert_eq!(edge_aware_sharpen(&flat, 2.0, 2.0, 0.0), flat);
        let slight_noise = rgba(9, 9, |x, y| {
            let value = 100 + ((x + y) % 2) as u8;
            Rgba([value, value, value, 66])
        });
        assert_eq!(
            edge_aware_sharpen(&slight_noise, 1.0, 1.0, 0.1),
            slight_noise
        );
    }

    #[test]
    fn sharpening_and_deblur_increase_edge_contrast_and_preserve_alpha() {
        let soft_edge = rgba(15, 5, |x, _| {
            let value = if x < 7 {
                70
            } else if x == 7 {
                125
            } else {
                180
            };
            Rgba([value, value, value, 19])
        });
        let sharpened = edge_aware_sharpen(&soft_edge, 1.0, 2.0, 0.01);
        let restored = mild_deblur(&soft_edge, 0.8, 1.5);
        assert!(sharpened.get_pixel(9, 2)[0] - sharpened.get_pixel(5, 2)[0] > 110);
        assert!(restored.get_pixel(9, 2)[0] - restored.get_pixel(5, 2)[0] > 110);
        assert!(restored.pixels().all(|pixel| pixel[3] == 19));
    }

    #[test]
    fn uneven_lighting_reduces_a_luminance_gradient() {
        let gradient = rgba(32, 8, |x, _| {
            let value = 60 + (x * 4) as u8;
            Rgba([value, value, value, 123])
        });
        let corrected = uneven_lighting(&gradient, 1.0, 8.0);
        let before = gradient.get_pixel(31, 4)[0] - gradient.get_pixel(0, 4)[0];
        let after = corrected.get_pixel(31, 4)[0] - corrected.get_pixel(0, 4)[0];
        assert!(after < before);
        assert!(corrected.pixels().all(|pixel| pixel[3] == 123));
    }

    #[test]
    fn document_enhancement_supports_color_and_grayscale() {
        let document = rgba(24, 12, |x, y| {
            let background = 205 + (x / 6) as u8 * 5;
            if y == 5 && (3..21).contains(&x) {
                Rgba([45, 55, 65, 88])
            } else if x == 4 && y == 8 {
                Rgba([180, 45, 35, 88])
            } else {
                Rgba([background, background, background, 88])
            }
        });
        let color = document_enhance(&document, 0.7, false);
        let gray = document_enhance(&document, 0.7, true);
        assert_ne!(color.get_pixel(4, 8)[0], color.get_pixel(4, 8)[1]);
        assert_eq!(gray.get_pixel(4, 8)[0], gray.get_pixel(4, 8)[1]);
        assert!(gray.pixels().all(|pixel| pixel[3] == 88));
    }

    #[test]
    fn all_restoration_helpers_handle_one_pixel_images() {
        let tiny = RgbaImage::from_pixel(1, 1, Rgba([10, 20, 30, 40]));
        assert_eq!(local_contrast(&tiny, 1.0, 128, 4.0).dimensions(), (1, 1));
        assert_eq!(denoise(&tiny, 1.0, 1.0).dimensions(), (1, 1));
        assert_eq!(deblock(&tiny, 1.0).dimensions(), (1, 1));
        assert_eq!(uneven_lighting(&tiny, 1.0, 96.0).dimensions(), (1, 1));
        assert_eq!(document_enhance(&tiny, 1.0, true).dimensions(), (1, 1));
    }
}
