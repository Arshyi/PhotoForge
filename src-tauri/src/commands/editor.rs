use crate::application::AppState;
use crate::domain::{EditOperation, EditPipeline, ExportResult, OpenImageResult, PreviewResult};
use crate::error::AppError;
use crate::image_processing::apply_pipeline;
use crate::infrastructure::{encode_preview, load_image, save_image};
use image::GenericImageView;
use std::path::PathBuf;
use std::sync::atomic::Ordering;
use std::time::Instant;
use tauri::State;

#[tauri::command]
pub async fn open_image(
    path: String,
    state: State<'_, AppState>,
) -> Result<OpenImageResult, AppError> {
    let started = Instant::now();
    let input_path = PathBuf::from(path);
    let loaded = tauri::async_runtime::spawn_blocking(move || load_image(&input_path))
        .await
        .map_err(|_| AppError::ProcessingFailure("image loading worker stopped".into()))??;

    let preview_data_url = encode_preview(loaded.preview.as_ref())?;
    let result = OpenImageResult {
        metadata: loaded.metadata.clone(),
        original_preview_data_url: preview_data_url.clone(),
        preview_data_url,
        processing_time_ms: started.elapsed().as_secs_f64() * 1_000.0,
    };

    let mut session = state
        .session
        .lock()
        .map_err(|_| AppError::ProcessingFailure("editor state is unavailable".into()))?;
    *session = Some(crate::application::EditorSession { source: loaded });
    state.latest_request.store(0, Ordering::Release);
    Ok(result)
}

#[tauri::command]
pub async fn render_preview(
    operations: Vec<EditOperation>,
    request_id: u64,
    state: State<'_, AppState>,
) -> Result<PreviewResult, AppError> {
    let mut pipeline = EditPipeline::default();
    pipeline.replace(operations)?;
    let validated_operations = pipeline.operations().to_vec();

    let source = {
        let session = state
            .session
            .lock()
            .map_err(|_| AppError::ProcessingFailure("editor state is unavailable".into()))?;
        session
            .as_ref()
            .ok_or(AppError::NoImageOpen)?
            .source
            .preview
            .clone()
    };

    state.latest_request.store(request_id, Ordering::Release);
    let started = Instant::now();
    let processed = tauri::async_runtime::spawn_blocking(move || {
        apply_pipeline(source.as_ref(), &validated_operations)
    })
    .await
    .map_err(|_| AppError::ProcessingFailure("preview worker stopped".into()))??;

    let is_current = state.latest_request.load(Ordering::Acquire) == request_id;
    let preview_data_url = if is_current {
        encode_preview(&processed)?
    } else {
        String::new()
    };

    Ok(PreviewResult {
        preview_data_url,
        request_id,
        processing_time_ms: started.elapsed().as_secs_f64() * 1_000.0,
        is_current,
        operation_count: pipeline.operations().len(),
    })
}

#[tauri::command]
pub async fn export_image(
    output_path: String,
    operations: Vec<EditOperation>,
    state: State<'_, AppState>,
) -> Result<ExportResult, AppError> {
    let mut pipeline = EditPipeline::default();
    pipeline.replace(operations)?;
    let validated_operations = pipeline.operations().to_vec();
    let output_path = PathBuf::from(output_path);

    let (source, original_path) = {
        let session = state
            .session
            .lock()
            .map_err(|_| AppError::ProcessingFailure("editor state is unavailable".into()))?;
        let source = &session.as_ref().ok_or(AppError::NoImageOpen)?.source;
        (source.original.clone(), source.path.clone())
    };

    let started = Instant::now();
    let (saved_path, width, height) = tauri::async_runtime::spawn_blocking(move || {
        let processed = apply_pipeline(source.as_ref(), &validated_operations)?;
        let (width, height) = processed.dimensions();
        let saved_path = save_image(&processed, &original_path, &output_path)?;
        Ok::<_, AppError>((saved_path, width, height))
    })
    .await
    .map_err(|_| AppError::ProcessingFailure("export worker stopped".into()))??;

    Ok(ExportResult {
        output_path: saved_path.to_string_lossy().into_owned(),
        width,
        height,
        processing_time_ms: started.elapsed().as_secs_f64() * 1_000.0,
    })
}
