use crate::domain::{
    CurvePoint, CurveSet, EditOperation, HslAdjustment, HslSettings, PerspectiveCorners,
    SelectiveColorAdjustment,
};
use image::{Rgba, RgbaImage};

use super::processor::clamp;

pub(crate) fn apply(image: &RgbaImage, operation: &EditOperation) -> RgbaImage {
    match operation {
        EditOperation::Curves { curves } => curves_image(image, curves),
        EditOperation::Levels {
            input_black,
            input_white,
            gamma,
            output_black,
            output_white,
        } => levels(
            image,
            *input_black,
            *input_white,
            *gamma,
            *output_black,
            *output_white,
        ),
        EditOperation::WhitePoint { red, green, blue } => {
            point_balance(image, [*red, *green, *blue], true)
        }
        EditOperation::BlackPoint { red, green, blue } => {
            point_balance(image, [*red, *green, *blue], false)
        }
        EditOperation::Crop {
            x,
            y,
            width,
            height,
            ..
        } => crop(image, *x, *y, *width, *height),
        EditOperation::Straighten { degrees } => rotate_arbitrary(image, *degrees),
        EditOperation::Perspective { corners } => perspective(image, corners),
        EditOperation::LensCorrection {
            distortion,
            vignetting,
            chromatic_aberration,
        } => lens_correction(image, *distortion, *vignetting, *chromatic_aberration),
        EditOperation::Hsl { settings } => hsl(image, settings),
        EditOperation::TemperatureTint { temperature, tint } => {
            temperature_tint(image, *temperature, *tint)
        }
        EditOperation::SelectiveColor {
            target_hue,
            width,
            adjustment,
        } => selective_color(image, *target_hue, *width, adjustment),
        _ => image.clone(),
    }
}

fn curves_image(source: &RgbaImage, curves: &CurveSet) -> RgbaImage {
    let rgb = curve_lut(&curves.rgb);
    let channels = [
        curve_lut(&curves.red),
        curve_lut(&curves.green),
        curve_lut(&curves.blue),
    ];
    map(source, |pixel| {
        let mut output = pixel;
        for channel in 0..3 {
            output[channel] = channels[channel][rgb[pixel[channel] as usize] as usize];
        }
        output
    })
}

fn curve_lut(points: &[CurvePoint]) -> [u8; 256] {
    let mut lut = [0_u8; 256];
    for (index, entry) in lut.iter_mut().enumerate() {
        let input = index as f32 / 255.0;
        let pair = points
            .windows(2)
            .find(|pair| input <= pair[1].input)
            .unwrap_or(&points[points.len() - 2..]);
        let span = (pair[1].input - pair[0].input).max(f32::EPSILON);
        let amount = ((input - pair[0].input) / span).clamp(0.0, 1.0);
        *entry = clamp((pair[0].output + (pair[1].output - pair[0].output) * amount) * 255.0);
    }
    lut
}

fn levels(
    source: &RgbaImage,
    input_black: u8,
    input_white: u8,
    gamma: f32,
    output_black: u8,
    output_white: u8,
) -> RgbaImage {
    let input_span = f32::from(input_white - input_black);
    let output_span = f32::from(output_white - output_black);
    map(source, |mut pixel| {
        for channel in &mut pixel.0[..3] {
            let normalized = ((f32::from(*channel) - f32::from(input_black)) / input_span)
                .clamp(0.0, 1.0)
                .powf(1.0 / gamma);
            *channel = clamp(f32::from(output_black) + normalized * output_span);
        }
        pixel
    })
}

fn point_balance(source: &RgbaImage, point: [u8; 3], white: bool) -> RgbaImage {
    map(source, |mut pixel| {
        for channel in 0..3 {
            pixel[channel] = if white {
                clamp(f32::from(pixel[channel]) * 255.0 / f32::from(point[channel].max(1)))
            } else {
                clamp(f32::from(pixel[channel]) - f32::from(point[channel]))
            };
        }
        pixel
    })
}

