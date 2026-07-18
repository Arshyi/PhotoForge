use crate::domain::{ColorCastEstimate, ImageQualityAnalysis};
use image::{DynamicImage, RgbaImage};

use super::restoration::luminance;

pub fn analyze_image_quality(source: &DynamicImage) -> ImageQualityAnalysis {
    let image = source.to_rgba8();
    analyze_rgba(&image)
}

fn analyze_rgba(image: &RgbaImage) -> ImageQualityAnalysis {
    let mut histogram = [0_u64; 256];
    let mut channels = [0.0_f64; 3];
    let mut total_luminance = 0.0_f64;
    let mut opaque_count = 0_u64;
    let mut white_count = 0_u64;

    for pixel in image.pixels().filter(|pixel| pixel[3] != 0) {
        let value = luminance(pixel);
        histogram[value.round().clamp(0.0, 255.0) as usize] += 1;
        total_luminance += f64::from(value);
        for channel in 0..3 {
            channels[channel] += f64::from(pixel[channel]);
        }
        let channel_min = pixel[0].min(pixel[1]).min(pixel[2]);
        let channel_max = pixel[0].max(pixel[1]).max(pixel[2]);
        if value >= 210.0 && channel_max - channel_min <= 28 {
            white_count += 1;
        }
        opaque_count += 1;
    }

    if opaque_count == 0 {
        return empty_analysis();
    }

    let average_luminance = (total_luminance / opaque_count as f64 / 255.0) as f32;
    let low = percentile(&histogram, opaque_count, 0.05);
    let high = percentile(&histogram, opaque_count, 0.95);
    let luminance_spread = (high.saturating_sub(low)) as f32 / 255.0;
    let channel_means = channels.map(|channel| (channel / opaque_count as f64 / 255.0) as f32);
    let neutral = 0.2126 * channel_means[0] + 0.7152 * channel_means[1] + 0.0722 * channel_means[2];
    let biases = channel_means.map(|channel| channel - neutral);
    let estimated_color_cast = ColorCastEstimate {
        dominant: cast_name(biases).to_string(),
        red_bias: biases[0],
        green_bias: biases[1],
        blue_bias: biases[2],
    };

    let (estimated_noise, estimated_sharpness, estimated_local_contrast, edge_density) =
        spatial_metrics(image);
    let white_background_ratio = white_count as f32 / opaque_count as f32;
    let likely_document =
        white_background_ratio >= 0.30 && edge_density >= 0.015 && luminance_spread >= 0.20;

    ImageQualityAnalysis {
        average_luminance,
        luminance_spread,
        estimated_color_cast,
        estimated_noise,
        estimated_sharpness,
        estimated_local_contrast,
        edge_density,
        white_background_ratio,
        likely_document,
    }
}

fn empty_analysis() -> ImageQualityAnalysis {
    ImageQualityAnalysis {
        average_luminance: 0.0,
        luminance_spread: 0.0,
        estimated_color_cast: ColorCastEstimate {
            dominant: "neutral".into(),
            red_bias: 0.0,
            green_bias: 0.0,
            blue_bias: 0.0,
        },
        estimated_noise: 0.0,
        estimated_sharpness: 0.0,
        estimated_local_contrast: 0.0,
        edge_density: 0.0,
        white_background_ratio: 0.0,
        likely_document: false,
    }
}

fn percentile(histogram: &[u64; 256], total: u64, fraction: f32) -> u8 {
    let target = (total as f32 * fraction).round() as u64;
    let mut cumulative = 0_u64;
    for (value, count) in histogram.iter().enumerate() {
        cumulative += count;
        if cumulative >= target.max(1) {
            return value as u8;
        }
    }
    255
}

fn cast_name(biases: [f32; 3]) -> &'static str {
    let maximum = biases.iter().copied().fold(f32::NEG_INFINITY, f32::max);
    let minimum = biases.iter().copied().fold(f32::INFINITY, f32::min);
    if maximum - minimum < 0.035 {
        "neutral"
    } else if biases[0] == maximum && biases[2] == minimum {
        "warm"
    } else if biases[2] == maximum && biases[0] == minimum {
        "cool"
    } else if biases[1] == maximum {
        "green"
    } else {
        "mixed"
    }
}

