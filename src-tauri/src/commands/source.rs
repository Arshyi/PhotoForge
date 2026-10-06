//! Commands for opening a source that may be too large to open whole.
//!
//! Probing reads a header and nothing else. A preview decodes at reduced size
//! through the cheapest path the format offers, gated by the same planner as
//! every other open. Nothing here uploads, caches to disk or contacts anything.
use serde::Serialize;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use tauri::State;

use crate::application::AppState;
use crate::error::AppError;
use crate::resources::admission::{AdmissionReport, SourceKind};
use crate::resources::memory::OsProbe;
use crate::source::open::{check_origin, source_preview, OriginState};
use crate::source::probe::{probe_path, report_with};
use crate::source::SourceOrigin;

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SourceAdmission {
    pub path: String,
    pub filename: String,
    pub kind: SourceKind,
    pub width: u32,
    pub height: u32,
    pub file_bytes: u64,
    pub has_icc: bool,
    pub bit_depth: u8,
    pub report: AdmissionReport,
}

/// Reads a source's header and says what can be done with it.
///
/// Priced against the budget the open itself would install, so that the choice
/// the dialog offers is the choice the open will honour.
#[tauri::command]
pub async fn probe_image_source(path: String) -> Result<SourceAdmission, AppError> {
    tauri::async_runtime::spawn_blocking(move || {
        let probed = probe_path(Path::new(&path))?;
        let (budget, limits) = crate::resources::candidate(&OsProbe);
        let report = report_with(&probed, 0, &budget, &limits);
        Ok(SourceAdmission {
            filename: probed
                .path
                .file_name()
                .and_then(|name| name.to_str())
                .unwrap_or("image")
                .to_string(),
            path,
            kind: probed.kind,
            width: probed.width,
            height: probed.height,
            file_bytes: probed.file_bytes,
            has_icc: probed.has_icc,
            bit_depth: probed.bit_depth,
            report,
        })
    })
    .await
    .map_err(|_| AppError::ProcessingFailure("source probe worker stopped".into()))?
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SourcePreviewResult {
    pub data_url: String,
    pub width: u32,
    pub height: u32,
}

/// A bounded picture of the whole source, for choosing a region on.
///
/// Starting one cancels the previous, and `cancel_source_preview` cancels it
/// directly, because it holds the CPU job gate while it decodes.
#[tauri::command]
pub async fn source_preview_image(
    path: String,
    max_edge: u32,
    state: State<'_, AppState>,
) -> Result<SourcePreviewResult, AppError> {
    let flag = Arc::new(AtomicBool::new(false));
    {
        let mut slot = state
            .source_preview_cancel
            .lock()
            .map_err(|_| AppError::ProcessingFailure("preview state is unavailable".into()))?;
        if let Some(previous) = slot.replace(Arc::clone(&flag)) {
            previous.store(true, Ordering::Release);
        }
    }
    let worker_flag = Arc::clone(&flag);
    let result = tauri::async_runtime::spawn_blocking(move || {
        let _job = crate::resources::acquire_job(Some(&worker_flag))?;
        source_preview(&PathBuf::from(path), max_edge, Some(&worker_flag))
    })
    .await
    .map_err(|_| AppError::ProcessingFailure("source preview worker stopped".into()))?;
    // Clear the slot if it is still ours, so a finished preview is not "cancelled".
    if let Ok(mut slot) = state.source_preview_cancel.lock() {
        if slot
            .as_ref()
            .is_some_and(|current| Arc::ptr_eq(current, &flag))
        {
            *slot = None;
        }
    }
    let preview = result?;
    Ok(SourcePreviewResult {
        data_url: preview.data_url,
        width: preview.width,
        height: preview.height,
    })
}

/// Stops the preview in flight, if there is one.
#[tauri::command]
pub async fn cancel_source_preview(state: State<'_, AppState>) -> Result<(), AppError> {
    if let Ok(mut slot) = state.source_preview_cancel.lock() {
        if let Some(flag) = slot.take() {
            flag.store(true, Ordering::Release);
        }
    }
    Ok(())
}

/// Whether the file a layer's origin names is still the one it describes.
///
/// Checked by hashing the file again, and only when the user asks.
#[tauri::command]
pub async fn inspect_source_origin(origin: SourceOrigin) -> Result<OriginState, AppError> {
    tauri::async_runtime::spawn_blocking(move || check_origin(&origin))
        .await
        .map_err(|_| AppError::ProcessingFailure("source check worker stopped".into()))?
}