fn crop(source: &RgbaImage, x: f32, y: f32, width: f32, height: f32) -> RgbaImage {
    let source_width = source.width();
    let source_height = source.height();
    let left = (x * source_width as f32).floor() as u32;
    let top = (y * source_height as f32).floor() as u32;
    let width = ((width * source_width as f32).round() as u32)
        .max(1)
        .min(source_width - left);
    let height = ((height * source_height as f32).round() as u32)
        .max(1)
        .min(source_height - top);
    image::imageops::crop_imm(source, left, top, width, height).to_image()
}

fn rotate_arbitrary(source: &RgbaImage, degrees: f32) -> RgbaImage {
    if degrees.abs() < f32::EPSILON {
        return source.clone();
    }
    let radians = degrees.to_radians();
    let (sin, cos) = radians.sin_cos();
    let center_x = (source.width() as f32 - 1.0) * 0.5;
    let center_y = (source.height() as f32 - 1.0) * 0.5;
    RgbaImage::from_fn(source.width(), source.height(), |x, y| {
        let dx = x as f32 - center_x;
        let dy = y as f32 - center_y;
        sample(
            source,
            center_x + dx * cos + dy * sin,
            center_y - dx * sin + dy * cos,
        )
    })
}

fn perspective(source: &RgbaImage, corners: &PerspectiveCorners) -> RgbaImage {
    let width = source.width();
    let height = source.height();
    RgbaImage::from_fn(width, height, |x, y| {
        let u = if width > 1 {
            x as f32 / (width - 1) as f32
        } else {
            0.0
        };
        let v = if height > 1 {
            y as f32 / (height - 1) as f32
        } else {
            0.0
        };
        let top = mix_point(corners.top_left, corners.top_right, u);
        let bottom = mix_point(corners.bottom_left, corners.bottom_right, u);
        let position = mix_point(top, bottom, v);
        sample(
            source,
            position[0] * (width.saturating_sub(1)) as f32,
            position[1] * (height.saturating_sub(1)) as f32,
        )
    })
}

fn mix_point(left: [f32; 2], right: [f32; 2], amount: f32) -> [f32; 2] {
    [
        left[0] + (right[0] - left[0]) * amount,
        left[1] + (right[1] - left[1]) * amount,
    ]
}

fn lens_correction(
    source: &RgbaImage,
    distortion: f32,
    vignetting: f32,
    chromatic_aberration: f32,
) -> RgbaImage {
    let width = source.width();
    let height = source.height();
    let center_x = (width as f32 - 1.0) * 0.5;
    let center_y = (height as f32 - 1.0) * 0.5;
    let scale_x = center_x.max(1.0);
    let scale_y = center_y.max(1.0);
    RgbaImage::from_fn(width, height, |x, y| {
        let nx = (x as f32 - center_x) / scale_x;
        let ny = (y as f32 - center_y) / scale_y;
        let radius_squared = nx * nx + ny * ny;
        let radial = 1.0 + distortion * radius_squared;
        let source_x = center_x + nx * radial * scale_x;
        let source_y = center_y + ny * radial * scale_y;
        let shift = chromatic_aberration * radius_squared * 2.0;
        let red = sample(source, source_x + nx * shift, source_y + ny * shift)[0];
        let center = sample(source, source_x, source_y);
        let blue = sample(source, source_x - nx * shift, source_y - ny * shift)[2];
        let light = (1.0 + vignetting * radius_squared).max(0.0);
        Rgba([
            clamp(f32::from(red) * light),
            clamp(f32::from(center[1]) * light),
            clamp(f32::from(blue) * light),
            center[3],
        ])
    })
}

