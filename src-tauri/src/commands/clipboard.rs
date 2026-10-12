//! Copying a picture to the system clipboard and pasting one from it.
//!
//! The pictures are 8-bit sRGB with straight alpha, which is what the clipboard carries; a
//! high-precision document is rendered and encoded for it (highlights beyond white are clipped
//! there and nowhere else, and the result says so).
use super::layers::{find_layer, resolve_pixels, LayerPixelsResult};
use crate::application::AppState;
use crate::domain::EditOperation;
use crate::error::AppError;
use crate::image_processing::high_precision::pipeline_typed;
use crate::layers::{render_document_typed, LayerDocument, RenderOptions};
use crate::mask::MaskSnapshot;
use crate::pixel::PixelBuffer;
use image::RgbaImage;
use serde::Serialize;
use std::time::Instant;
use tauri::State;

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ClipboardCopyResult {
    pub width: u32,
    pub height: u32,
    /// "layer", or "merged" for the visible picture with the document's edits.
    pub scope: &'static str,
    /// Whether a selection limited what was copied.
    pub selection: bool,
    /// The forms put on the clipboard.
    pub formats: Vec<&'static str>,
    pub png_bytes: usize,
    /// Highlights beyond white in a high-precision document are clipped for the clipboard.
    pub clipped_highlights: bool,
    pub processing_time_ms: f64,
}

fn clipboard_error(error: crate::clipboard::ClipboardError) -> AppError {
    AppError::Clipboard(error.to_string())
}

/// The bounding box of the pixels with any coverage, as (left, top, width, height).
fn coverage_bounds(image: &RgbaImage, coverage: Option<&[u8]>) -> Option<(u32, u32, u32, u32)> {
    let (width, height) = image.dimensions();
    let (mut left, mut top, mut right, mut bottom) = (width, height, 0_u32, 0_u32);
    for y in 0..height {
        for x in 0..width {
            let index = (y * width + x) as usize;
            let covered = match coverage {
                Some(mask) => mask[index] > 0,
                None => image.get_pixel(x, y).0[3] > 0,
            };
            if covered {
                left = left.min(x);
                top = top.min(y);
                right = right.max(x + 1);
                bottom = bottom.max(y + 1);
            }
        }
    }
    (right > left && bottom > top).then(|| (left, top, right - left, bottom - top))
}

/// Cuts `image` down to what is to be copied: inside the selection if there is one (pixels
/// outside it are transparent, partly selected ones partly so), otherwise the picture as it is
/// (a layer is trimmed to where it has anything).
pub(crate) fn picture_to_copy(
    mut image: RgbaImage,
    selection: Option<&crate::mask::MaskBitmap>,
    trim_to_content: bool,
) -> Result<RgbaImage, AppError> {
    if let Some(mask) = selection {
        let (width, height) = image.dimensions();
        if mask.width() != width || mask.height() != height {
            return Err(AppError::MaskDimensionMismatch {
                mask_width: mask.width(),
                mask_height: mask.height(),
                image_width: width,
                image_height: height,
            });
        }
        let coverage = mask.coverage();
        for (pixel, cover) in image.pixels_mut().zip(coverage) {
            let alpha = u32::from(pixel.0[3]) * u32::from(*cover);
            pixel.0[3] = ((alpha + 127) / 255) as u8;
        }
        let bounds = coverage_bounds(&image, Some(coverage)).ok_or_else(|| {
            AppError::Clipboard("The selection is empty, so there is nothing to copy.".into())
        })?;
        return Ok(
            image::imageops::crop_imm(&image, bounds.0, bounds.1, bounds.2, bounds.3).to_image(),
        );
    }
    if trim_to_content {
        let bounds = coverage_bounds(&image, None)
            .ok_or_else(|| AppError::Clipboard("This layer has nothing visible to copy.".into()))?;
        return Ok(
            image::imageops::crop_imm(&image, bounds.0, bounds.1, bounds.2, bounds.3).to_image(),
        );
    }
    Ok(image)
}

