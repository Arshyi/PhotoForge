use super::model::{Layer, LayerKind};
use super::transform::LayerTransform;
use crate::error::AppError;
use crate::mask::MaskBitmap;

/// Bilinear coverage sample at a continuous point in a mask's own pixel space.
fn sample(mask: &MaskBitmap, x: f32, y: f32) -> f32 {
    if !x.is_finite() || !y.is_finite() {
        return 0.0;
    }
    if x < 0.0 || y < 0.0 || x >= mask.width() as f32 || y >= mask.height() as f32 {
        return 0.0;
    }
    let sample_x = (x - 0.5).clamp(0.0, mask.width().saturating_sub(1) as f32);
    let sample_y = (y - 0.5).clamp(0.0, mask.height().saturating_sub(1) as f32);
    let x0 = sample_x.floor() as u32;
    let y0 = sample_y.floor() as u32;
    let x1 = (x0 + 1).min(mask.width() - 1);
    let y1 = (y0 + 1).min(mask.height() - 1);
    let fx = sample_x - x0 as f32;
    let fy = sample_y - y0 as f32;
    let top = f32::from(mask.get(x0, y0)) * (1.0 - fx) + f32::from(mask.get(x1, y0)) * fx;
    let bottom = f32::from(mask.get(x0, y1)) * (1.0 - fx) + f32::from(mask.get(x1, y1)) * fx;
    top * (1.0 - fy) + bottom * fy
}

/// The pixel space a layer's mask lives in: its own buffer for a pixel layer,
/// the canvas for a group or adjustment layer.
pub fn mask_space(layer: &Layer, canvas_width: u32, canvas_height: u32) -> (u32, u32) {
    match layer.kind() {
        LayerKind::Pixel => layer
            .pixel_dimensions()
            .unwrap_or((canvas_width, canvas_height)),
        LayerKind::Group | LayerKind::Adjustment => (canvas_width, canvas_height),
    }
}

/// Projects a canvas-space selection into a layer's own mask space.
///
/// The layer transform is applied forwards, so a mask created from a selection
/// lines up with what the user saw on the canvas, and then continues to travel
/// with the layer when the layer is later moved or scaled.
pub fn selection_to_layer_mask(
    selection: &MaskBitmap,
    layer: &Layer,
    canvas_width: u32,
    canvas_height: u32,
) -> Result<MaskBitmap, AppError> {
    if (selection.width(), selection.height()) != (canvas_width, canvas_height) {
        return Err(AppError::MaskDimensionMismatch {
            mask_width: selection.width(),
            mask_height: selection.height(),
            image_width: canvas_width,
            image_height: canvas_height,
        });
    }
    let (width, height) = mask_space(layer, canvas_width, canvas_height);
    let transform = layer.transform;
    transform.validate()?;
    let mut mask = MaskBitmap::empty(width, height)?;
    for y in 0..height {
        for x in 0..width {
            let (document_x, document_y) =
                transform.forward((x as f32 + 0.5, y as f32 + 0.5), width, height);
            let coverage = sample(selection, document_x, document_y);
            mask.set(x, y, coverage.round().clamp(0.0, 255.0) as u8);
        }
    }
    Ok(mask)
}

/// Projects a layer mask back into a canvas-space selection.
pub fn layer_mask_to_selection(
    mask: &MaskBitmap,
    layer: &Layer,
    canvas_width: u32,
    canvas_height: u32,
) -> Result<MaskBitmap, AppError> {
    let (width, height) = mask_space(layer, canvas_width, canvas_height);
    if (mask.width(), mask.height()) != (width, height) {
        return Err(AppError::MaskDimensionMismatch {
            mask_width: mask.width(),
            mask_height: mask.height(),
            image_width: width,
            image_height: height,
        });
    }
    let transform = layer.transform;
    let inverse = transform.inverse(width, height)?;
    let mut selection = MaskBitmap::empty(canvas_width, canvas_height)?;
    for y in 0..canvas_height {
        for x in 0..canvas_width {
            let (local_x, local_y) = inverse.apply(x as f32 + 0.5, y as f32 + 0.5);
            let coverage = sample(mask, local_x, local_y);
            selection.set(x, y, coverage.round().clamp(0.0, 255.0) as u8);
        }
    }
    Ok(selection)
}