fn hsl(source: &RgbaImage, settings: &HslSettings) -> RgbaImage {
    map(source, |pixel| {
        let (mut hue, mut saturation, mut lightness) = rgb_to_hsl(pixel[0], pixel[1], pixel[2]);
        let channel = hue_adjustment(settings, hue);
        hue = (hue + settings.master.hue + channel.hue).rem_euclid(360.0);
        saturation =
            (saturation * (1.0 + settings.master.saturation + channel.saturation)).clamp(0.0, 1.0);
        lightness =
            (lightness + settings.master.lightness * 0.5 + channel.lightness * 0.5).clamp(0.0, 1.0);
        let [red, green, blue] = hsl_to_rgb(hue, saturation, lightness);
        Rgba([red, green, blue, pixel[3]])
    })
}

fn hue_adjustment(settings: &HslSettings, hue: f32) -> HslAdjustment {
    let entries = [
        (0.0, settings.red),
        (60.0, settings.yellow),
        (120.0, settings.green),
        (180.0, settings.cyan),
        (240.0, settings.blue),
        (300.0, settings.magenta),
    ];
    let (center, adjustment) = entries
        .iter()
        .min_by(|(left, _), (right, _)| {
            hue_distance(hue, *left)
                .partial_cmp(&hue_distance(hue, *right))
                .unwrap_or(std::cmp::Ordering::Equal)
        })
        .copied()
        .unwrap_or_default();
    let weight = (1.0 - hue_distance(hue, center) / 60.0).clamp(0.0, 1.0);
    HslAdjustment {
        hue: adjustment.hue * weight,
        saturation: adjustment.saturation * weight,
        lightness: adjustment.lightness * weight,
    }
}

fn hue_distance(left: f32, right: f32) -> f32 {
    let distance = (left - right).abs().rem_euclid(360.0);
    distance.min(360.0 - distance)
}

fn temperature_tint(source: &RgbaImage, temperature: f32, tint: f32) -> RgbaImage {
    map(source, |pixel| {
        Rgba([
            clamp(f32::from(pixel[0]) * (1.0 + temperature * 0.25 + tint * 0.05)),
            clamp(f32::from(pixel[1]) * (1.0 - tint * 0.18)),
            clamp(f32::from(pixel[2]) * (1.0 - temperature * 0.25 + tint * 0.05)),
            pixel[3],
        ])
    })
}

fn selective_color(
    source: &RgbaImage,
    target_hue: f32,
    width: f32,
    adjustment: &SelectiveColorAdjustment,
) -> RgbaImage {
    map(source, |pixel| {
        let (hue, _, _) = rgb_to_hsl(pixel[0], pixel[1], pixel[2]);
        let weight = (1.0 - hue_distance(hue, target_hue) / width).clamp(0.0, 1.0);
        let black_factor = 1.0 - adjustment.black * weight;
        Rgba([
            clamp(f32::from(pixel[0]) * (1.0 - adjustment.cyan * weight) * black_factor),
            clamp(f32::from(pixel[1]) * (1.0 - adjustment.magenta * weight) * black_factor),
            clamp(f32::from(pixel[2]) * (1.0 - adjustment.yellow * weight) * black_factor),
            pixel[3],
        ])
    })
}

fn rgb_to_hsl(red: u8, green: u8, blue: u8) -> (f32, f32, f32) {
    let red = f32::from(red) / 255.0;
    let green = f32::from(green) / 255.0;
    let blue = f32::from(blue) / 255.0;
    let maximum = red.max(green).max(blue);
    let minimum = red.min(green).min(blue);
    let delta = maximum - minimum;
    let lightness = (maximum + minimum) * 0.5;
    if delta <= f32::EPSILON {
        return (0.0, 0.0, lightness);
    }
    let saturation = delta / (1.0 - (2.0 * lightness - 1.0).abs()).max(f32::EPSILON);
    let hue = if maximum == red {
        60.0 * ((green - blue) / delta).rem_euclid(6.0)
    } else if maximum == green {
        60.0 * ((blue - red) / delta + 2.0)
    } else {
        60.0 * ((red - green) / delta + 4.0)
    };
    (hue, saturation, lightness)
}

