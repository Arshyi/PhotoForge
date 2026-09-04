//! Model Manager commands.
//!
//! Everything here is an explicit user action on a file the user already has.
//! Nothing downloads, nothing runs a model, and nothing touches a path outside
//! PhotoForge's own model directory.
use tauri::State;

use crate::application::AppState;
use crate::error::AppError;
use crate::inference::model::{
    Capability, ModelColorSpace, ModelDescriptor, ModelFormat, Normalization,
};
use crate::inference::InferenceStatus;

/// Reports the runtime, the installed models and the honest state of every
/// capability, including why an unavailable one is unavailable.
#[tauri::command]
pub async fn inference_status(state: State<'_, AppState>) -> Result<InferenceStatus, AppError> {
    Ok(state.models.status())
}

/// What the interface sends to install a model the user picked.
///
/// The file's size, hash and format are not accepted from here: they are read
/// from the file itself during import, so the recorded identity cannot be a
/// claim made by whoever wrote this request.
#[derive(Debug, Clone, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ModelImportRequest {
    pub path: String,
    pub id: String,
    pub name: String,
    pub version: String,
    pub architecture: String,
    pub capability: Capability,
    pub color_space: ModelColorSpace,
    pub normalization: Normalization,
    pub input_channels: u32,
    pub output_channels: u32,
    pub scale: u32,
    pub tile_size: u32,
    pub tile_overlap: u32,
    #[serde(default)]
    pub license: String,
    #[serde(default)]
    pub source: String,
}

/// Installs a model from a path the user chose.
#[tauri::command]
pub async fn import_inference_model(
    request: ModelImportRequest,
    state: State<'_, AppState>,
) -> Result<ModelDescriptor, AppError> {
    let path = std::path::PathBuf::from(&request.path);
    let descriptor = ModelDescriptor {
        id: request.id,
        name: request.name,
        version: request.version,
        architecture: request.architecture,
        capability: request.capability,
        // Determined by inspection, not by the request.
        format: ModelFormat::Onnx,
        color_space: request.color_space,
        normalization: request.normalization,
        input_channels: request.input_channels,
        output_channels: request.output_channels,
        scale: request.scale,
        tile_size: request.tile_size,
        tile_overlap: request.tile_overlap,
        file_bytes: 0,
        sha256: String::new(),
        license: request.license,
        source: request.source,
    };
    state.models.import(&path, descriptor)
}

/// Removes an installed model and its file from PhotoForge's model directory.
///
/// Never touches the file the user imported from, and never anything outside
/// that directory.
#[tauri::command]
pub async fn remove_inference_model(
    id: String,
    state: State<'_, AppState>,
) -> Result<InferenceStatus, AppError> {
    state.models.remove(&id)?;
    Ok(state.models.status())
}
