use crate::application::AppState;
use crate::domain::EditOperation;
use crate::error::AppError;
use crate::image_processing::apply_pipeline;
use crate::mask::{
    align_to_image_edges, apply_mask_operation, compose, export_png, import_png, load_mask,
    mask_diagnostics, rasterize, save_mask, select_color_range, select_magic_wand,
    ColorRangeOptions, CompositionMode, MaskDiagnostics, MaskFile, MaskOperation, MaskSnapshot,
    Point, SelectionShape, WandOptions,
};
use image::GenericImageView;
use serde::Serialize;
use std::path::PathBuf;
use std::sync::atomic::Ordering;
use std::time::Instant;
use tauri::State;

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MaskResult {
    pub mask: MaskSnapshot,
    pub diagnostics: MaskDiagnostics,
    pub document_id: u64,
    pub request_id: u64,
    pub processing_time_ms: f64,
    pub is_current: bool,
}

#[allow(clippy::too_many_arguments)]
#[tauri::command]
pub async fn rasterize_selection(
    width: u32,
    height: u32,
    shape: SelectionShape,
    mode: CompositionMode,
    base: Option<MaskSnapshot>,
    document_id: u64,
    request_id: u64,
    state: State<'_, AppState>,
) -> Result<MaskResult, AppError> {
    begin_request(&state, request_id);
    let _permit = state.mask_gate.lock().await;
    ensure_current(&state, document_id, request_id, Some((width, height)))?;
    state.mask_cancelled.store(false, Ordering::Release);
    let started = Instant::now();
    let bitmap = tauri::async_runtime::spawn_blocking(move || {
        let incoming = rasterize(width, height, &shape)?;
        let base = decode_base(base, width, height)?;
        compose(base.as_ref(), &incoming, mode)
    })
    .await
    .map_err(|_| AppError::ProcessingFailure("selection rasterizer stopped".into()))??;
    result(&state, bitmap, document_id, request_id, started)
}

#[tauri::command]
pub async fn transform_selection_mask(
    mask: MaskSnapshot,
    operation: MaskOperation,
    document_id: u64,
    request_id: u64,
    state: State<'_, AppState>,
) -> Result<MaskResult, AppError> {
    begin_request(&state, request_id);
    let _permit = state.mask_gate.lock().await;
    ensure_current(
        &state,
        document_id,
        request_id,
        Some((mask.width, mask.height)),
    )?;
    state.mask_cancelled.store(false, Ordering::Release);
    let cancelled = state.mask_cancelled.clone();
    let started = Instant::now();
    let bitmap = tauri::async_runtime::spawn_blocking(move || {
        let decoded = mask.decode()?;
        apply_mask_operation(&decoded, &operation, Some(cancelled.as_ref()))
    })
    .await
    .map_err(|_| AppError::ProcessingFailure("selection operation worker stopped".into()))??;
    result(&state, bitmap, document_id, request_id, started)
}

#[tauri::command]
pub async fn compose_selection_masks(
    base: MaskSnapshot,
    incoming: MaskSnapshot,
    mode: CompositionMode,
    document_id: u64,
    request_id: u64,
    state: State<'_, AppState>,
) -> Result<MaskResult, AppError> {
    begin_request(&state, request_id);
    let _permit = state.mask_gate.lock().await;
    ensure_current(
        &state,
        document_id,
        request_id,
        Some((base.width, base.height)),
    )?;
    state.mask_cancelled.store(false, Ordering::Release);
    let started = Instant::now();
    let bitmap = tauri::async_runtime::spawn_blocking(move || {
        let base = base.decode()?;
        let incoming = incoming.decode()?;
        compose(Some(&base), &incoming, mode)
    })
    .await
    .map_err(|_| AppError::ProcessingFailure("mask composition worker stopped".into()))??;
    result(&state, bitmap, document_id, request_id, started)
}

