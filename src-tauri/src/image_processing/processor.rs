use crate::domain::EditOperation;
use crate::error::AppError;
use image::{imageops, DynamicImage, Rgba, RgbaImage};

pub fn apply_pipeline(
    source: &DynamicImage,
    operations: &[EditOperation],
) -> Result<DynamicImage, AppError> {
    let mut current = source.clone();
    for operation in operations {
        operation.validate()?;
        current = apply_operation(&current, operation)?;
    }
    Ok(current)
}

fn apply_operation(
    image: &DynamicImage,
    operation: &EditOperation,
) -> Result<DynamicImage, AppError> {
    let rgba = image.to_rgba8();
    let output = match operation {
        EditOperation::Brightness { amount } => map_rgb(&rgba, |red, green, blue| {
            let offset = amount * 255.0;
            (
                clamp(red as f32 + offset),
                clamp(green as f32 + offset),
                clamp(blue as f32 + offset),
            )
        }),
        EditOperation::Contrast { amount } => {
            let factor = 1.0 + amount;
            map_rgb(&rgba, |red, green, blue| {
                (
                    clamp((red as f32 - 127.5) * factor + 127.5),
                    clamp((green as f32 - 127.5) * factor + 127.5),
                    clamp((blue as f32 - 127.5) * factor + 127.5),
                )
            })
        }
        EditOperation::Saturation { amount } => {
            let factor = 1.0 + amount;
            map_rgb(&rgba, |red, green, blue| {
                let luminance = 0.2126 * red as f32 + 0.7152 * green as f32 + 0.0722 * blue as f32;
                (
                    clamp(luminance + (red as f32 - luminance) * factor),
                    clamp(luminance + (green as f32 - luminance) * factor),
                    clamp(luminance + (blue as f32 - luminance) * factor),
                )
            })
        }
        EditOperation::Gamma { value } => {
            let inverse = 1.0 / value;
            map_rgb(&rgba, |red, green, blue| {
                (
                    clamp(255.0 * (red as f32 / 255.0).powf(inverse)),
                    clamp(255.0 * (green as f32 / 255.0).powf(inverse)),
                    clamp(255.0 * (blue as f32 / 255.0).powf(inverse)),
                )
            })
        }
        EditOperation::Grayscale => map_rgb(&rgba, |red, green, blue| {
            let luminance =
                clamp(0.2126 * red as f32 + 0.7152 * green as f32 + 0.0722 * blue as f32);
            (luminance, luminance, luminance)
        }),
        EditOperation::Sepia => map_rgb(&rgba, |red, green, blue| {
            let red = red as f32;
            let green = green as f32;
            let blue = blue as f32;
            (
                clamp(0.393 * red + 0.769 * green + 0.189 * blue),
                clamp(0.349 * red + 0.686 * green + 0.168 * blue),
                clamp(0.272 * red + 0.534 * green + 0.131 * blue),
            )
        }),
        EditOperation::ReflectHorizontal => imageops::flip_horizontal(&rgba),
        EditOperation::Rotate { degrees } => match degrees.rem_euclid(360) {
            0 => rgba,
            90 => imageops::rotate90(&rgba),
            180 => imageops::rotate180(&rgba),
            270 => imageops::rotate270(&rgba),
            _ => {
                return Err(AppError::InvalidOperation(
                    "rotation must be a multiple of 90 degrees".into(),
                ))
            }
        },
        EditOperation::GaussianBlur { radius } => {
            if *radius == 0.0 {
                rgba
            } else {
                imageops::blur(&rgba, *radius)
            }
        }
        EditOperation::Sharpen { strength } => unsharp_mask(&rgba, *strength),
    };

    Ok(DynamicImage::ImageRgba8(output))
}

fn map_rgb<F>(source: &RgbaImage, mut transform: F) -> RgbaImage
where
    F: FnMut(u8, u8, u8) -> (u8, u8, u8),
{
    let mut output = RgbaImage::new(source.width(), source.height());
    for (x, y, pixel) in source.enumerate_pixels() {
        let (red, green, blue) = transform(pixel[0], pixel[1], pixel[2]);
        output.put_pixel(x, y, Rgba([red, green, blue, pixel[3]]));
    }
    output
}