fn hsl_to_rgb(hue: f32, saturation: f32, lightness: f32) -> [u8; 3] {
    let chroma = (1.0 - (2.0 * lightness - 1.0).abs()) * saturation;
    let section = hue / 60.0;
    let secondary = chroma * (1.0 - (section.rem_euclid(2.0) - 1.0).abs());
    let (red, green, blue) = match section.floor() as i32 {
        0 => (chroma, secondary, 0.0),
        1 => (secondary, chroma, 0.0),
        2 => (0.0, chroma, secondary),
        3 => (0.0, secondary, chroma),
        4 => (secondary, 0.0, chroma),
        _ => (chroma, 0.0, secondary),
    };
    let offset = lightness - chroma * 0.5;
    [
        clamp((red + offset) * 255.0),
        clamp((green + offset) * 255.0),
        clamp((blue + offset) * 255.0),
    ]
}

fn sample(source: &RgbaImage, x: f32, y: f32) -> Rgba<u8> {
    if x < 0.0 || y < 0.0 || x > (source.width() - 1) as f32 || y > (source.height() - 1) as f32 {
        return Rgba([0, 0, 0, 0]);
    }
    let left = x.floor() as u32;
    let top = y.floor() as u32;
    let right = (left + 1).min(source.width() - 1);
    let bottom = (top + 1).min(source.height() - 1);
    let fx = x - left as f32;
    let fy = y - top as f32;
    let mut output = [0_u8; 4];
    for (channel, output_channel) in output.iter_mut().enumerate() {
        let top_value = f32::from(source.get_pixel(left, top)[channel]) * (1.0 - fx)
            + f32::from(source.get_pixel(right, top)[channel]) * fx;
        let bottom_value = f32::from(source.get_pixel(left, bottom)[channel]) * (1.0 - fx)
            + f32::from(source.get_pixel(right, bottom)[channel]) * fx;
        *output_channel = clamp(top_value * (1.0 - fy) + bottom_value * fy);
    }
    Rgba(output)
}