#[allow(clippy::too_many_arguments)]
#[tauri::command]
pub async fn refine_selection_mask(
    mask: MaskSnapshot,
    operation: MaskOperation,
    edge_strength: f32,
    sample_merged: bool,
    operations: Vec<EditOperation>,
    document_id: u64,
    request_id: u64,
    state: State<'_, AppState>,
) -> Result<MaskResult, AppError> {
    if !matches!(operation, MaskOperation::Refine { .. }) {
        return Err(AppError::InvalidMask(
            "edge-aware refinement requires refine parameters".into(),
        ));
    }
    begin_request(&state, request_id);
    let _permit = state.mask_gate.lock().await;
    let source = source_for_selection(&state, document_id, request_id)?;
    state.mask_cancelled.store(false, Ordering::Release);
    let cancelled = state.mask_cancelled.clone();
    let started = Instant::now();
    let bitmap = tauri::async_runtime::spawn_blocking(move || {
        let image = rendered_source(source.as_ref(), &operations, sample_merged)?.to_rgba8();
        let decoded = mask.decode()?;
        let refined = apply_mask_operation(&decoded, &operation, Some(cancelled.as_ref()))?;
        align_to_image_edges(&refined, &image, edge_strength, Some(cancelled.as_ref()))
    })
    .await
    .map_err(|_| AppError::ProcessingFailure("edge-refinement worker stopped".into()))??;
    result(&state, bitmap, document_id, request_id, started)
}

#[allow(clippy::too_many_arguments)]
#[tauri::command]
pub async fn magic_wand_selection(
    point: Point,
    options: WandOptions,
    mode: CompositionMode,
    base: Option<MaskSnapshot>,
    sample_merged: bool,
    operations: Vec<EditOperation>,
    document_id: u64,
    request_id: u64,
    state: State<'_, AppState>,
) -> Result<MaskResult, AppError> {
    begin_request(&state, request_id);
    let _permit = state.mask_gate.lock().await;
    let source = source_for_selection(&state, document_id, request_id)?;
    state.mask_cancelled.store(false, Ordering::Release);
    let cancelled = state.mask_cancelled.clone();
    let started = Instant::now();
    let bitmap = tauri::async_runtime::spawn_blocking(move || {
        let image = rendered_source(source.as_ref(), &operations, sample_merged)?.to_rgba8();
        let incoming = select_magic_wand(&image, point, options, Some(cancelled.as_ref()))?;
        let base = decode_base(base, image.width(), image.height())?;
        compose(base.as_ref(), &incoming, mode)
    })
    .await
    .map_err(|_| AppError::ProcessingFailure("magic-wand worker stopped".into()))??;
    result(&state, bitmap, document_id, request_id, started)
}

#[allow(clippy::too_many_arguments)]
#[tauri::command]
pub async fn color_range_selection(
    samples: Vec<Point>,
    options: ColorRangeOptions,
    mode: CompositionMode,
    base: Option<MaskSnapshot>,
    sample_merged: bool,
    operations: Vec<EditOperation>,
    document_id: u64,
    request_id: u64,
    state: State<'_, AppState>,
) -> Result<MaskResult, AppError> {
    begin_request(&state, request_id);
    let _permit = state.mask_gate.lock().await;
    let source = source_for_selection(&state, document_id, request_id)?;
    state.mask_cancelled.store(false, Ordering::Release);
    let cancelled = state.mask_cancelled.clone();
    let started = Instant::now();
    let bitmap = tauri::async_runtime::spawn_blocking(move || {
        let image = rendered_source(source.as_ref(), &operations, sample_merged)?.to_rgba8();
        let incoming = select_color_range(&image, &samples, options, Some(cancelled.as_ref()))?;
        let base = decode_base(base, image.width(), image.height())?;
        compose(base.as_ref(), &incoming, mode)
    })
    .await
    .map_err(|_| AppError::ProcessingFailure("color-range worker stopped".into()))??;
    result(&state, bitmap, document_id, request_id, started)
}

#[tauri::command]
pub fn cancel_mask_operation(request_id: u64, state: State<'_, AppState>) -> bool {
    if state.latest_mask_request.load(Ordering::Acquire) == request_id {
        state.mask_cancelled.store(true, Ordering::Release);
        true
    } else {
        false
    }
}

#[tauri::command]
pub fn inspect_selection_mask(mask: MaskSnapshot) -> Result<MaskDiagnostics, AppError> {
    Ok(mask_diagnostics(&mask.decode()?))
}

#[tauri::command]
pub fn validate_mask_snapshot(mask: MaskSnapshot) -> Result<MaskSnapshot, AppError> {
    mask.decode()?;
    Ok(mask)
}

#[tauri::command]
pub fn import_mask_file(path: String) -> Result<MaskFile, AppError> {
    load_mask(&PathBuf::from(path))
}

#[tauri::command]
pub fn export_mask_file(path: String, document: MaskFile) -> Result<String, AppError> {
    save_mask(&PathBuf::from(path), &document).map(|value| value.to_string_lossy().into_owned())
}

#[tauri::command]
pub fn import_mask_png(path: String) -> Result<MaskSnapshot, AppError> {
    Ok(MaskSnapshot::encode(&import_png(&PathBuf::from(path))?))
}

