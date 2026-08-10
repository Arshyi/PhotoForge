use crate::domain::EditOperation;
use crate::error::AppError;
use image::{imageops, DynamicImage, Rgba, RgbaImage};

use super::professional;
use super::restoration;

pub fn apply_pipeline(
    source: &DynamicImage,
    operations: &[EditOperation],
) -> Result<DynamicImage, AppError> {
    if operations.is_empty() {
        return Ok(source.clone());
    }

    let mut current = source.to_rgba8();
    for operation in operations {
        operation.validate()?;
        current = apply_operation(&current, operation)?;
    }
    Ok(DynamicImage::ImageRgba8(current))
}

fn apply_operation(image: &RgbaImage, operation: &EditOperation) -> Result<RgbaImage, AppError> {
    let output = match operation {
        EditOperation::Brightness { amount } => map_rgb(image, |red, green, blue| {
            let offset = amount * 255.0;
            (
                clamp(red as f32 + offset),
                clamp(green as f32 + offset),
                clamp(blue as f32 + offset),
            )
        }),
        EditOperation::Contrast { amount } => {
            let factor = 1.0 + amount;
            map_rgb(image, |red, green, blue| {
                (
                    clamp((red as f32 - 127.5) * factor + 127.5),
                    clamp((green as f32 - 127.5) * factor + 127.5),
                    clamp((blue as f32 - 127.5) * factor + 127.5),
                )
            })
        }
        EditOperation::Saturation { amount } => {
            let factor = 1.0 + amount;
            map_rgb(image, |red, green, blue| {
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
            map_rgb(image, |red, green, blue| {
                (
                    clamp(255.0 * (red as f32 / 255.0).powf(inverse)),
                    clamp(255.0 * (green as f32 / 255.0).powf(inverse)),
                    clamp(255.0 * (blue as f32 / 255.0).powf(inverse)),
                )
            })
        }
        EditOperation::Grayscale => map_rgb(image, |red, green, blue| {
            let luminance =
                clamp(0.2126 * red as f32 + 0.7152 * green as f32 + 0.0722 * blue as f32);
            (luminance, luminance, luminance)
        }),
        EditOperation::Sepia => map_rgb(image, |red, green, blue| {
            let red = red as f32;
            let green = green as f32;
            let blue = blue as f32;
            (
                clamp(0.393 * red + 0.769 * green + 0.189 * blue),
                clamp(0.349 * red + 0.686 * green + 0.168 * blue),
                clamp(0.272 * red + 0.534 * green + 0.131 * blue),
            )
        }),
        EditOperation::ReflectHorizontal => imageops::flip_horizontal(image),
        EditOperation::Rotate { degrees } => match degrees.rem_euclid(360) {
            0 => image.clone(),
            90 => imageops::rotate90(image),
            180 => imageops::rotate180(image),
            270 => imageops::rotate270(image),
            _ => {
                return Err(AppError::InvalidOperation(
                    "rotation must be a multiple of 90 degrees".into(),
                ))
            }
        },
        EditOperation::GaussianBlur { radius } => {
            if *radius == 0.0 {
                image.clone()
            } else {
                imageops::blur(image, *radius)
            }
        }
        EditOperation::Sharpen { strength } => unsharp_mask(image, *strength),
        EditOperation::AutoWhiteBalance { strength } => {
            restoration::auto_white_balance(image, *strength)
        }
        EditOperation::LocalContrast {
            strength,
            tile_size,
            clip_limit,
        } => restoration::local_contrast(image, *strength, *tile_size, *clip_limit),
        EditOperation::Denoise {
            strength,
            preserve_edges,
        } => restoration::denoise(image, *strength, *preserve_edges),
        EditOperation::Deblock { strength } => restoration::deblock(image, *strength),
        EditOperation::EdgeAwareSharpen {
            strength,
            radius,
            threshold,
        } => restoration::edge_aware_sharpen(image, *strength, *radius, *threshold),
        EditOperation::MildDeblur { strength, radius } => {
            restoration::mild_deblur(image, *strength, *radius)
        }
        EditOperation::DocumentEnhance {
            strength,
            grayscale,
        } => restoration::document_enhance(image, *strength, *grayscale),
        EditOperation::UnevenLightingCorrection { strength, radius } => {
            restoration::uneven_lighting(image, *strength, *radius)
        }
        EditOperation::Masked {
            operation,
            mask,
            invert,
            ..
        } => {
            let decoded = mask.decode()?;
            if (decoded.width(), decoded.height()) != image.dimensions() {
                return Err(AppError::MaskDimensionMismatch {
                    mask_width: decoded.width(),
                    mask_height: decoded.height(),
                    image_width: image.width(),
                    image_height: image.height(),
                });
            }
            let adjusted = apply_operation(image, operation)?;
            blend_masked(image, &adjusted, &decoded, *invert)?
        }
        EditOperation::Curves { .. }
        | EditOperation::Levels { .. }
        | EditOperation::WhitePoint { .. }
        | EditOperation::BlackPoint { .. }
        | EditOperation::Crop { .. }
        | EditOperation::Straighten { .. }
        | EditOperation::Perspective { .. }
        | EditOperation::LensCorrection { .. }
        | EditOperation::Hsl { .. }
        | EditOperation::TemperatureTint { .. }
        | EditOperation::SelectiveColor { .. } => professional::apply(image, operation),
    };

    Ok(output)
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

fn blend_masked(
    source: &RgbaImage,
    adjusted: &RgbaImage,
    mask: &crate::mask::MaskBitmap,
    invert: bool,
) -> Result<RgbaImage, AppError> {
    if source.dimensions() != adjusted.dimensions()
        || source.dimensions() != (mask.width(), mask.height())
    {
        return Err(AppError::InvalidOperation(
            "masked adjustments must preserve the image dimensions".into(),
        ));
    }
    let mut output = RgbaImage::new(source.width(), source.height());
    for (index, ((x, y, original), changed)) in
        source.enumerate_pixels().zip(adjusted.pixels()).enumerate()
    {
        let coverage = if invert {
            255 - mask.coverage()[index]
        } else {
            mask.coverage()[index]
        };
        let inverse = 255_u16 - u16::from(coverage);
        let blend = |before: u8, after: u8| {
            ((u16::from(before) * inverse + u16::from(after) * u16::from(coverage) + 127) / 255)
                as u8
        };
        output.put_pixel(
            x,
            y,
            Rgba([
                blend(original[0], changed[0]),
                blend(original[1], changed[1]),
                blend(original[2], changed[2]),
                original[3],
            ]),
        );
    }
    Ok(output)
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

    #[test]
    fn neutral_operations_are_pixel_exact() {
        let source = image(2, 1, &[[12, 34, 56, 78], [210, 180, 140, 99]]);
        for operation in [
            EditOperation::Brightness { amount: 0.0 },
            EditOperation::Contrast { amount: 0.0 },
            EditOperation::Saturation { amount: 0.0 },
            EditOperation::Gamma { value: 1.0 },
            EditOperation::GaussianBlur { radius: 0.0 },
            EditOperation::Sharpen { strength: 0.0 },
            EditOperation::AutoWhiteBalance { strength: 0.0 },
            EditOperation::LocalContrast {
                strength: 0.0,
                tile_size: 8,
                clip_limit: 0.5,
            },
            EditOperation::Denoise {
                strength: 0.0,
                preserve_edges: 0.0,
            },
            EditOperation::Deblock { strength: 0.0 },
            EditOperation::EdgeAwareSharpen {
                strength: 0.0,
                radius: 0.5,
                threshold: 0.0,
            },
            EditOperation::MildDeblur {
                strength: 0.0,
                radius: 0.5,
            },
            EditOperation::DocumentEnhance {
                strength: 0.0,
                grayscale: true,
            },
            EditOperation::UnevenLightingCorrection {
                strength: 0.0,
                radius: 4.0,
            },
        ] {
            assert_eq!(
                apply_pipeline(&source, &[operation]).unwrap().to_rgba8(),
                source.to_rgba8()
            );
        }
    }

    #[test]
    fn saturation_minimum_matches_grayscale_luminance() {
        let source = image(1, 1, &[[200, 100, 25, 41]]);
        let desaturated = apply_pipeline(&source, &[EditOperation::Saturation { amount: -1.0 }])
            .unwrap()
            .to_rgba8();
        let grayscale = apply_pipeline(&source, &[EditOperation::Grayscale])
            .unwrap()
            .to_rgba8();
        assert_eq!(desaturated, grayscale);
        assert_eq!(desaturated.get_pixel(0, 0)[3], 41);
    }

    #[test]
    fn sepia_uses_documented_coefficients_and_preserves_alpha() {
        let source = image(1, 1, &[[100, 150, 200, 37]]);
        let result = apply_pipeline(&source, &[EditOperation::Sepia])
            .unwrap()
            .to_rgba8();
        assert_eq!(result.get_pixel(0, 0).0, [192, 171, 134, 37]);
    }

    #[test]
    fn sharpening_preserves_alpha() {
        let source = image(3, 1, &[[0, 0, 0, 10], [255, 128, 64, 20], [0, 0, 0, 30]]);
        let result = apply_pipeline(&source, &[EditOperation::Sharpen { strength: 1.5 }])
            .unwrap()
            .to_rgba8();
        assert_eq!(
            result.pixels().map(|pixel| pixel[3]).collect::<Vec<_>>(),
            [10, 20, 30]
        );
    }

    #[test]
    fn validates_non_finite_parameters() {
        let source = image(1, 1, &[[0, 0, 0, 255]]);
        assert!(
            apply_pipeline(&source, &[EditOperation::Brightness { amount: f32::NAN }]).is_err()
        );
        assert!(apply_pipeline(
            &source,
            &[EditOperation::GaussianBlur {
                radius: f32::INFINITY,
            }]
        )
        .is_err());
    }

    #[test]
    fn transform_then_pixel_operation_keeps_rotated_dimensions() {
        let source = image(2, 1, &[[10, 20, 30, 1], [40, 50, 60, 2]]);
        let result = apply_pipeline(
            &source,
            &[
                EditOperation::Rotate { degrees: 270 },
                EditOperation::Brightness { amount: 0.1 },
            ],
        )
        .unwrap()
        .to_rgba8();
        assert_eq!(result.dimensions(), (1, 2));
        assert_eq!(result.get_pixel(0, 0).0, [66, 76, 86, 2]);
        assert_eq!(result.get_pixel(0, 1).0, [36, 46, 56, 1]);
    }

    #[test]
    fn restoration_operations_participate_in_pipeline_ordering() {
        let source = image(8, 8, &vec![[160, 100, 70, 255]; 64]);
        let first = apply_pipeline(
            &source,
            &[
                EditOperation::AutoWhiteBalance { strength: 1.0 },
                EditOperation::Brightness { amount: 0.1 },
            ],
        )
        .unwrap();
        let reversed = apply_pipeline(
            &source,
            &[
                EditOperation::Brightness { amount: 0.1 },
                EditOperation::AutoWhiteBalance { strength: 1.0 },
            ],
        )
        .unwrap();
        assert_ne!(first.to_rgba8(), reversed.to_rgba8());
    }

    #[test]
    fn rejects_invalid_restoration_parameters_and_non_finite_values() {
        let source = image(1, 1, &[[10, 20, 30, 255]]);
        for operation in [
            EditOperation::AutoWhiteBalance { strength: f32::NAN },
            EditOperation::LocalContrast {
                strength: 0.5,
                tile_size: 1,
                clip_limit: 1.0,
            },
            EditOperation::Denoise {
                strength: f32::INFINITY,
                preserve_edges: 0.5,
            },
            EditOperation::EdgeAwareSharpen {
                strength: 1.0,
                radius: 99.0,
                threshold: 0.0,
            },
            EditOperation::MildDeblur {
                strength: 0.5,
                radius: 0.0,
            },
            EditOperation::UnevenLightingCorrection {
                strength: 0.5,
                radius: f32::INFINITY,
            },
        ] {
            assert!(apply_pipeline(&source, &[operation]).is_err());
        }
    }

    #[test]
    fn restoration_pipeline_is_deterministic_for_export_consistency() {
        let source = image(
            4,
            3,
            &[
                [20, 40, 80, 10],
                [60, 90, 120, 20],
                [140, 100, 60, 30],
                [200, 180, 160, 40],
                [22, 44, 82, 50],
                [62, 92, 122, 60],
                [142, 102, 62, 70],
                [202, 182, 162, 80],
                [24, 48, 84, 90],
                [64, 94, 124, 100],
                [144, 104, 64, 110],
                [204, 184, 164, 120],
            ],
        );
        let operations = [
            EditOperation::AutoWhiteBalance { strength: 0.4 },
            EditOperation::LocalContrast {
                strength: 0.3,
                tile_size: 16,
                clip_limit: 1.2,
            },
            EditOperation::EdgeAwareSharpen {
                strength: 0.25,
                radius: 1.0,
                threshold: 0.04,
            },
        ];
        let preview = apply_pipeline(&source, &operations).unwrap().to_rgba8();
        let export = apply_pipeline(&source, &operations).unwrap().to_rgba8();
        assert_eq!(preview, export);
    }

    #[test]
    fn masked_brightness_blends_partial_coverage_and_preserves_alpha() {
        let source = image(
            3,
            1,
            &[[10, 20, 30, 10], [10, 20, 30, 20], [10, 20, 30, 30]],
        );
        let mask = crate::mask::MaskBitmap::from_coverage(3, 1, vec![0, 128, 255]).unwrap();
        let result = apply_pipeline(
            &source,
            &[EditOperation::Masked {
                operation: Box::new(EditOperation::Brightness { amount: 0.2 }),
                mask: crate::mask::MaskSnapshot::encode(&mask),
                invert: false,
                mask_id: Some("subject".into()),
            }],
        )
        .unwrap()
        .to_rgba8();
        assert_eq!(result.get_pixel(0, 0).0, [10, 20, 30, 10]);
        assert_eq!(result.get_pixel(1, 0).0, [36, 46, 56, 20]);
        assert_eq!(result.get_pixel(2, 0).0, [61, 71, 81, 30]);
    }

    #[test]
    fn outside_mask_application_inverts_coverage() {
        let source = image(2, 1, &[[10, 20, 30, 255], [10, 20, 30, 255]]);
        let mask = crate::mask::MaskBitmap::from_coverage(2, 1, vec![255, 0]).unwrap();
        let result = apply_pipeline(
            &source,
            &[EditOperation::Masked {
                operation: Box::new(EditOperation::Contrast { amount: 0.5 }),
                mask: crate::mask::MaskSnapshot::encode(&mask),
                invert: true,
                mask_id: None,
            }],
        )
        .unwrap()
        .to_rgba8();
        assert_eq!(result.get_pixel(0, 0), source.to_rgba8().get_pixel(0, 0));
        assert_ne!(result.get_pixel(1, 0), source.to_rgba8().get_pixel(1, 0));
    }

    #[test]
    fn geometric_edits_cannot_be_wrapped_in_masks() {
        let mask = crate::mask::MaskSnapshot::encode(&crate::mask::MaskBitmap::full(1, 1).unwrap());
        let operation = EditOperation::Masked {
            operation: Box::new(EditOperation::Rotate { degrees: 90 }),
            mask,
            invert: false,
            mask_id: None,
        };
        assert!(operation.validate().is_err());
    }

    #[test]
    fn rejects_stale_same_aspect_mask_after_crop() {
        let source = image(8, 4, &[[10, 20, 30, 255]; 32]);
        let stale_mask =
            crate::mask::MaskSnapshot::encode(&crate::mask::MaskBitmap::full(8, 4).unwrap());
        let result = apply_pipeline(
            &source,
            &[
                EditOperation::Crop {
                    x: 0.0,
                    y: 0.0,
                    width: 0.5,
                    height: 0.5,
                    aspect_ratio: None,
                    overlay: crate::domain::CropOverlay::None,
                },
                EditOperation::Masked {
                    operation: Box::new(EditOperation::Brightness { amount: 0.2 }),
                    mask: stale_mask,
                    invert: false,
                    mask_id: None,
                },
            ],
        );

        assert!(matches!(
            result,
            Err(AppError::MaskDimensionMismatch {
                mask_width: 8,
                mask_height: 4,
                image_width: 4,
                image_height: 2,
            })
        ));
    }

    #[test]
    fn applies_valid_crop_then_exact_stage_mask() {
        let source = image(4, 2, &[[10, 20, 30, 255]; 8]);
        let mask = crate::mask::MaskBitmap::from_coverage(2, 2, vec![255, 0, 0, 255]).unwrap();
        let result = apply_pipeline(
            &source,
            &[
                EditOperation::Crop {
                    x: 0.0,
                    y: 0.0,
                    width: 0.5,
                    height: 1.0,
                    aspect_ratio: None,
                    overlay: crate::domain::CropOverlay::None,
                },
                EditOperation::Masked {
                    operation: Box::new(EditOperation::Brightness { amount: 0.2 }),
                    mask: crate::mask::MaskSnapshot::encode(&mask),
                    invert: false,
                    mask_id: Some("cropped-subject".into()),
                },
            ],
        )
        .unwrap()
        .to_rgba8();

        assert_eq!(result.dimensions(), (2, 2));
        assert_eq!(result.get_pixel(0, 0).0, [61, 71, 81, 255]);
        assert_eq!(result.get_pixel(1, 0).0, [10, 20, 30, 255]);
        assert_eq!(result.get_pixel(0, 1).0, [10, 20, 30, 255]);
        assert_eq!(result.get_pixel(1, 1).0, [61, 71, 81, 255]);
    }
}