/// Intersects a canvas-space selection with the area a layer actually covers,
/// so an edit confined to a selection cannot spill outside its layer.
pub fn intersect_with_layer_bounds(
    selection: &MaskBitmap,
    layer: &Layer,
    canvas_width: u32,
    canvas_height: u32,
) -> Result<MaskBitmap, AppError> {
    let (width, height) = mask_space(layer, canvas_width, canvas_height);
    let transform: LayerTransform = layer.transform;
    let inverse = transform.inverse(width, height)?;
    let mut result = selection.clone();
    for y in 0..canvas_height.min(selection.height()) {
        for x in 0..canvas_width.min(selection.width()) {
            let (local_x, local_y) = inverse.apply(x as f32 + 0.5, y as f32 + 0.5);
            let inside = local_x >= 0.0
                && local_y >= 0.0
                && local_x < width as f32
                && local_y < height as f32;
            if !inside {
                result.set(x, y, 0);
            }
        }
    }
    Ok(result)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::layers::model::fixtures::*;

    fn selection(width: u32, height: u32) -> MaskBitmap {
        MaskBitmap::empty(width, height).unwrap()
    }

    #[test]
    fn an_untransformed_layer_the_size_of_the_canvas_copies_coverage_exactly() {
        let mut source = selection(4, 4);
        source.set(0, 0, 255);
        source.set(3, 3, 128);
        let layer = pixel_layer("a", 4, 4);
        let mask = selection_to_layer_mask(&source, &layer, 4, 4).unwrap();
        assert_eq!(mask.get(0, 0), 255);
        assert_eq!(mask.get(3, 3), 128);
        assert_eq!(mask.get(1, 1), 0);
    }

    #[test]
    fn conversion_round_trips_for_an_untransformed_layer() {
        let mut source = selection(6, 6);
        for x in 0..6 {
            source.set(x, 2, 200);
        }
        let layer = pixel_layer("a", 6, 6);
        let mask = selection_to_layer_mask(&source, &layer, 6, 6).unwrap();
        let restored = layer_mask_to_selection(&mask, &layer, 6, 6).unwrap();
        assert_eq!(restored.coverage(), source.coverage());
    }

    #[test]
    fn a_translated_layer_receives_the_selection_under_where_it_sits() {
        let mut source = selection(8, 8);
        source.set(5, 5, 255);
        let mut layer = pixel_layer("a", 4, 4);
        layer.transform = LayerTransform {
            translate_x: 4.0,
            translate_y: 4.0,
            ..LayerTransform::default()
        };
        let mask = selection_to_layer_mask(&source, &layer, 8, 8).unwrap();
        assert_eq!((mask.width(), mask.height()), (4, 4));
        // Canvas pixel (5,5) maps to layer pixel (1,1) after a 4px shift.
        assert_eq!(mask.get(1, 1), 255);
        assert_eq!(mask.get(0, 0), 0);
    }

    #[test]
    fn a_translated_layer_mask_maps_back_to_the_canvas() {
        let mut mask = MaskBitmap::empty(4, 4).unwrap();
        mask.set(1, 1, 255);
        let mut layer = pixel_layer("a", 4, 4);
        layer.transform = LayerTransform {
            translate_x: 4.0,
            translate_y: 4.0,
            ..LayerTransform::default()
        };
        let restored = layer_mask_to_selection(&mask, &layer, 8, 8).unwrap();
        assert_eq!(restored.get(5, 5), 255);
        assert_eq!(restored.get(1, 1), 0);
    }

    #[test]
    fn a_group_layer_masks_in_canvas_space() {
        let group = group_layer("g", vec![pixel_layer("child", 2, 2)]);
        assert_eq!(mask_space(&group, 10, 6), (10, 6));
        let mut source = selection(10, 6);
        source.set(9, 5, 255);
        let mask = selection_to_layer_mask(&source, &group, 10, 6).unwrap();
        assert_eq!((mask.width(), mask.height()), (10, 6));
        assert_eq!(mask.get(9, 5), 255);
    }

    #[test]
    fn an_adjustment_layer_masks_in_canvas_space() {
        let layer = adjustment_layer("adj", crate::domain::EditOperation::Sepia);
        assert_eq!(mask_space(&layer, 7, 3), (7, 3));
    }

    #[test]
    fn a_selection_that_does_not_match_the_canvas_is_rejected() {
        let source = selection(4, 4);
        let layer = pixel_layer("a", 4, 4);
        assert!(matches!(
            selection_to_layer_mask(&source, &layer, 8, 8),
            Err(AppError::MaskDimensionMismatch { .. })
        ));
    }

    #[test]
    fn a_layer_mask_that_does_not_match_its_space_is_rejected() {
        let mask = MaskBitmap::empty(3, 3).unwrap();
        let layer = pixel_layer("a", 4, 4);
        assert!(matches!(
            layer_mask_to_selection(&mask, &layer, 4, 4),
            Err(AppError::MaskDimensionMismatch { .. })
        ));
    }

    #[test]
    fn a_singular_transform_is_rejected_rather_than_producing_garbage() {
        let mut layer = pixel_layer("a", 4, 4);
        layer.transform = LayerTransform {
            scale_x: 0.0,
            ..LayerTransform::default()
        };
        let mask = MaskBitmap::empty(4, 4).unwrap();
        assert!(layer_mask_to_selection(&mask, &layer, 4, 4).is_err());
        assert!(selection_to_layer_mask(&mask, &layer, 4, 4).is_err());
    }

    #[test]
    fn intersecting_clears_selection_outside_the_layer() {
        let mut source = MaskBitmap::full(8, 8).unwrap();
        let mut layer = pixel_layer("a", 4, 4);
        layer.transform = LayerTransform {
            translate_x: 2.0,
            translate_y: 2.0,
            ..LayerTransform::default()
        };
        source = intersect_with_layer_bounds(&source, &layer, 8, 8).unwrap();
        assert_eq!(source.get(3, 3), 255);
        assert_eq!(source.get(0, 0), 0);
        assert_eq!(source.get(7, 7), 0);
    }

    #[test]
    fn intersecting_with_a_canvas_sized_layer_changes_nothing() {
        let source = MaskBitmap::full(4, 4).unwrap();
        let layer = pixel_layer("a", 4, 4);
        let result = intersect_with_layer_bounds(&source, &layer, 4, 4).unwrap();
        assert_eq!(result.coverage(), source.coverage());
    }

    #[test]
    fn a_flipped_layer_mirrors_the_selection_it_receives() {
        let mut source = selection(4, 1);
        source.set(0, 0, 255);
        let mut layer = pixel_layer("a", 4, 1);
        layer.transform = LayerTransform {
            flip_horizontal: true,
            ..LayerTransform::default()
        };
        let mask = selection_to_layer_mask(&source, &layer, 4, 1).unwrap();
        // Layer pixel 3 maps forward onto canvas pixel 0.
        assert_eq!(mask.get(3, 0), 255);
        assert_eq!(mask.get(0, 0), 0);
    }
}
