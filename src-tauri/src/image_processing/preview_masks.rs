use crate::domain::EditOperation;
use crate::error::AppError;
use crate::mask::{MaskBitmap, MaskSnapshot};

type Dimensions = (u32, u32);

/// Builds a temporary operation list for preview rendering without changing the
/// full-resolution mask snapshots stored in the edit pipeline.
pub(crate) fn prepare_preview_operations(
    operations: &[EditOperation],
    full_source: Dimensions,
    preview_source: Dimensions,
) -> Result<Vec<EditOperation>, AppError> {
    validate_source_dimensions(full_source)?;
    validate_source_dimensions(preview_source)?;

    let mut full_stage = full_source;
    let mut preview_stage = preview_source;
    let mut prepared = Vec::with_capacity(operations.len());

    for operation in operations {
        operation.validate()?;
        let preview_operation = match operation {
            EditOperation::Masked {
                operation: inner,
                mask,
                invert,
                mask_id,
            } => {
                let decoded = mask.decode()?;
                if (decoded.width(), decoded.height()) != full_stage {
                    return Err(AppError::MaskDimensionMismatch {
                        mask_width: decoded.width(),
                        mask_height: decoded.height(),
                        image_width: full_stage.0,
                        image_height: full_stage.1,
                    });
                }
                let preview_mask =
                    resize_preview_coverage(&decoded, preview_stage.0, preview_stage.1)?;
                EditOperation::Masked {
                    operation: inner.clone(),
                    mask: MaskSnapshot::encode(&preview_mask),
                    invert: *invert,
                    mask_id: mask_id.clone(),
                }
            }
            _ => operation.clone(),
        };
        prepared.push(preview_operation);

        full_stage = dimensions_after(operation, full_stage)?;
        preview_stage = dimensions_after(operation, preview_stage)?;
    }

    Ok(prepared)
}

fn validate_source_dimensions(dimensions: Dimensions) -> Result<(), AppError> {
    if dimensions.0 == 0 || dimensions.1 == 0 {
        return Err(AppError::InvalidOperation(
            "image stage dimensions must be greater than zero".into(),
        ));
    }
    Ok(())
}

fn dimensions_after(
    operation: &EditOperation,
    dimensions: Dimensions,
) -> Result<Dimensions, AppError> {
    match operation {
        EditOperation::Rotate { degrees } if matches!(degrees.rem_euclid(360), 90 | 270) => {
            Ok((dimensions.1, dimensions.0))
        }
        EditOperation::Crop {
            x,
            y,
            width,
            height,
            ..
        } => crop_dimensions(dimensions, *x, *y, *width, *height),
        _ => Ok(dimensions),
    }
}

fn crop_dimensions(
    dimensions: Dimensions,
    x: f32,
    y: f32,
    width: f32,
    height: f32,
) -> Result<Dimensions, AppError> {
    let left = (x * dimensions.0 as f32).floor() as u32;
    let top = (y * dimensions.1 as f32).floor() as u32;
    if left >= dimensions.0 || top >= dimensions.1 {
        return Err(AppError::InvalidOperation(
            "crop produces an empty image stage".into(),
        ));
    }

    let output_width = ((width * dimensions.0 as f32).round() as u32)
        .max(1)
        .min(dimensions.0 - left);
    let output_height = ((height * dimensions.1 as f32).round() as u32)
        .max(1)
        .min(dimensions.1 - top);
    Ok((output_width, output_height))
}