fn spatial_metrics(image: &RgbaImage) -> (f32, f32, f32, f32) {
    if image.width() < 3 || image.height() < 3 {
        return (0.0, 0.0, 0.0, 0.0);
    }
    let mut noise_total = 0.0_f64;
    let mut noise_count = 0_u64;
    let mut laplacian_total = 0.0_f64;
    let mut contrast_total = 0.0_f64;
    let mut edge_count = 0_u64;
    let mut sample_count = 0_u64;

    for y in 1..image.height() - 1 {
        for x in 1..image.width() - 1 {
            let center_pixel = image.get_pixel(x, y);
            if center_pixel[3] == 0 {
                continue;
            }
            let center = luminance(center_pixel);
            let neighbors = [
                luminance(image.get_pixel(x - 1, y)),
                luminance(image.get_pixel(x + 1, y)),
                luminance(image.get_pixel(x, y - 1)),
                luminance(image.get_pixel(x, y + 1)),
            ];
            let mean = neighbors.iter().sum::<f32>() / 4.0;
            let local_difference = (center - mean).abs();
            contrast_total += f64::from(local_difference / 255.0);
            let laplacian = 4.0 * center - neighbors.iter().sum::<f32>();
            laplacian_total += f64::from(laplacian * laplacian);
            if local_difference > 20.0 {
                edge_count += 1;
            } else {
                noise_total += f64::from(local_difference);
                noise_count += 1;
            }
            sample_count += 1;
        }
    }

    if sample_count == 0 {
        return (0.0, 0.0, 0.0, 0.0);
    }
    let noise = if noise_count == 0 {
        0.0
    } else {
        (noise_total / noise_count as f64 / 32.0).clamp(0.0, 1.0) as f32
    };
    let sharpness = (laplacian_total / sample_count as f64 / (255.0 * 255.0 * 4.0))
        .sqrt()
        .clamp(0.0, 1.0) as f32;
    let contrast = (contrast_total / sample_count as f64).clamp(0.0, 1.0) as f32;
    let edges = edge_count as f32 / sample_count as f32;
    (noise, sharpness, contrast, edges)
}

#[cfg(test)]
mod tests {
    use super::*;
    use image::Rgba;

    fn image(width: u32, height: u32, build: impl Fn(u32, u32) -> Rgba<u8>) -> DynamicImage {
        DynamicImage::ImageRgba8(RgbaImage::from_fn(width, height, build))
    }

    #[test]
    fn reports_average_spread_and_neutral_cast() {
        let source = image(10, 10, |x, _| {
            let value = (x * 25) as u8;
            Rgba([value, value, value, 255])
        });
        let analysis = analyze_image_quality(&source);
        assert!((analysis.average_luminance - 0.44).abs() < 0.05);
        assert!(analysis.luminance_spread > 0.75);
        assert_eq!(analysis.estimated_color_cast.dominant, "neutral");
    }

    #[test]
    fn identifies_warm_and_cool_channel_imbalance() {
        let warm = image(8, 8, |_, _| Rgba([190, 120, 70, 255]));
        let cool = image(8, 8, |_, _| Rgba([70, 120, 190, 255]));
        assert_eq!(
            analyze_image_quality(&warm).estimated_color_cast.dominant,
            "warm"
        );
        assert_eq!(
            analyze_image_quality(&cool).estimated_color_cast.dominant,
            "cool"
        );
    }

    #[test]
    fn sharp_edges_score_above_flat_images() {
        let flat = image(16, 16, |_, _| Rgba([120, 120, 120, 255]));
        let edge = image(16, 16, |x, _| {
            let value = if x < 8 { 20 } else { 235 };
            Rgba([value, value, value, 255])
        });
        assert!(
            analyze_image_quality(&edge).estimated_sharpness
                > analyze_image_quality(&flat).estimated_sharpness
        );
    }

    #[test]
    fn noisy_fixture_has_a_higher_noise_estimate() {
        let flat = image(20, 20, |_, _| Rgba([120, 120, 120, 255]));
        let noisy = image(20, 20, |x, y| {
            let value = if (x + y) % 2 == 0 { 112 } else { 128 };
            Rgba([value, value, value, 255])
        });
        assert!(
            analyze_image_quality(&noisy).estimated_noise
                > analyze_image_quality(&flat).estimated_noise
        );
    }

    #[test]
    fn recognizes_a_white_document_with_dark_marks() {
        let document = image(32, 24, |x, y| {
            if y % 5 == 0 && (4..28).contains(&x) {
                Rgba([25, 25, 25, 255])
            } else {
                Rgba([240, 240, 240, 255])
            }
        });
        let analysis = analyze_image_quality(&document);
        assert!(analysis.white_background_ratio > 0.6);
        assert!(analysis.likely_document);
    }

    #[test]
    fn transparent_image_produces_finite_empty_metrics() {
        let source = image(1, 1, |_, _| Rgba([255, 0, 0, 0]));
        let analysis = analyze_image_quality(&source);
        assert_eq!(analysis, empty_analysis());
        assert!(analysis.average_luminance.is_finite());
    }
}
