use crate::application::{preview_batch, run_batch, AppState};
use crate::components::RestorationEngineFactory;
use crate::domain::{
    validate_shortcuts, BatchOptions, BatchPreview, BatchState, BatchStatus, EditOperation,
    ExportProfile, ExportResult, HistogramResult, PixelInspection, ShortcutBinding, Workflow,
    WorkflowDocument, WorkspaceLayout,
};
use crate::error::AppError;
use crate::image_processing::{apply_pipeline, calculate_histogram, inspect_pixel};
use crate::infrastructure::{
    load_workflow, parse_workflow_json, save_image_with_profile, save_workflow,
};
use image::GenericImageView;
use std::path::PathBuf;
use std::sync::atomic::Ordering;
use std::time::Instant;
use tauri::State;

#[tauri::command]
pub async fn generate_histogram(
    operations: Vec<EditOperation>,
    document_id: u64,
    request_id: u64,
    state: State<'_, AppState>,
) -> Result<HistogramResult, AppError> {
    for operation in &operations {
        operation.validate()?;
    }
    state
        .latest_histogram_request
        .store(request_id, Ordering::Release);
    let _permit = state.histogram_gate.lock().await;
    if state.latest_histogram_request.load(Ordering::Acquire) != request_id {
        return Ok(stale_histogram(document_id, request_id));
    }
    let source = {
        let session = state
            .session
            .lock()
            .map_err(|_| AppError::HistogramGeneration("editor state is unavailable".into()))?;
        let session = session.as_ref().ok_or(AppError::NoImageOpen)?;
        if session.document_id != document_id {
            return Ok(stale_histogram(document_id, request_id));
        }
        session.source.preview.clone()
    };
    let started = Instant::now();
    let (before, after) = tauri::async_runtime::spawn_blocking(move || {
        let before = calculate_histogram(source.as_ref());
        let processed = apply_pipeline(source.as_ref(), &operations)?;
        let after = calculate_histogram(&processed);
        Ok::<_, AppError>((before, after))
    })
    .await
    .map_err(|_| AppError::HistogramGeneration("histogram worker stopped".into()))??;
    let is_current = state.latest_histogram_request.load(Ordering::Acquire) == request_id;
    Ok(HistogramResult {
        before,
        after,
        document_id,
        request_id,
        processing_time_ms: started.elapsed().as_secs_f64() * 1_000.0,
        is_current,
    })
}

fn stale_histogram(document_id: u64, request_id: u64) -> HistogramResult {
    let empty = calculate_histogram(&image::DynamicImage::new_rgba8(0, 0));
    HistogramResult {
        before: empty.clone(),
        after: empty,
        document_id,
        request_id,
        processing_time_ms: 0.0,
        is_current: false,
    }
}

#[tauri::command]
pub async fn inspect_image_pixel(
    x: u32,
    y: u32,
    operations: Vec<EditOperation>,
    document_id: u64,
    state: State<'_, AppState>,
) -> Result<PixelInspection, AppError> {
    let source = {
        let session = state
            .session
            .lock()
            .map_err(|_| AppError::ProcessingFailure("editor state is unavailable".into()))?;
        let session = session.as_ref().ok_or(AppError::NoImageOpen)?;
        if session.document_id != document_id {
            return Err(AppError::NoImageOpen);
        }
        session.source.preview.clone()
    };
    tauri::async_runtime::spawn_blocking(move || {
        let processed = apply_pipeline(source.as_ref(), &operations)?;
        inspect_pixel(&processed, x, y)
            .ok_or_else(|| AppError::CropBounds("pixel coordinates are outside the image".into()))
    })
    .await
    .map_err(|_| AppError::ProcessingFailure("pixel inspector worker stopped".into()))?
}

#[tauri::command]
pub async fn create_point_operation(
    x: u32,
    y: u32,
    white: bool,
    operations: Vec<EditOperation>,
    document_id: u64,
    state: State<'_, AppState>,
) -> Result<EditOperation, AppError> {
    let sample = inspect_image_pixel(x, y, operations, document_id, state).await?;
    if white && (sample.red == 0 || sample.green == 0 || sample.blue == 0) {
        return Err(AppError::InvalidOperation(
            "a zero-valued channel cannot establish a white point".into(),
        ));
    }
    Ok(if white {
        EditOperation::WhitePoint {
            red: sample.red,
            green: sample.green,
            blue: sample.blue,
        }
    } else {
        EditOperation::BlackPoint {
            red: sample.red,
            green: sample.green,
            blue: sample.blue,
        }
    })
}

#[tauri::command]
pub fn validate_workflow_json(json: String) -> Result<WorkflowDocument, AppError> {
    parse_workflow_json(&json)
}

#[tauri::command]
pub fn import_workflow(path: String) -> Result<WorkflowDocument, AppError> {
    load_workflow(&PathBuf::from(path))
}

#[tauri::command]
pub fn export_workflow(path: String, document: WorkflowDocument) -> Result<String, AppError> {
    save_workflow(&PathBuf::from(path), &document).map(|saved| saved.to_string_lossy().into_owned())
}

#[tauri::command]
pub async fn preview_batch_workflow(
    options: BatchOptions,
    workflow: Workflow,
) -> Result<BatchPreview, AppError> {
    tauri::async_runtime::spawn_blocking(move || preview_batch(&options, &workflow))
        .await
        .map_err(|_| AppError::BatchFailure("batch preview worker stopped".into()))?
}