/// Copies the active layer (`layer_id`), or the whole visible picture with the document's edits,
/// to the clipboard. A selection, in the space of what is rendered, limits it.
///
/// The document is not changed. A layer is copied as it looks on the canvas (its transform, mask
/// and adjustments applied; the groups around it are not).
#[tauri::command]
pub async fn copy_to_clipboard(
    document: LayerDocument,
    operations: Vec<EditOperation>,
    layer_id: Option<String>,
    selection: Option<MaskSnapshot>,
    state: State<'_, AppState>,
) -> Result<ClipboardCopyResult, AppError> {
    document.validate()?;
    for operation in &operations {
        operation.validate()?;
    }
    // A picture made without a plug-in the document needs would be a different picture.
    let plugins = crate::plugins::store::global();
    crate::plugins::document::ensure_available(&plugins, &document)?;
    crate::plugins::document::ensure_operations_available(&plugins, &operations)?;
    let decoded_selection = match &selection {
        Some(snapshot) => Some(snapshot.decode()?),
        None => None,
    };
    let (resolved, _) = resolve_pixels(&state, &document, false)?;
    let started = Instant::now();
    let scope = if layer_id.is_some() {
        "layer"
    } else {
        "merged"
    };
    let (written, clipped) = tauri::async_runtime::spawn_blocking(move || {
        let _job = crate::resources::acquire_job(None)?;
        let buffer = match &layer_id {
            Some(id) => {
                let layer = find_layer(&document, id)?;
                let mut alone = document.clone();
                alone.layers = vec![layer];
                alone.active_layer_id = None;
                render_document_typed(&alone, &resolved, RenderOptions::default())?
            }
            None => {
                let rendered =
                    render_document_typed(&document, &resolved, RenderOptions::default())?;
                pipeline_typed(rendered, &operations, None)?
            }
        };
        let clipped = match &buffer {
            PixelBuffer::EncodedSrgba8(_) => false,
            PixelBuffer::LinearRgbaF32(image) => image
                .pixels()
                .iter()
                .any(|p| p.red > 1.0 || p.green > 1.0 || p.blue > 1.0),
        };
        let image = buffer.encoded8().as_ref().clone();
        let picture = picture_to_copy(image, decoded_selection.as_ref(), layer_id.is_some())?;
        let written = crate::clipboard::write_image(&picture).map_err(clipboard_error)?;
        Ok::<_, AppError>((written, clipped))
    })
    .await
    .map_err(|_| AppError::ProcessingFailure("clipboard worker stopped".into()))??;
    Ok(ClipboardCopyResult {
        width: written.width,
        height: written.height,
        scope,
        selection: selection.is_some(),
        formats: written.formats,
        png_bytes: written.png_bytes,
        clipped_highlights: clipped,
        processing_time_ms: started.elapsed().as_secs_f64() * 1_000.0,
    })
}

/// What a paste put in the session: a pixel buffer for a new layer, and the form it came in.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ClipboardPasteResult {
    #[serde(flatten)]
    pub pixels: LayerPixelsResult,
    pub format: crate::clipboard::ClipboardFormat,
}

/// Reads the picture on the clipboard into a new pixel buffer in the session, for a layer.
#[tauri::command]
pub async fn paste_clipboard_image(
    state: State<'_, AppState>,
) -> Result<ClipboardPasteResult, AppError> {
    let limit = crate::resources::max_working_pixels();
    let pasted = tauri::async_runtime::spawn_blocking(move || {
        let _job = crate::resources::acquire_job(None)?;
        crate::clipboard::read_image(limit).map_err(clipboard_error)
    })
    .await
    .map_err(|_| AppError::ProcessingFailure("clipboard worker stopped".into()))??;
    let (width, height) = pasted.image.dimensions();
    let mut store = state
        .layers
        .lock()
        .map_err(|_| AppError::ProcessingFailure("layer store is unavailable".into()))?;
    let pixel_id = store.register_typed(PixelBuffer::from(pasted.image))?;
    Ok(ClipboardPasteResult {
        pixels: LayerPixelsResult {
            pixel_id,
            width,
            height,
            filename: Some("Pasted image".to_string()),
            raw: None,
            origin: None,
        },
        format: pasted.format,
    })
}

