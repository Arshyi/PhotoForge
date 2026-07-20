use crate::domain::{HistogramChannels, PixelInspection};
use image::{DynamicImage, GenericImageView};

pub fn calculate_histogram(image: &DynamicImage) -> HistogramChannels {
    let pixels = image.to_rgba8();
    let mut red = vec![0_u64; 256];
    let mut green = vec![0_u64; 256];
    let mut blue = vec![0_u64; 256];
    let mut luminance = vec![0_u64; 256];
    let mut shadow_clipping = 0_u64;
    let mut highlight_clipping = 0_u64;
    for pixel in pixels.pixels() {
        red[pixel[0] as usize] += 1;
        green[pixel[1] as usize] += 1;
        blue[pixel[2] as usize] += 1;
        let light = (0.2126 * f32::from(pixel[0])
            + 0.7152 * f32::from(pixel[1])
            + 0.0722 * f32::from(pixel[2]))
        .round()
        .clamp(0.0, 255.0) as usize;
        luminance[light] += 1;
        shadow_clipping += u64::from(pixel[0] == 0 || pixel[1] == 0 || pixel[2] == 0);
        highlight_clipping += u64::from(pixel[0] == 255 || pixel[1] == 255 || pixel[2] == 255);
    }
    HistogramChannels {
        red,
        green,
        blue,
        luminance,
        shadow_clipping,
        highlight_clipping,
        pixel_count: u64::from(pixels.width()) * u64::from(pixels.height()),
    }
}

pub fn inspect_pixel(image: &DynamicImage, x: u32, y: u32) -> Option<PixelInspection> {
    if x >= image.width() || y >= image.height() {
        return None;
    }
    let pixel = image.get_pixel(x, y);
    let (hue, saturation, value) = rgb_to_hsv(pixel[0], pixel[1], pixel[2]);
    Some(PixelInspection {
        x,
        y,
        red: pixel[0],
        green: pixel[1],
        blue: pixel[2],
        alpha: pixel[3],
        hue,
        saturation,
        value,
    })
}

fn rgb_to_hsv(red: u8, green: u8, blue: u8) -> (f32, f32, f32) {
    let red = f32::from(red) / 255.0;
    let green = f32::from(green) / 255.0;
    let blue = f32::from(blue) / 255.0;
    let maximum = red.max(green).max(blue);
    let minimum = red.min(green).min(blue);
    let delta = maximum - minimum;
    let hue = if delta <= f32::EPSILON {
        0.0
    } else if maximum == red {
        60.0 * ((green - blue) / delta).rem_euclid(6.0)
    } else if maximum == green {
        60.0 * ((blue - red) / delta + 2.0)
    } else {
        60.0 * ((red - green) / delta + 4.0)
    };
    let saturation = if maximum <= f32::EPSILON {
        0.0
    } else {
        delta / maximum
    };
    (hue, saturation, maximum)
}

#[cfg(test)]
mod tests {
    use super::*;
    use image::{Rgba, RgbaImage};

    #[test]
    fn histogram_counts_every_pixel_per_channel() {
        let image = DynamicImage::ImageRgba8(RgbaImage::from_fn(3, 2, |x, y| {
            Rgba([(x * 20) as u8, (y * 30) as u8, 90, 255])
        }));
        let histogram = calculate_histogram(&image);
        assert_eq!(histogram.pixel_count, 6);
        assert_eq!(histogram.red.iter().sum::<u64>(), 6);
        assert_eq!(histogram.green.iter().sum::<u64>(), 6);
        assert_eq!(histogram.blue.iter().sum::<u64>(), 6);
        assert_eq!(histogram.luminance.iter().sum::<u64>(), 6);
    }

    #[test]
    fn histogram_bins_are_exact() {
        let image = DynamicImage::ImageRgba8(RgbaImage::from_fn(2, 1, |x, _| {
            if x == 0 {
                Rgba([10, 20, 30, 255])
            } else {
                Rgba([10, 40, 250, 255])
            }
        }));
        let histogram = calculate_histogram(&image);
        assert_eq!(histogram.red[10], 2);
        assert_eq!(histogram.green[20], 1);
        assert_eq!(histogram.green[40], 1);
        assert_eq!(histogram.blue[30], 1);
        assert_eq!(histogram.blue[250], 1);
    }

    #[test]
    fn clipping_indicators_count_pixels_once() {
        let image = DynamicImage::ImageRgba8(RgbaImage::from_fn(4, 1, |x, _| match x {
            0 => Rgba([0, 0, 0, 255]),
            1 => Rgba([255, 255, 255, 255]),
            2 => Rgba([0, 255, 100, 255]),
            _ => Rgba([1, 2, 3, 255]),
        }));
        let histogram = calculate_histogram(&image);
        assert_eq!(histogram.shadow_clipping, 2);
        assert_eq!(histogram.highlight_clipping, 2);
    }

    #[test]
    fn luminance_uses_rec_709_weights() {
        let image = DynamicImage::ImageRgba8(RgbaImage::from_pixel(1, 1, Rgba([255, 0, 0, 255])));
        assert_eq!(calculate_histogram(&image).luminance[54], 1);
    }

    #[test]
    fn pixel_inspector_reports_rgb_hsv_and_coordinates() {
        let image = DynamicImage::ImageRgba8(RgbaImage::from_pixel(2, 3, Rgba([255, 0, 0, 99])));
        let sample = inspect_pixel(&image, 1, 2).unwrap();
        assert_eq!((sample.x, sample.y), (1, 2));
        assert_eq!(
            (sample.red, sample.green, sample.blue, sample.alpha),
            (255, 0, 0, 99)
        );
        assert_eq!(
            (sample.hue, sample.saturation, sample.value),
            (0.0, 1.0, 1.0)
        );
    }

    #[test]
    fn pixel_inspector_rejects_out_of_bounds_coordinates() {
        let image = DynamicImage::new_rgba8(2, 2);
        assert!(inspect_pixel(&image, 2, 0).is_none());
        assert!(inspect_pixel(&image, 0, 2).is_none());
    }

    #[test]
    fn hsv_primary_hues_are_correct() {
        for (rgb, expected) in [
            ([255, 0, 0], 0.0),
            ([255, 255, 0], 60.0),
            ([0, 255, 0], 120.0),
            ([0, 255, 255], 180.0),
            ([0, 0, 255], 240.0),
            ([255, 0, 255], 300.0),
        ] {
            assert!((rgb_to_hsv(rgb[0], rgb[1], rgb[2]).0 - expected).abs() < 0.001);
        }
    }

    #[test]
    fn transparent_pixels_are_still_inspectable() {
        let image = DynamicImage::ImageRgba8(RgbaImage::from_pixel(1, 1, Rgba([7, 8, 9, 0])));
        assert_eq!(inspect_pixel(&image, 0, 0).unwrap().alpha, 0);
    }
}
