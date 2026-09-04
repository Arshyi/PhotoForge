use crate::error::AppError;
use crate::raw::{decoder_capabilities, inspect_raw_path, RawInspection};
use serde::Serialize;
use std::path::PathBuf;

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RawInspectionResult {
    pub inspection: RawInspection,
    pub capabilities: crate::raw::RawDecoderCapabilities,
}

/// Inspects a user-selected RAW source without decoding or modifying it. The
/// command is metadata-only until a vetted decoder backend is bundled;
/// recognised formats therefore return an explicit unavailable status instead
/// of falling through to the 8-bit PNG/JPEG/WebP loader.
#[tauri::command]
pub fn inspect_raw(path: String) -> Result<RawInspectionResult, AppError> {
    let inspection = inspect_raw_path(&PathBuf::from(path))
        .map_err(|error| AppError::RawInspection(error.to_string()))?;
    Ok(RawInspectionResult {
        inspection,
        capabilities: decoder_capabilities(),
    })
}