/// Bilinear coverage resizing is intentionally private to preview preparation.
/// Unlike general mask resampling, preview dimensions may have a slightly
/// different aspect ratio because thumbnail and crop dimensions round separately.
fn resize_preview_coverage(
    source: &MaskBitmap,
    width: u32,
    height: u32,
) -> Result<MaskBitmap, AppError> {
    if source.width() == width && source.height() == height {
        return Ok(source.clone());
    }

    let mut output = MaskBitmap::empty(width, height)?;
    let scale_x = source.width() as f64 / width as f64;
    let scale_y = source.height() as f64 / height as f64;
    for y in 0..height {
        let source_y = ((y as f64 + 0.5) * scale_y - 0.5).clamp(0.0, source.height() as f64 - 1.0);
        let y0 = source_y.floor() as u32;
        let y1 = (y0 + 1).min(source.height() - 1);
        let fy = source_y - y0 as f64;
        for x in 0..width {
            let source_x =
                ((x as f64 + 0.5) * scale_x - 0.5).clamp(0.0, source.width() as f64 - 1.0);
            let x0 = source_x.floor() as u32;
            let x1 = (x0 + 1).min(source.width() - 1);
            let fx = source_x - x0 as f64;
            let top = source.get(x0, y0) as f64 * (1.0 - fx) + source.get(x1, y0) as f64 * fx;
            let bottom = source.get(x0, y1) as f64 * (1.0 - fx) + source.get(x1, y1) as f64 * fx;
            output.set(x, y, (top * (1.0 - fy) + bottom * fy).round() as u8);
        }
    }
    Ok(output)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::CropOverlay;

    fn masked(mask: MaskBitmap) -> EditOperation {
        EditOperation::Masked {
            operation: Box::new(EditOperation::Brightness { amount: 0.2 }),
            mask: MaskSnapshot::encode(&mask),
            invert: false,
            mask_id: Some("subject".into()),
        }
    }

    fn prepared_mask(operation: &EditOperation) -> MaskBitmap {
        match operation {
            EditOperation::Masked { mask, .. } => mask.decode().unwrap(),
            _ => panic!("expected a masked operation"),
        }
    }

    #[test]
    fn prepares_preview_mask_despite_rounding_aspect_change() {
        let coverage = (0..35).map(|value| value * 7).collect();
        let operations = vec![masked(MaskBitmap::from_coverage(7, 5, coverage).unwrap())];
        let original = operations.clone();

        let prepared = prepare_preview_operations(&operations, (7, 5), (4, 3)).unwrap();
        let preview_mask = prepared_mask(&prepared[0]);

        assert_eq!((preview_mask.width(), preview_mask.height()), (4, 3));
        assert_eq!(operations, original);
        match &operations[0] {
            EditOperation::Masked { mask, .. } => assert_eq!((mask.width, mask.height), (7, 5)),
            _ => unreachable!(),
        }
    }

    #[test]
    fn prepares_decontamination_for_preview_without_mutating_full_mask_or_settings() {
        let full_mask =
            MaskBitmap::from_coverage(4, 2, vec![255, 255, 128, 0, 255, 255, 128, 0]).unwrap();
        let operation = EditOperation::Masked {
            operation: Box::new(EditOperation::DecontaminateColors {
                enabled: true,
                strength: 0.75,
                radius: 3,
            }),
            mask: MaskSnapshot::encode(&full_mask),
            invert: true,
            mask_id: Some("edge-cleanup".into()),
        };

        let prepared =
            prepare_preview_operations(std::slice::from_ref(&operation), (4, 2), (2, 1)).unwrap();
        let preview_mask = prepared_mask(&prepared[0]);
        assert_eq!((preview_mask.width(), preview_mask.height()), (2, 1));
        assert_eq!(prepared_mask(&operation), full_mask);
        assert!(matches!(
            &prepared[0],
            EditOperation::Masked {
                operation,
                invert: true,
                mask_id: Some(mask_id),
                ..
            } if mask_id == "edge-cleanup"
                && matches!(
                    operation.as_ref(),
                    EditOperation::DecontaminateColors {
                        enabled: true,
                        strength: 0.75,
                        radius: 3,
                    }
                )
        ));
    }

    #[test]
    fn tracks_rotate_and_crop_stages_for_full_and_preview_images() {
        let operations = vec![
            EditOperation::Rotate { degrees: 90 },
            EditOperation::Crop {
                x: 0.0,
                y: 0.0,
                width: 0.5,
                height: 0.5,
                aspect_ratio: None,
                overlay: CropOverlay::RuleOfThirds,
            },
            masked(MaskBitmap::full(3, 4).unwrap()),
        ];

        let prepared = prepare_preview_operations(&operations, (8, 6), (4, 3)).unwrap();

        assert_eq!(&prepared[..2], &operations[..2]);
        let preview_mask = prepared_mask(&prepared[2]);
        assert_eq!((preview_mask.width(), preview_mask.height()), (2, 2));
    }

    #[test]
    fn rejects_stale_mask_at_tracked_full_resolution_stage() {
        let operations = vec![
            EditOperation::Crop {
                x: 0.0,
                y: 0.0,
                width: 0.5,
                height: 0.5,
                aspect_ratio: None,
                overlay: CropOverlay::None,
            },
            masked(MaskBitmap::full(8, 4).unwrap()),
        ];

        assert!(matches!(
            prepare_preview_operations(&operations, (8, 4), (4, 2)),
            Err(AppError::MaskDimensionMismatch {
                mask_width: 8,
                mask_height: 4,
                image_width: 4,
                image_height: 2,
            })
        ));
    }

    #[test]
    fn rejects_corrupted_snapshot_before_preview_resize() {
        let mut operation = masked(MaskBitmap::full(4, 3).unwrap());
        if let EditOperation::Masked { mask, .. } = &mut operation {
            mask.checksum = "corrupted".into();
        }

        assert!(matches!(
            prepare_preview_operations(&[operation], (4, 3), (2, 2)),
            Err(AppError::InvalidMask(_))
        ));
    }

    #[test]
    fn preview_resize_uses_bilinear_coverage_without_aspect_gate() {
        let source = MaskBitmap::from_coverage(2, 2, vec![0, 255, 255, 0]).unwrap();
        let resized = resize_preview_coverage(&source, 4, 3).unwrap();

        assert_eq!((resized.width(), resized.height()), (4, 3));
        assert_eq!(resized.get(0, 0), 0);
        assert_eq!(resized.get(3, 0), 255);
        assert_eq!(resized.get(0, 2), 255);
        assert_eq!(resized.get(3, 2), 0);
        assert!(resized.get(1, 1) > 0 && resized.get(1, 1) < 255);
    }
}