/// Whether the clipboard holds a picture, so the interface can say whether paste has anything to
/// paste.
#[tauri::command]
pub async fn clipboard_has_image() -> Result<bool, AppError> {
    tauri::async_runtime::spawn_blocking(crate::clipboard::has_image)
        .await
        .map_err(|_| AppError::ProcessingFailure("clipboard worker stopped".into()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::mask::MaskBitmap;
    use image::Rgba;

    fn picture(width: u32, height: u32, alpha: u8) -> RgbaImage {
        let mut image = RgbaImage::new(width, height);
        for (x, y, pixel) in image.enumerate_pixels_mut() {
            *pixel = Rgba([x as u8 * 10, y as u8 * 10, 99, alpha]);
        }
        image
    }

    fn selection(width: u32, height: u32, covered: &[(u32, u32, u8)]) -> MaskBitmap {
        let mut mask = MaskBitmap::empty(width, height).unwrap();
        for &(x, y, value) in covered {
            mask.coverage_mut()[(y * width + x) as usize] = value;
        }
        mask
    }

    #[test]
    fn a_selection_cuts_the_picture_to_its_bounds_and_its_coverage() {
        let mask = selection(6, 5, &[(2, 1, 255), (3, 1, 128), (2, 2, 255), (3, 2, 255)]);
        let copied = picture_to_copy(picture(6, 5, 255), Some(&mask), false).unwrap();
        assert_eq!(copied.dimensions(), (2, 2));
        // (2, 1) of the picture is at (0, 0) of the copy, fully selected.
        assert_eq!(copied.get_pixel(0, 0).0, [20, 10, 99, 255]);
        // Half selected: half as opaque.
        assert_eq!(copied.get_pixel(1, 0).0, [30, 10, 99, 128]);
        assert_eq!(copied.get_pixel(0, 1).0, [20, 20, 99, 255]);
    }

    #[test]
    fn a_selection_multiplies_the_alpha_the_picture_already_has() {
        let mask = selection(2, 1, &[(0, 0, 128), (1, 0, 255)]);
        let copied = picture_to_copy(picture(2, 1, 100), Some(&mask), false).unwrap();
        assert_eq!(copied.get_pixel(0, 0).0[3], 50); // 100 * 128 / 255
        assert_eq!(copied.get_pixel(1, 0).0[3], 100);
    }

    #[test]
    fn an_empty_selection_has_nothing_to_copy_and_a_wrong_sized_one_is_refused() {
        let empty = selection(4, 4, &[]);
        let error = picture_to_copy(picture(4, 4, 255), Some(&empty), false).unwrap_err();
        assert!(error.to_string().contains("selection is empty"), "{error}");
        let wrong = selection(5, 4, &[(0, 0, 255)]);
        assert!(matches!(
            picture_to_copy(picture(4, 4, 255), Some(&wrong), false),
            Err(AppError::MaskDimensionMismatch { .. })
        ));
    }

    #[test]
    fn a_layer_is_trimmed_to_where_it_has_anything_and_a_merged_picture_is_not() {
        let mut layer = RgbaImage::new(8, 6);
        layer.put_pixel(3, 2, Rgba([255, 0, 0, 255]));
        layer.put_pixel(5, 3, Rgba([0, 255, 0, 40]));
        let trimmed = picture_to_copy(layer.clone(), None, true).unwrap();
        assert_eq!(trimmed.dimensions(), (3, 2));
        assert_eq!(trimmed.get_pixel(0, 0).0, [255, 0, 0, 255]);
        assert_eq!(trimmed.get_pixel(2, 1).0, [0, 255, 0, 40]);
        let whole = picture_to_copy(layer, None, false).unwrap();
        assert_eq!(whole.dimensions(), (8, 6));
    }

    #[test]
    fn a_layer_with_nothing_visible_has_nothing_to_copy() {
        let error = picture_to_copy(RgbaImage::new(4, 4), None, true).unwrap_err();
        assert!(error.to_string().contains("nothing visible"), "{error}");
    }
}
