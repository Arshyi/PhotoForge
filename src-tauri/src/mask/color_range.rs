use super::bitmap::MaskBitmap;
use super::geometry::Point;
use super::progress::MaskWorkContext;
use crate::error::AppError;
use image::{Rgba, RgbaImage};
use serde::{Deserialize, Serialize};
use std::sync::atomic::AtomicBool;

const MAX_COLOR_SAMPLES: usize = 32;

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ColorRangeOptions {
    pub tolerance: f32,
    pub luminance_sensitivity: f32,
    pub hue_sensitivity: f32,
    pub saturation_sensitivity: f32,
}

impl ColorRangeOptions {
    fn validate(self) -> Result<(), AppError> {
        let values = [
            self.tolerance,
            self.luminance_sensitivity,
            self.hue_sensitivity,
            self.saturation_sensitivity,
        ];
        if values
            .iter()
            .all(|value| value.is_finite() && (0.0..=1.0).contains(value))
            && self.tolerance > 0.0
        {
            Ok(())
        } else {
            Err(AppError::InvalidMask(
                "color-range settings must be finite values between 0 and 1".into(),
            ))
        }
    }
}

pub fn select(
    image: &RgbaImage,
    sample_points: &[Point],
    options: ColorRangeOptions,
    cancelled: Option<&AtomicBool>,
) -> Result<MaskBitmap, AppError> {
    select_with_progress(
        image,
        sample_points,
        options,
        MaskWorkContext::cancellation_only(cancelled),
    )
}

pub(crate) fn select_with_progress(
    image: &RgbaImage,
    sample_points: &[Point],
    options: ColorRangeOptions,
    context: MaskWorkContext<'_>,
) -> Result<MaskBitmap, AppError> {
    options.validate()?;
    if sample_points.is_empty() || sample_points.len() > MAX_COLOR_SAMPLES {
        return Err(AppError::InvalidMask(format!(
            "color range requires 1 to {MAX_COLOR_SAMPLES} sample points"
        )));
    }
    let mut samples = Vec::with_capacity(sample_points.len());
    for point in sample_points {
        point.validate()?;
        if point.x < 0.0
            || point.y < 0.0
            || point.x >= image.width() as f32
            || point.y >= image.height() as f32
        {
            return Err(AppError::InvalidMask(
                "color-range sample is outside the image".into(),
            ));
        }
        samples.push(ColorFeatures::from_pixel(
            *image.get_pixel(point.x.floor() as u32, point.y.floor() as u32),
        ));
    }

    let mut mask = MaskBitmap::empty(image.width(), image.height())?;
    let pixels = u64::from(image.width()) * u64::from(image.height());
    for (index, pixel) in image.pixels().enumerate() {
        if index % 4_096 == 0 {
            context.report("color_range_pixels", index as u64, pixels)?;
        }
        if pixel[3] == 0 {
            mask.coverage_mut()[index] = 0;
            continue;
        }
        let value = ColorFeatures::from_pixel(*pixel);
        let distance = samples
            .iter()
            .map(|sample| value.distance(*sample, options))
            .fold(f32::INFINITY, f32::min);
        let soft_limit = (options.tolerance + 0.1).min(1.0);
        let coverage = if distance <= options.tolerance {
            255
        } else if distance >= soft_limit {
            0
        } else {
            (255.0 * (1.0 - (distance - options.tolerance) / (soft_limit - options.tolerance)))
                .round() as u8
        };
        mask.coverage_mut()[index] =
            ((u16::from(coverage) * u16::from(pixel[3]) + 127) / 255) as u8;
    }
    context.report("color_range_pixels", pixels, pixels)?;
    Ok(mask)
}

#[derive(Debug, Clone, Copy)]
struct ColorFeatures {
    hue: f32,
    saturation: f32,
    luminance: f32,
}

impl ColorFeatures {
    fn from_pixel(pixel: Rgba<u8>) -> Self {
        let red = srgb_to_linear(pixel[0] as f32 / 255.0);
        let green = srgb_to_linear(pixel[1] as f32 / 255.0);
        let blue = srgb_to_linear(pixel[2] as f32 / 255.0);
        let max = red.max(green).max(blue);
        let min = red.min(green).min(blue);
        let delta = max - min;
        let saturation = if max <= f32::EPSILON {
            0.0
        } else {
            delta / max
        };
        let hue = if delta <= f32::EPSILON {
            0.0
        } else if max == red {
            ((green - blue) / delta).rem_euclid(6.0) / 6.0
        } else if max == green {
            ((blue - red) / delta + 2.0) / 6.0
        } else {
            ((red - green) / delta + 4.0) / 6.0
        };
        Self {
            hue,
            saturation,
            luminance: 0.2126 * red + 0.7152 * green + 0.0722 * blue,
        }
    }

    fn distance(self, other: Self, options: ColorRangeOptions) -> f32 {
        let hue_delta = (self.hue - other.hue).abs();
        let circular_hue = hue_delta.min(1.0 - hue_delta) * 2.0;
        let low_saturation_weight = self.saturation.min(other.saturation).clamp(0.0, 1.0);
        let hue = circular_hue * low_saturation_weight * options.hue_sensitivity;
        let saturation =
            (self.saturation - other.saturation).abs() * options.saturation_sensitivity;
        let luminance =
            (self.luminance - other.luminance).abs().sqrt() * options.luminance_sensitivity;
        let total_weight = options.hue_sensitivity
            + options.saturation_sensitivity
            + options.luminance_sensitivity;
        if total_weight <= f32::EPSILON {
            0.0
        } else {
            (hue * hue + saturation * saturation + luminance * luminance).sqrt()
                / total_weight.sqrt()
        }
    }
}

fn srgb_to_linear(value: f32) -> f32 {
    if value <= 0.04045 {
        value / 12.92
    } else {
        ((value + 0.055) / 1.055).powf(2.4)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn options() -> ColorRangeOptions {
        ColorRangeOptions {
            tolerance: 0.08,
            luminance_sensitivity: 1.0,
            hue_sensitivity: 1.0,
            saturation_sensitivity: 1.0,
        }
    }

    #[test]
    fn selects_similar_color_and_rejects_different_color() {
        let mut image = RgbaImage::new(3, 1);
        image.put_pixel(0, 0, Rgba([200, 30, 30, 255]));
        image.put_pixel(1, 0, Rgba([202, 31, 31, 255]));
        image.put_pixel(2, 0, Rgba([30, 30, 200, 255]));
        let mask = select(&image, &[Point { x: 0.0, y: 0.0 }], options(), None).unwrap();
        assert_eq!(mask.get(0, 0), 255);
        assert!(mask.get(1, 0) > 200);
        assert_eq!(mask.get(2, 0), 0);
    }

    #[test]
    fn transparent_pixels_are_unselected() {
        let image = RgbaImage::from_pixel(1, 1, Rgba([200, 30, 30, 0]));
        let mask = select(&image, &[Point { x: 0.0, y: 0.0 }], options(), None).unwrap();
        assert_eq!(mask.get(0, 0), 0);
    }

    #[test]
    fn grayscale_and_low_saturation_ignore_unstable_hue() {
        let first = ColorFeatures::from_pixel(Rgba([80, 80, 80, 255]));
        let second = ColorFeatures::from_pixel(Rgba([82, 81, 81, 255]));
        assert!(first.distance(second, options()) < 0.08);
    }
}