#[tauri::command]
pub async fn start_batch_workflow(
    batch_id: u64,
    options: BatchOptions,
    workflow: Workflow,
    state: State<'_, AppState>,
) -> Result<BatchStatus, AppError> {
    let _permit = state.batch_gate.lock().await;
    state.batch_cancelled.store(false, Ordering::Release);
    let status = state.batch_status.clone();
    let cancelled = state.batch_cancelled.clone();
    let result = tauri::async_runtime::spawn_blocking(move || {
        run_batch(batch_id, options, workflow, status, cancelled)
    })
    .await
    .map_err(|_| AppError::BatchFailure("batch worker stopped".into()))?;
    if let Err(error) = &result {
        if let Ok(mut status) = state.batch_status.lock() {
            status.batch_id = batch_id;
            status.state = BatchState::Failed;
            status.failures.push(crate::domain::BatchFailureRecord {
                input_path: String::new(),
                error: error.to_string(),
            });
        }
    }
    result
}

#[tauri::command]
pub fn get_batch_status(state: State<'_, AppState>) -> Result<BatchStatus, AppError> {
    state
        .batch_status
        .lock()
        .map(|status| status.clone())
        .map_err(|_| AppError::BatchFailure("batch status is unavailable".into()))
}

#[tauri::command]
pub fn cancel_batch(state: State<'_, AppState>) -> Result<BatchStatus, AppError> {
    state.batch_cancelled.store(true, Ordering::Release);
    let mut status = state
        .batch_status
        .lock()
        .map_err(|_| AppError::BatchFailure("batch status is unavailable".into()))?;
    if status.state == BatchState::Running || status.state == BatchState::Discovering {
        status.state = BatchState::Cancelling;
    }
    Ok(status.clone())
}

#[tauri::command]
pub fn validate_workspace_layout(layout: WorkspaceLayout) -> Result<WorkspaceLayout, AppError> {
    layout.validate()?;
    Ok(layout)
}

#[tauri::command]
pub fn validate_shortcut_bindings(
    bindings: Vec<ShortcutBinding>,
) -> Result<Vec<ShortcutBinding>, AppError> {
    validate_shortcuts(&bindings)?;
    Ok(bindings)
}

#[tauri::command]
pub async fn export_with_profile(
    output_path: String,
    operations: Vec<EditOperation>,
    profile: ExportProfile,
    state: State<'_, AppState>,
) -> Result<ExportResult, AppError> {
    let _permit = state.export_gate.lock().await;
    for operation in &operations {
        operation.validate()?;
    }
    let output_path = PathBuf::from(output_path);
    let (source, original_path) = {
        let session = state
            .session
            .lock()
            .map_err(|_| AppError::ProcessingFailure("editor state is unavailable".into()))?;
        let source = &session.as_ref().ok_or(AppError::NoImageOpen)?.source;
        (source.original.clone(), source.path.clone())
    };
    let engine = {
        let registry = state.components.lock().map_err(|_| {
            AppError::ComponentInitializationFailure("registry is unavailable".into())
        })?;
        RestorationEngineFactory::create(registry.active_engine())
    };
    let started = Instant::now();
    let (saved_path, width, height) = tauri::async_runtime::spawn_blocking(move || {
        let processed = engine.process(source.as_ref(), &operations)?;
        let (width, height) = processed.dimensions();
        let saved = save_image_with_profile(&processed, &original_path, &output_path, profile)?;
        Ok::<_, AppError>((saved, width, height))
    })
    .await
    .map_err(|_| AppError::ProcessingFailure("profile export worker stopped".into()))??;
    Ok(ExportResult {
        output_path: saved_path.to_string_lossy().into_owned(),
        width,
        height,
        processing_time_ms: started.elapsed().as_secs_f64() * 1_000.0,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::{EditOperation, Workflow, WORKFLOW_SCHEMA_VERSION};

    #[test]
    fn validates_workflow_command_payload() {
        let document = WorkflowDocument {
            schema_version: WORKFLOW_SCHEMA_VERSION,
            workflow: Workflow {
                id: "x".into(),
                name: "X".into(),
                description: String::new(),
                folder: String::new(),
                favorite: false,
                operations: vec![EditOperation::Grayscale],
                created_at: String::new(),
                updated_at: String::new(),
            },
        };
        let json = serde_json::to_string(&document).unwrap();
        assert_eq!(validate_workflow_json(json).unwrap(), document);
    }

    #[test]
    fn validates_workspace_at_command_boundary() {
        let layout = WorkspaceLayout {
            schema_version: 1,
            name: "Editing".into(),
            left_panel_width: 240,
            right_panel_width: 360,
            collapsed_sections: vec![],
            active_panel: "tools".into(),
            high_contrast: false,
            ui_scale: 1.0,
        };
        assert_eq!(validate_workspace_layout(layout.clone()).unwrap(), layout);
    }

    #[test]
    fn validates_shortcuts_at_command_boundary() {
        let bindings = vec![ShortcutBinding {
            action: "open".into(),
            keys: "Ctrl+O".into(),
        }];
        assert_eq!(
            validate_shortcut_bindings(bindings.clone()).unwrap(),
            bindings
        );
    }
}
