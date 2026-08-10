use crate::application::AppState;
use crate::domain::EditOperation;
use crate::error::AppError;
use crate::image_processing::apply_pipeline;
use crate::mask::{
    align_to_image_edges_with_progress, apply_mask_operation_with_progress, checked_mask_length,
    compose_with_progress, export_png, import_png, load_mask, mask_diagnostics,
    mask_operation_work_units, mask_progress_snapshot, rasterize,
    remap_between_chains_with_progress, request_mask_progress_cancel, save_mask,
    select_color_range_with_progress, select_magic_wand_with_progress, ColorRangeOptions,
    CompositionMode, GeometryChain, GeometryStep, MaskDiagnostics, MaskFile, MaskOperation,
    MaskProgress, MaskProgressHandle, MaskSnapshot, MaskWorkContext, Point, SelectionShape,
    WandOptions,
};
use image::GenericImageView;
use serde::{Deserialize, Serialize};
use std::collections::HashSet;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
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

pub const MAX_REMAP_ITEMS: usize = 256;
pub const MAX_REMAP_TOTAL_PIXELS: u64 = 200_000_000;

#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MaskRemapItem {
    pub key: String,
    pub mask: MaskSnapshot,
    pub old_stage: usize,
    pub new_stage: usize,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RemappedMaskItem {
    pub key: String,
    pub mask: MaskSnapshot,
    pub diagnostics: MaskDiagnostics,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MaskRemapResult {
    pub masks: Vec<RemappedMaskItem>,
    pub final_width: u32,
    pub final_height: u32,
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
    let progress = begin_request(&state, document_id, request_id, "rasterize")?;
    let outcome = async {
        let _permit = state.mask_gate.lock().await;
        prepare_request(&state, &progress, document_id, request_id)?;
        let pixels = checked_mask_length(width, height)? as u64;
        let planned = progress.planned(pixels.saturating_mul(2));
        let cancelled = state.mask_cancelled.clone();
        let started = Instant::now();
        let bitmap = tauri::async_runtime::spawn_blocking(move || {
            let report = |phase: &str, completed: u64, total: u64| {
                planned.report_local(phase, completed, total)
            };
            let context = MaskWorkContext::new(Some(cancelled.as_ref()), Some(&report));
            context.report("rasterize_pixels", 0, pixels)?;
            let incoming = rasterize(width, height, &shape)?;
            context.report("rasterize_pixels", pixels, pixels)?;
            let base = decode_base(base, width, height)?;
            compose_with_progress(base.as_ref(), &incoming, mode, context)
        })
        .await
        .map_err(|_| AppError::ProcessingFailure("selection rasterizer stopped".into()))??;
        result(&state, bitmap, document_id, request_id, started)
    }
    .await;
    finish_request(&state, &progress, outcome)
}

#[tauri::command]
pub async fn transform_selection_mask(
    mask: MaskSnapshot,
    operation: MaskOperation,
    document_id: u64,
    request_id: u64,
    state: State<'_, AppState>,
) -> Result<MaskResult, AppError> {
    let progress = begin_request(&state, document_id, request_id, operation_name(&operation))?;
    let outcome = async {
        let _permit = state.mask_gate.lock().await;
        prepare_request(&state, &progress, document_id, request_id)?;
        let pixels = checked_mask_length(mask.width, mask.height)? as u64;
        let planned = progress.planned(mask_operation_work_units(&operation, pixels));
        let cancelled = state.mask_cancelled.clone();
        let started = Instant::now();
        let bitmap = tauri::async_runtime::spawn_blocking(move || {
            let report = |phase: &str, completed: u64, total: u64| {
                planned.report_local(phase, completed, total)
            };
            let decoded = mask.decode()?;
            apply_mask_operation_with_progress(
                &decoded,
                &operation,
                MaskWorkContext::new(Some(cancelled.as_ref()), Some(&report)),
            )
        })
        .await
        .map_err(|_| AppError::ProcessingFailure("selection operation worker stopped".into()))??;
        result(&state, bitmap, document_id, request_id, started)
    }
    .await;
    finish_request(&state, &progress, outcome)
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
    let progress = begin_request(&state, document_id, request_id, "compose")?;
    let outcome = async {
        let _permit = state.mask_gate.lock().await;
        prepare_request(&state, &progress, document_id, request_id)?;
        let pixels = checked_mask_length(base.width, base.height)? as u64;
        let planned = progress.planned(pixels);
        let cancelled = state.mask_cancelled.clone();
        let started = Instant::now();
        let bitmap = tauri::async_runtime::spawn_blocking(move || {
            let report = |phase: &str, completed: u64, total: u64| {
                planned.report_local(phase, completed, total)
            };
            let context = MaskWorkContext::new(Some(cancelled.as_ref()), Some(&report));
            let base = base.decode()?;
            let incoming = incoming.decode()?;
            compose_with_progress(Some(&base), &incoming, mode, context)
        })
        .await
        .map_err(|_| AppError::ProcessingFailure("mask composition worker stopped".into()))??;
        result(&state, bitmap, document_id, request_id, started)
    }
    .await;
    finish_request(&state, &progress, outcome)
}

#[allow(clippy::too_many_arguments)]
#[tauri::command]
pub async fn remap_selection_masks(
    old_geometry: Vec<GeometryStep>,
    new_geometry: Vec<GeometryStep>,
    items: Vec<MaskRemapItem>,
    document_id: u64,
    request_id: u64,
    state: State<'_, AppState>,
) -> Result<MaskRemapResult, AppError> {
    let progress = begin_request(&state, document_id, request_id, "remap_geometry")?;
    let outcome = async {
        let _permit = state.mask_gate.lock().await;
        prepare_request(&state, &progress, document_id, request_id)?;
        let source = source_for_selection(&state, document_id, request_id)?;
        let original_dimensions = source.dimensions();
        let cancelled = state.mask_cancelled.clone();
        let worker_progress = progress.clone();
        let started = Instant::now();
        let (masks, final_dimensions) = tauri::async_runtime::spawn_blocking(move || {
            remap_batch(
                original_dimensions,
                old_geometry,
                new_geometry,
                items,
                Some(cancelled.as_ref()),
                Some(&worker_progress),
            )
        })
        .await
        .map_err(|_| AppError::ProcessingFailure("mask geometry worker stopped".into()))??;
        let is_current = request_is_current(&state, document_id, request_id)?;
        Ok(MaskRemapResult {
            masks,
            final_width: final_dimensions.0,
            final_height: final_dimensions.1,
            document_id,
            request_id,
            processing_time_ms: started.elapsed().as_secs_f64() * 1_000.0,
            is_current,
        })
    }
    .await;
    finish_request(&state, &progress, outcome)
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
    let progress = begin_request(&state, document_id, request_id, "refine")?;
    let outcome = async {
        if !matches!(operation, MaskOperation::Refine { .. }) {
            return Err(AppError::InvalidMask(
                "edge-aware refinement requires refine parameters".into(),
            ));
        }
        let _permit = state.mask_gate.lock().await;
        prepare_request(&state, &progress, document_id, request_id)?;
        let source = source_for_selection(&state, document_id, request_id)?;
        let pixels = checked_mask_length(mask.width, mask.height)? as u64;
        let edge_units = if edge_strength == 0.0 || mask.width < 3 || mask.height < 3 {
            pixels
        } else {
            u64::from(mask.width - 2) * u64::from(mask.height - 2)
        };
        let planned = progress
            .planned(mask_operation_work_units(&operation, pixels).saturating_add(edge_units));
        let cancelled = state.mask_cancelled.clone();
        let started = Instant::now();
        let bitmap = tauri::async_runtime::spawn_blocking(move || {
            let report = |phase: &str, completed: u64, total: u64| {
                planned.report_local(phase, completed, total)
            };
            let context = MaskWorkContext::new(Some(cancelled.as_ref()), Some(&report));
            context.report("render_selection_source", 0, 0)?;
            let image = rendered_source(source.as_ref(), &operations, sample_merged)?.to_rgba8();
            context.check_cancelled()?;
            let decoded = mask.decode()?;
            let refined = apply_mask_operation_with_progress(&decoded, &operation, context)?;
            align_to_image_edges_with_progress(&refined, &image, edge_strength, context)
        })
        .await
        .map_err(|_| AppError::ProcessingFailure("edge-refinement worker stopped".into()))??;
        result(&state, bitmap, document_id, request_id, started)
    }
    .await;
    finish_request(&state, &progress, outcome)
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
    let progress = begin_request(&state, document_id, request_id, "magic_wand")?;
    let outcome = async {
        let _permit = state.mask_gate.lock().await;
        prepare_request(&state, &progress, document_id, request_id)?;
        let source = source_for_selection(&state, document_id, request_id)?;
        let cancelled = state.mask_cancelled.clone();
        let worker_progress = progress.clone();
        let started = Instant::now();
        let bitmap = tauri::async_runtime::spawn_blocking(move || {
            worker_progress.mark_running("render_selection_source")?;
            let image = rendered_source(source.as_ref(), &operations, sample_merged)?.to_rgba8();
            let pixels = u64::from(image.width()) * u64::from(image.height());
            let selection_units = if options.contiguous { 0 } else { pixels };
            let planned = worker_progress.planned(selection_units.saturating_add(pixels));
            let report = |phase: &str, completed: u64, total: u64| {
                planned.report_local(phase, completed, total)
            };
            let context = MaskWorkContext::new(Some(cancelled.as_ref()), Some(&report));
            context.check_cancelled()?;
            let incoming = select_magic_wand_with_progress(&image, point, options, context)?;
            let base = decode_base(base, image.width(), image.height())?;
            compose_with_progress(base.as_ref(), &incoming, mode, context)
        })
        .await
        .map_err(|_| AppError::ProcessingFailure("magic-wand worker stopped".into()))??;
        result(&state, bitmap, document_id, request_id, started)
    }
    .await;
    finish_request(&state, &progress, outcome)
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
    let progress = begin_request(&state, document_id, request_id, "color_range")?;
    let outcome = async {
        let _permit = state.mask_gate.lock().await;
        prepare_request(&state, &progress, document_id, request_id)?;
        let source = source_for_selection(&state, document_id, request_id)?;
        let cancelled = state.mask_cancelled.clone();
        let worker_progress = progress.clone();
        let started = Instant::now();
        let bitmap = tauri::async_runtime::spawn_blocking(move || {
            worker_progress.mark_running("render_selection_source")?;
            let image = rendered_source(source.as_ref(), &operations, sample_merged)?.to_rgba8();
            let pixels = u64::from(image.width()) * u64::from(image.height());
            let planned = worker_progress.planned(pixels.saturating_mul(2));
            let report = |phase: &str, completed: u64, total: u64| {
                planned.report_local(phase, completed, total)
            };
            let context = MaskWorkContext::new(Some(cancelled.as_ref()), Some(&report));
            context.check_cancelled()?;
            let incoming = select_color_range_with_progress(&image, &samples, options, context)?;
            let base = decode_base(base, image.width(), image.height())?;
            compose_with_progress(base.as_ref(), &incoming, mode, context)
        })
        .await
        .map_err(|_| AppError::ProcessingFailure("color-range worker stopped".into()))??;
        result(&state, bitmap, document_id, request_id, started)
    }
    .await;
    finish_request(&state, &progress, outcome)
}

#[tauri::command]
pub fn cancel_mask_operation(
    request_id: u64,
    state: State<'_, AppState>,
) -> Result<bool, AppError> {
    if request_mask_progress_cancel(&state.mask_progress, request_id)? {
        state.mask_cancelled.store(true, Ordering::Release);
        Ok(true)
    } else {
        Ok(false)
    }
}

#[tauri::command]
pub fn get_mask_progress(
    document_id: u64,
    request_id: u64,
    state: State<'_, AppState>,
) -> Result<Option<MaskProgress>, AppError> {
    mask_progress_snapshot(&state.mask_progress, document_id, request_id)
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

fn begin_request(
    state: &AppState,
    document_id: u64,
    request_id: u64,
    operation: impl Into<String>,
) -> Result<MaskProgressHandle, AppError> {
    state.mask_cancelled.store(true, Ordering::Release);
    let progress = MaskProgressHandle::begin(
        state.mask_progress.clone(),
        document_id,
        request_id,
        operation,
    )?;
    state
        .latest_mask_request
        .store(request_id, Ordering::Release);
    Ok(progress)
}

fn prepare_request(
    state: &AppState,
    progress: &MaskProgressHandle,
    document_id: u64,
    request_id: u64,
) -> Result<(), AppError> {
    ensure_current(state, document_id, request_id)?;
    state.mask_cancelled.store(false, Ordering::Release);
    if progress.is_cancelling()? {
        state.mask_cancelled.store(true, Ordering::Release);
        return Err(AppError::MaskCancelled);
    }
    progress.mark_running("preparing")
}

fn finish_request<T>(
    state: &AppState,
    progress: &MaskProgressHandle,
    outcome: Result<T, AppError>,
) -> Result<T, AppError> {
    match outcome {
        Ok(value) => {
            if state.mask_cancelled.load(Ordering::Acquire) || progress.is_cancelling()? {
                let _ = progress.acknowledge_cancelled();
                return Err(AppError::MaskCancelled);
            }
            progress.complete()?;
            Ok(value)
        }
        Err(error) => {
            if matches!(&error, AppError::MaskCancelled) {
                let _ = progress.acknowledge_cancelled();
            } else {
                let _ = progress.fail();
            }
            Err(error)
        }
    }
}

fn ensure_current(state: &AppState, document_id: u64, request_id: u64) -> Result<(), AppError> {
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
    Ok(())
}

fn source_for_selection(
    state: &AppState,
    document_id: u64,
    request_id: u64,
) -> Result<std::sync::Arc<image::DynamicImage>, AppError> {
    ensure_current(state, document_id, request_id)?;
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
        return apply_pipeline(source, operations);
    }
    let geometry: Vec<_> = operations
        .iter()
        .filter(|operation| {
            matches!(
                operation,
                EditOperation::ReflectHorizontal
                    | EditOperation::Rotate { .. }
                    | EditOperation::Crop { .. }
                    | EditOperation::Straighten { .. }
                    | EditOperation::Perspective { .. }
            )
        })
        .cloned()
        .collect();
    apply_pipeline(source, &geometry)
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

fn remap_batch(
    original_dimensions: (u32, u32),
    old_geometry: Vec<GeometryStep>,
    new_geometry: Vec<GeometryStep>,
    items: Vec<MaskRemapItem>,
    cancelled: Option<&AtomicBool>,
    progress: Option<&MaskProgressHandle>,
) -> Result<(Vec<RemappedMaskItem>, (u32, u32)), AppError> {
    if items.len() > MAX_REMAP_ITEMS {
        return Err(AppError::InvalidMask(format!(
            "mask geometry batches may contain at most {MAX_REMAP_ITEMS} items"
        )));
    }
    let old_chain = GeometryChain::new(original_dimensions.0, original_dimensions.1, old_geometry)?;
    let new_chain = GeometryChain::new(original_dimensions.0, original_dimensions.1, new_geometry)?;
    let final_dimensions = new_chain.dimensions_at(new_chain.len())?;
    if items.is_empty() {
        if let Some(progress) = progress {
            progress.report("geometry_validation", 0, 0)?;
        }
        return Ok((Vec::new(), final_dimensions));
    }
    let mut keys = HashSet::with_capacity(items.len());
    let mut aggregate_pixels = 0_u64;
    let mut work_units = 0_u64;
    for item in &items {
        if item.key.is_empty() || item.key.len() > 128 || !keys.insert(item.key.clone()) {
            return Err(AppError::InvalidMask(
                "mask geometry item keys must be unique values containing 1 to 128 bytes".into(),
            ));
        }
        let old_dimensions = old_chain.dimensions_at(item.old_stage)?;
        let new_dimensions = new_chain.dimensions_at(item.new_stage)?;
        if (item.mask.width, item.mask.height) != old_dimensions {
            return Err(AppError::MaskDimensionMismatch {
                mask_width: item.mask.width,
                mask_height: item.mask.height,
                image_width: old_dimensions.0,
                image_height: old_dimensions.1,
            });
        }
        let source_pixels = u64::from(item.mask.width)
            .checked_mul(u64::from(item.mask.height))
            .ok_or(AppError::OutOfMemoryRisk)?;
        let target_pixels = u64::from(new_dimensions.0)
            .checked_mul(u64::from(new_dimensions.1))
            .ok_or(AppError::OutOfMemoryRisk)?;
        aggregate_pixels = aggregate_pixels
            .checked_add(source_pixels)
            .and_then(|value| value.checked_add(target_pixels))
            .ok_or(AppError::OutOfMemoryRisk)?;
        if aggregate_pixels > MAX_REMAP_TOTAL_PIXELS {
            return Err(AppError::MaskTooLarge {
                pixels: aggregate_pixels,
                limit: MAX_REMAP_TOTAL_PIXELS,
            });
        }
        work_units = work_units.saturating_add(old_chain.remap_work_units(
            item.old_stage,
            &new_chain,
            item.new_stage,
        )?);
    }

    let planned = progress.map(|progress| progress.planned(work_units));
    let mut remapped = Vec::with_capacity(items.len());
    for (index, item) in items.into_iter().enumerate() {
        if cancelled.is_some_and(|flag| flag.load(Ordering::Acquire)) {
            return Err(AppError::MaskCancelled);
        }
        if let Some(planned) = &planned {
            planned.report_local(&format!("remap_item_{index}"), 0, 0)?;
        }
        let report = |phase: &str, completed: u64, total: u64| {
            if let Some(planned) = &planned {
                planned.report_local(phase, completed, total)
            } else {
                Ok(())
            }
        };
        let decoded = item.mask.decode()?;
        let bitmap = remap_between_chains_with_progress(
            &decoded,
            &old_chain,
            item.old_stage,
            &new_chain,
            item.new_stage,
            MaskWorkContext::new(cancelled, Some(&report)),
        )?;
        remapped.push(RemappedMaskItem {
            key: item.key,
            mask: MaskSnapshot::encode(&bitmap),
            diagnostics: mask_diagnostics(&bitmap),
        });
    }
    Ok((remapped, final_dimensions))
}

fn result(
    state: &AppState,
    bitmap: crate::mask::MaskBitmap,
    document_id: u64,
    request_id: u64,
    started: Instant,
) -> Result<MaskResult, AppError> {
    let is_current = request_is_current(state, document_id, request_id)?;
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

fn request_is_current(
    state: &AppState,
    document_id: u64,
    request_id: u64,
) -> Result<bool, AppError> {
    Ok(
        state.latest_mask_request.load(Ordering::Acquire) == request_id
            && !state.mask_cancelled.load(Ordering::Acquire)
            && state.pending_open_request.load(Ordering::Acquire) == 0
            && state
                .session
                .lock()
                .map_err(|_| AppError::ProcessingFailure("editor state is unavailable".into()))?
                .as_ref()
                .is_some_and(|session| session.document_id == document_id),
    )
}

fn operation_name(operation: &MaskOperation) -> &'static str {
    match operation {
        MaskOperation::SelectAll => "select_all",
        MaskOperation::Deselect => "deselect",
        MaskOperation::Invert => "invert",
        MaskOperation::Feather { .. } => "feather",
        MaskOperation::Expand { .. } => "expand",
        MaskOperation::Contract { .. } => "contract",
        MaskOperation::Smooth { .. } => "smooth",
        MaskOperation::FillHoles => "fill_holes",
        MaskOperation::RemoveSmallIslands { .. } => "remove_small_islands",
        MaskOperation::Border { .. } => "border",
        MaskOperation::Refine { .. } => "refine",
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::application::EditorSession;
    use crate::domain::{CropOverlay, ImageMetadata};
    use crate::infrastructure::LoadedImage;
    use image::{DynamicImage, Rgba, RgbaImage};
    use std::path::PathBuf;
    use std::sync::Arc;

    #[test]
    fn base_dimensions_are_checked() {
        let base = MaskSnapshot::encode(&crate::mask::MaskBitmap::empty(2, 2).unwrap());
        assert!(decode_base(Some(base), 3, 2).is_err());
    }

    #[test]
    fn sample_merged_accepts_dimension_changes() {
        let source = image::DynamicImage::new_rgba8(2, 1);
        assert_eq!(
            rendered_source(&source, &[EditOperation::Rotate { degrees: 90 }], true)
                .unwrap()
                .dimensions(),
            (1, 2)
        );
    }

    #[test]
    fn geometry_only_source_applies_crop_and_rotation_but_excludes_brightness() {
        let mut image = RgbaImage::from_pixel(4, 2, Rgba([10, 20, 30, 255]));
        image.put_pixel(2, 1, Rgba([70, 80, 90, 255]));
        let source = DynamicImage::ImageRgba8(image);
        let rendered = rendered_source(
            &source,
            &[
                EditOperation::Brightness { amount: 1.0 },
                EditOperation::Crop {
                    x: 0.0,
                    y: 0.0,
                    width: 0.75,
                    height: 1.0,
                    aspect_ratio: None,
                    overlay: CropOverlay::None,
                },
                EditOperation::Rotate { degrees: 90 },
            ],
            false,
        )
        .unwrap()
        .to_rgba8();
        assert_eq!(rendered.dimensions(), (2, 3));
        assert!(rendered.pixels().all(|pixel| pixel[0] < 200));
        assert!(rendered.pixels().any(|pixel| pixel[0] == 70));
    }

    fn test_state(dimensions: (u32, u32), document_id: u64, request_id: u64) -> AppState {
        let state = AppState::default();
        let image = Arc::new(DynamicImage::new_rgba8(dimensions.0, dimensions.1));
        *state.session.lock().unwrap() = Some(EditorSession {
            source: LoadedImage {
                path: PathBuf::from("test.png"),
                original: image.clone(),
                preview: image,
                metadata: ImageMetadata {
                    filename: "test.png".into(),
                    width: dimensions.0,
                    height: dimensions.1,
                    format: "PNG".into(),
                    file_size: 0,
                    color_space: "sRGB".into(),
                    bit_depth: 8,
                    has_alpha: true,
                    created_at: None,
                    modified_at: None,
                    camera_model: None,
                    exif_available: false,
                },
            },
            document_id,
            analysis: None,
        });
        state
            .latest_mask_request
            .store(request_id, Ordering::Release);
        state
    }

    #[test]
    fn current_stage_dimensions_are_not_forced_to_original_dimensions() {
        let state = test_state((8, 4), 11, 22);
        assert!(ensure_current(&state, 11, 22).is_ok());
        assert!(checked_mask_length(3, 8).is_ok());
        let current_stage = MaskSnapshot::encode(&crate::mask::MaskBitmap::full(3, 8).unwrap());
        assert!(decode_base(Some(current_stage), 3, 8).is_ok());
    }

    #[test]
    fn request_terminal_states_cover_success_failure_and_rapid_cancel() {
        let state = test_state((4, 4), 1, 10);
        let completed = begin_request(&state, 1, 10, "invert").unwrap();
        state.mask_cancelled.store(false, Ordering::Release);
        assert!(finish_request(&state, &completed, Ok::<_, AppError>(())).is_ok());
        assert_eq!(
            completed.snapshot().unwrap().unwrap().state,
            crate::mask::MaskProgressState::Completed
        );

        let failed = begin_request(&state, 1, 11, "invert").unwrap();
        state.mask_cancelled.store(false, Ordering::Release);
        let failure: Result<(), AppError> = finish_request(
            &state,
            &failed,
            Err(AppError::InvalidMask("fixture".into())),
        );
        assert!(failure.is_err());
        assert_eq!(
            failed.snapshot().unwrap().unwrap().state,
            crate::mask::MaskProgressState::Failed
        );

        let cancelled = begin_request(&state, 1, 12, "invert").unwrap();
        request_mask_progress_cancel(&state.mask_progress, 12).unwrap();
        state.mask_cancelled.store(true, Ordering::Release);
        assert!(matches!(
            finish_request(&state, &cancelled, Ok::<_, AppError>(())),
            Err(AppError::MaskCancelled)
        ));
        assert_eq!(
            cancelled.snapshot().unwrap().unwrap().state,
            crate::mask::MaskProgressState::Cancelled
        );
    }

    #[test]
    fn queued_cancellation_survives_request_preparation() {
        let state = test_state((4, 4), 7, 70);
        let progress = begin_request(&state, 7, 70, "feather").unwrap();
        request_mask_progress_cancel(&state.mask_progress, 70).unwrap();
        state.mask_cancelled.store(true, Ordering::Release);
        assert!(matches!(
            prepare_request(&state, &progress, 7, 70),
            Err(AppError::MaskCancelled)
        ));
        assert!(state.mask_cancelled.load(Ordering::Acquire));
    }

    fn remap_item(key: &str, mask: MaskSnapshot) -> MaskRemapItem {
        MaskRemapItem {
            key: key.into(),
            mask,
            old_stage: 0,
            new_stage: 1,
        }
    }

    #[test]
    fn remap_batch_preserves_keys_and_is_all_or_error() {
        let mask = MaskSnapshot::encode(
            &crate::mask::MaskBitmap::from_coverage(2, 1, vec![64, 192]).unwrap(),
        );
        let geometry = vec![GeometryStep::Rotate { degrees: 90 }];
        let (result, dimensions) = remap_batch(
            (2, 1),
            vec![],
            geometry.clone(),
            vec![remap_item("active", mask.clone())],
            None,
            None,
        )
        .unwrap();
        assert_eq!(dimensions, (1, 2));
        assert_eq!(result[0].key, "active");
        assert_eq!(result[0].mask.decode().unwrap().coverage(), &[64, 192]);

        let mut invalid = remap_item("named", mask);
        invalid.mask.checksum = "fnv1a64:0000000000000000".into();
        assert!(remap_batch(
            (2, 1),
            vec![],
            geometry,
            vec![
                remap_item(
                    "active",
                    MaskSnapshot::encode(&crate::mask::MaskBitmap::full(2, 1).unwrap())
                ),
                invalid,
            ],
            None,
            None,
        )
        .is_err());
    }

    #[test]
    fn remap_batch_rejects_invalid_stages_dimensions_and_keys() {
        let snapshot = MaskSnapshot::encode(&crate::mask::MaskBitmap::full(2, 1).unwrap());
        let mut invalid_stage = remap_item("active", snapshot.clone());
        invalid_stage.new_stage = 2;
        assert!(remap_batch(
            (2, 1),
            vec![],
            vec![GeometryStep::Rotate { degrees: 90 }],
            vec![invalid_stage],
            None,
            None,
        )
        .is_err());

        let wrong_dimensions = remap_item(
            "active",
            MaskSnapshot::encode(&crate::mask::MaskBitmap::full(1, 1).unwrap()),
        );
        assert!(matches!(
            remap_batch(
                (2, 1),
                vec![],
                vec![GeometryStep::Rotate { degrees: 90 }],
                vec![wrong_dimensions],
                None,
                None,
            ),
            Err(AppError::MaskDimensionMismatch { .. })
        ));

        assert!(remap_batch(
            (2, 1),
            vec![],
            vec![GeometryStep::Rotate { degrees: 90 }],
            vec![
                remap_item("duplicate", snapshot.clone()),
                remap_item("duplicate", snapshot),
            ],
            None,
            None,
        )
        .is_err());
    }

    #[test]
    fn remap_batch_enforces_item_and_aggregate_pixel_caps_before_decode() {
        let small = MaskSnapshot::encode(&crate::mask::MaskBitmap::full(1, 1).unwrap());
        let too_many = (0..=MAX_REMAP_ITEMS)
            .map(|index| MaskRemapItem {
                key: format!("mask-{index}"),
                mask: small.clone(),
                old_stage: 0,
                new_stage: 0,
            })
            .collect();
        assert!(remap_batch((1, 1), vec![], vec![], too_many, None, None).is_err());

        let declared_large = MaskSnapshot {
            version: crate::mask::MASK_FORMAT_VERSION,
            width: 10_000,
            height: 10_000,
            encoding: "base64_u8".into(),
            data: String::new(),
            checksum: String::new(),
        };
        let items = ["one", "two"]
            .into_iter()
            .map(|key| MaskRemapItem {
                key: key.into(),
                mask: declared_large.clone(),
                old_stage: 0,
                new_stage: 0,
            })
            .collect();
        assert!(matches!(
            remap_batch((10_000, 10_000), vec![], vec![], items, None, None),
            Err(AppError::MaskTooLarge { .. })
        ));
    }

    #[test]
    fn empty_remap_batch_validates_geometry_and_returns_final_dimensions() {
        let (masks, dimensions) = remap_batch(
            (3, 2),
            vec![],
            vec![GeometryStep::Rotate { degrees: 90 }],
            vec![],
            None,
            None,
        )
        .unwrap();
        assert!(masks.is_empty());
        assert_eq!(dimensions, (2, 3));
    }

    #[test]
    fn empty_remap_batch_rejects_folded_perspective() {
        let folded = GeometryStep::Perspective {
            corners: crate::domain::PerspectiveCorners {
                top_left: [0.0, 0.0],
                top_right: [1.0, 0.0],
                bottom_right: [0.4, 0.4],
                bottom_left: [0.0, 1.0],
            },
        };
        assert!(matches!(
            remap_batch((10, 10), vec![], vec![folded], vec![], None, None),
            Err(AppError::InvalidMask(_))
        ));
    }

    #[test]
    fn remap_batch_acknowledges_cancellation() {
        let cancelled = AtomicBool::new(true);
        let item = MaskRemapItem {
            key: "active".into(),
            mask: MaskSnapshot::encode(&crate::mask::MaskBitmap::full(4, 4).unwrap()),
            old_stage: 0,
            new_stage: 0,
        };
        assert!(matches!(
            remap_batch((4, 4), vec![], vec![], vec![item], Some(&cancelled), None,),
            Err(AppError::MaskCancelled)
        ));
    }

    #[test]
    fn remap_batch_progress_advances_across_repeated_item_phases() {
        let progress = MaskProgressHandle::begin(
            Arc::new(std::sync::Mutex::new(None)),
            1,
            2,
            "remap_geometry",
        )
        .unwrap();
        let mask = MaskSnapshot::encode(
            &crate::mask::MaskBitmap::from_coverage(3, 2, vec![0, 64, 128, 192, 224, 255]).unwrap(),
        );
        let items = ["active", "named"]
            .into_iter()
            .map(|key| remap_item(key, mask.clone()))
            .collect();
        remap_batch(
            (3, 2),
            vec![],
            vec![GeometryStep::Rotate { degrees: 90 }],
            items,
            None,
            Some(&progress),
        )
        .unwrap();
        let snapshot = progress.snapshot().unwrap().unwrap();
        assert_eq!(snapshot.completed_units, 4);
        assert_eq!(snapshot.total_units, 4);
        assert_eq!(snapshot.phase, "geometry_step_0_rows");
    }
}