fn unsharp_mask(source: &RgbaImage, strength: f32) -> RgbaImage {
    if strength == 0.0 {
        return source.clone();
    }

    let blurred = imageops::blur(source, 1.2);
    let mut output = RgbaImage::new(source.width(), source.height());
    for (x, y, original) in source.enumerate_pixels() {
        let soft = blurred.get_pixel(x, y);
        output.put_pixel(
            x,
            y,
            Rgba([
                clamp(original[0] as f32 + (original[0] as f32 - soft[0] as f32) * strength),
                clamp(original[1] as f32 + (original[1] as f32 - soft[1] as f32) * strength),
                clamp(original[2] as f32 + (original[2] as f32 - soft[2] as f32) * strength),
                original[3],
            ]),
        );
    }
    output
}

pub(crate) fn clamp(value: f32) -> u8 {
    value.round().clamp(0.0, 255.0) as u8
}

#[cfg(test)]
mod tests {
    use super::*;

    fn image(width: u32, height: u32, pixels: &[[u8; 4]]) -> DynamicImage {
        let buffer =
            RgbaImage::from_fn(width, height, |x, y| Rgba(pixels[(y * width + x) as usize]));
        DynamicImage::ImageRgba8(buffer)
    }

    #[test]
    fn clamps_pixel_values() {
        assert_eq!(clamp(-50.0), 0);
        assert_eq!(clamp(300.0), 255);
        assert_eq!(clamp(127.6), 128);
    }

    #[test]
    fn adjusts_brightness_and_preserves_alpha() {
        let source = image(1, 1, &[[240, 10, 30, 77]]);
        let result = apply_pipeline(&source, &[EditOperation::Brightness { amount: 0.1 }])
            .unwrap()
            .to_rgba8();
        assert_eq!(result.get_pixel(0, 0).0, [255, 36, 56, 77]);
    }

    #[test]
    fn converts_to_grayscale() {
        let source = image(1, 1, &[[200, 100, 25, 255]]);
        let result = apply_pipeline(&source, &[EditOperation::Grayscale])
            .unwrap()
            .to_rgba8();
        let pixel = result.get_pixel(0, 0);
        assert_eq!(pixel[0], pixel[1]);
        assert_eq!(pixel[1], pixel[2]);
    }

    #[test]
    fn reflects_horizontally() {
        let source = image(2, 1, &[[255, 0, 0, 255], [0, 0, 255, 255]]);
        let result = apply_pipeline(&source, &[EditOperation::ReflectHorizontal])
            .unwrap()
            .to_rgba8();
        assert_eq!(result.get_pixel(0, 0).0, [0, 0, 255, 255]);
        assert_eq!(result.get_pixel(1, 0).0, [255, 0, 0, 255]);
    }

    #[test]
    fn rotates_clockwise() {
        let source = image(2, 1, &[[255, 0, 0, 255], [0, 0, 255, 255]]);
        let result = apply_pipeline(&source, &[EditOperation::Rotate { degrees: 90 }])
            .unwrap()
            .to_rgba8();
        assert_eq!(result.dimensions(), (1, 2));
        assert_eq!(result.get_pixel(0, 0).0, [255, 0, 0, 255]);
        assert_eq!(result.get_pixel(0, 1).0, [0, 0, 255, 255]);
    }

    #[test]
    fn applies_operations_in_order() {
        let source = image(1, 1, &[[20, 60, 100, 255]]);
        let first = apply_pipeline(
            &source,
            &[
                EditOperation::Brightness { amount: 0.1 },
                EditOperation::Contrast { amount: 0.5 },
            ],
        )
        .unwrap();
        let reversed = apply_pipeline(
            &source,
            &[
                EditOperation::Contrast { amount: 0.5 },
                EditOperation::Brightness { amount: 0.1 },
            ],
        )
        .unwrap();
        assert_ne!(first.to_rgba8(), reversed.to_rgba8());
    }
}