#[tauri::command]
pub fn export_mask_png(path: String, mask: MaskSnapshot) -> Result<String, AppError> {
    let bitmap = mask.decode()?;
    export_png(&PathBuf::from(path), &bitmap).map(|value| value.to_string_lossy().into_owned())
}

fn begin_request(state: &AppState, request_id: u64) {
    state
        .latest_mask_request
        .store(request_id, Ordering::Release);
    state.mask_cancelled.store(true, Ordering::Release);
}

fn ensure_current(
    state: &AppState,
    document_id: u64,
    request_id: u64,
    dimensions: Option<(u32, u32)>,
) -> Result<(), AppError> {
    if state.latest_mask_request.load(Ordering::Acquire) != request_id
        || state.pending_open_request.load(Ordering::Acquire) != 0
    {
        return Err(AppError::MaskCancelled);
    }
    let session = state
        .session
        .lock()
        .map_err(|_| AppError::ProcessingFailure("editor state is unavailable".into()))?;
    let session = session.as_ref().ok_or(AppError::NoImageOpen)?;
    if session.document_id != document_id {
        return Err(AppError::MaskCancelled);
    }
    if let Some((width, height)) = dimensions {
        let actual = session.source.original.dimensions();
        if actual != (width, height) {
            return Err(AppError::MaskDimensionMismatch {
                mask_width: width,
                mask_height: height,
                image_width: actual.0,
                image_height: actual.1,
            });
        }
    }
    Ok(())
}

fn source_for_selection(
    state: &AppState,
    document_id: u64,
    request_id: u64,
) -> Result<std::sync::Arc<image::DynamicImage>, AppError> {
    ensure_current(state, document_id, request_id, None)?;
    let session = state
        .session
        .lock()
        .map_err(|_| AppError::ProcessingFailure("editor state is unavailable".into()))?;
    Ok(session
        .as_ref()
        .ok_or(AppError::NoImageOpen)?
        .source
        .original
        .clone())
}

fn rendered_source(
    source: &image::DynamicImage,
    operations: &[EditOperation],
    sample_merged: bool,
) -> Result<image::DynamicImage, AppError> {
    if sample_merged {
        let rendered = apply_pipeline(source, operations)?;
        if rendered.dimensions() != source.dimensions() {
            return Err(AppError::InvalidMask(
                "sample-merged selection is unavailable after a dimension-changing edit".into(),
            ));
        }
        Ok(rendered)
    } else {
        Ok(source.clone())
    }
}

fn decode_base(
    base: Option<MaskSnapshot>,
    width: u32,
    height: u32,
) -> Result<Option<crate::mask::MaskBitmap>, AppError> {
    base.map(|snapshot| {
        let bitmap = snapshot.decode()?;
        if bitmap.width() != width || bitmap.height() != height {
            return Err(AppError::MaskDimensionMismatch {
                mask_width: bitmap.width(),
                mask_height: bitmap.height(),
                image_width: width,
                image_height: height,
            });
        }
        Ok(bitmap)
    })
    .transpose()
}

fn result(
    state: &AppState,
    bitmap: crate::mask::MaskBitmap,
    document_id: u64,
    request_id: u64,
    started: Instant,
) -> Result<MaskResult, AppError> {
    let is_current = state.latest_mask_request.load(Ordering::Acquire) == request_id
        && !state.mask_cancelled.load(Ordering::Acquire)
        && state.pending_open_request.load(Ordering::Acquire) == 0
        && state
            .session
            .lock()
            .map_err(|_| AppError::ProcessingFailure("editor state is unavailable".into()))?
            .as_ref()
            .is_some_and(|session| session.document_id == document_id);
    let diagnostics = mask_diagnostics(&bitmap);
    Ok(MaskResult {
        mask: MaskSnapshot::encode(&bitmap),
        diagnostics,
        document_id,
        request_id,
        processing_time_ms: started.elapsed().as_secs_f64() * 1_000.0,
        is_current,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn base_dimensions_are_checked() {
        let base = MaskSnapshot::encode(&crate::mask::MaskBitmap::empty(2, 2).unwrap());
        assert!(decode_base(Some(base), 3, 2).is_err());
    }

    #[test]
    fn sample_merged_rejects_dimension_changes() {
        let source = image::DynamicImage::new_rgba8(2, 1);
        assert!(rendered_source(&source, &[EditOperation::Rotate { degrees: 90 }], true).is_err());
    }
}