fn map<F>(source: &RgbaImage, mut transform: F) -> RgbaImage
where
    F: FnMut(Rgba<u8>) -> Rgba<u8>,
{
    RgbaImage::from_fn(source.width(), source.height(), |x, y| {
        transform(*source.get_pixel(x, y))
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::{CropOverlay, CurveSet};

    fn fixture() -> RgbaImage {
        RgbaImage::from_fn(16, 12, |x, y| {
            Rgba([(x * 13) as u8, (y * 17) as u8, ((x + y) * 7) as u8, 173])
        })
    }

    #[test]
    fn identity_curves_preserve_pixels() {
        assert_eq!(curves_image(&fixture(), &CurveSet::default()), fixture());
    }

    #[test]
    fn curves_interpolate_multiple_control_points() {
        let curves = CurveSet {
            rgb: vec![
                CurvePoint {
                    input: 0.0,
                    output: 0.0,
                },
                CurvePoint {
                    input: 0.5,
                    output: 0.75,
                },
                CurvePoint {
                    input: 1.0,
                    output: 1.0,
                },
            ],
            ..CurveSet::default()
        };
        assert!(
            curves_image(&fixture(), &curves).get_pixel(7, 6)[0] > fixture().get_pixel(7, 6)[0]
        );
    }

    #[test]
    fn levels_respect_input_and_output_ranges() {
        let source = RgbaImage::from_pixel(1, 1, Rgba([20, 120, 240, 9]));
        let output = levels(&source, 20, 240, 1.0, 10, 210);
        assert_eq!(output.get_pixel(0, 0).0, [10, 101, 210, 9]);
    }

    #[test]
    fn white_and_black_points_are_deterministic() {
        let source = RgbaImage::from_pixel(1, 1, Rgba([100, 120, 200, 55]));
        assert_eq!(
            point_balance(&source, [100, 120, 200], true)
                .get_pixel(0, 0)
                .0,
            [255, 255, 255, 55]
        );
        assert_eq!(
            point_balance(&source, [10, 20, 30], false)
                .get_pixel(0, 0)
                .0,
            [90, 100, 170, 55]
        );
    }

    #[test]
    fn crop_uses_normalized_bounds() {
        let output = crop(&fixture(), 0.25, 0.25, 0.5, 0.5);
        assert_eq!(output.dimensions(), (8, 6));
        assert_eq!(output.get_pixel(0, 0), fixture().get_pixel(4, 3));
    }

    #[test]
    fn zero_straighten_is_identity() {
        assert_eq!(rotate_arbitrary(&fixture(), 0.0), fixture());
    }

    #[test]
    fn identity_perspective_preserves_image() {
        assert_eq!(
            perspective(&fixture(), &PerspectiveCorners::default()),
            fixture()
        );
    }

    #[test]
    fn lens_identity_preserves_image() {
        assert_eq!(lens_correction(&fixture(), 0.0, 0.0, 0.0), fixture());
    }

    #[test]
    fn hsl_identity_preserves_primary_colors() {
        let source = RgbaImage::from_pixel(1, 1, Rgba([255, 0, 0, 222]));
        assert_eq!(hsl(&source, &HslSettings::default()), source);
    }

    #[test]
    fn temperature_warms_red_and_cools_blue() {
        let source = RgbaImage::from_pixel(1, 1, Rgba([100, 100, 100, 255]));
        let output = temperature_tint(&source, 1.0, 0.0);
        assert!(output.get_pixel(0, 0)[0] > output.get_pixel(0, 0)[2]);
    }

    #[test]
    fn selective_color_only_changes_near_target_hue() {
        let source = RgbaImage::from_fn(2, 1, |x, _| {
            if x == 0 {
                Rgba([255, 0, 0, 255])
            } else {
                Rgba([0, 255, 0, 255])
            }
        });
        let output = selective_color(
            &source,
            0.0,
            30.0,
            &SelectiveColorAdjustment {
                cyan: 0.5,
                ..Default::default()
            },
        );
        assert!(output.get_pixel(0, 0)[0] < 255);
        assert_eq!(output.get_pixel(1, 0), source.get_pixel(1, 0));
    }

    #[test]
    fn professional_dispatch_covers_crop() {
        let operation = EditOperation::Crop {
            x: 0.0,
            y: 0.0,
            width: 0.5,
            height: 1.0,
            aspect_ratio: Some("free".into()),
            overlay: CropOverlay::RuleOfThirds,
        };
        assert_eq!(apply(&fixture(), &operation).dimensions(), (8, 12));
    }

    #[test]
    fn rgba_alpha_is_preserved_by_color_operations() {
        let source = fixture();
        for output in [
            levels(&source, 0, 255, 1.2, 0, 255),
            temperature_tint(&source, 0.4, -0.2),
            hsl(&source, &HslSettings::default()),
        ] {
            assert!(output.pixels().all(|pixel| pixel[3] == 173));
        }
    }

    #[test]
    fn rgb_hsl_round_trip_primary_palette() {
        for color in [
            [255, 0, 0],
            [0, 255, 0],
            [0, 0, 255],
            [128, 128, 128],
            [255, 255, 255],
            [0, 0, 0],
        ] {
            let (h, s, l) = rgb_to_hsl(color[0], color[1], color[2]);
            let converted = hsl_to_rgb(h, s, l);
            assert!(converted
                .iter()
                .zip(color)
                .all(|(left, right)| left.abs_diff(right) <= 1));
        }
    }

    #[test]
    fn transforms_are_repeatable() {
        let source = fixture();
        let first = lens_correction(&source, 0.2, 0.1, 0.1);
        let second = lens_correction(&source, 0.2, 0.1, 0.1);
        assert_eq!(first, second);
    }
}
