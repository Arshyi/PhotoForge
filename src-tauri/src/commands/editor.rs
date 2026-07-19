use crate::application::AppState;
use crate::components::RestorationEngineFactory;
use crate::domain::{
    AnalysisResult, EditOperation, EditPipeline, ExportResult, OpenImageResult, PreviewResult,
};
use crate::error::AppError;
use crate::image_processing::analyze_image_quality;
use crate::infrastructure::{encode_preview, load_image, save_image};
use image::GenericImageView;
use std::path::PathBuf;
use std::sync::atomic::Ordering;
use std::time::Instant;
use tauri::State;

#[tauri::command]
pub async fn open_image(
    path: String,
    request_id: u64,
    state: State<'_, AppState>,
) -> Result<OpenImageResult, AppError> {
    let started = Instant::now();
    state
        .latest_open_request
        .store(request_id, Ordering::Release);
    state
        .pending_open_request
        .store(request_id, Ordering::Release);
    state.latest_preview_request.store(0, Ordering::Release);
    state.latest_analysis_request.store(0, Ordering::Release);
    state.latest_plan_request.store(0, Ordering::Release);

    let input_path = PathBuf::from(path);
    let loaded = match tauri::async_runtime::spawn_blocking(move || load_image(&input_path)).await {
        Ok(Ok(loaded)) => loaded,
        Ok(Err(error)) => {
            clear_pending_open(&state, request_id);
            return Err(error);
        }
        Err(_) => {
            clear_pending_open(&state, request_id);
            return Err(AppError::ProcessingFailure(
                "image loading worker stopped".into(),
            ));
        }
    };

    if state.latest_open_request.load(Ordering::Acquire) != request_id {
        return Ok(stale_open_result(loaded.metadata, request_id, started));
    }

    let preview_data_url = match encode_preview(loaded.preview.as_ref()) {
        Ok(preview) => preview,
        Err(error) => {
            clear_pending_open(&state, request_id);
            return Err(error);
        }
    };

    if state.latest_open_request.load(Ordering::Acquire) != request_id {
        return Ok(stale_open_result(loaded.metadata, request_id, started));
    }

    let result = OpenImageResult {
        metadata: loaded.metadata.clone(),
        original_preview_data_url: preview_data_url.clone(),
        preview_data_url,
        processing_time_ms: started.elapsed().as_secs_f64() * 1_000.0,
        document_id: request_id,
        is_current: true,
    };

    let mut session = state
        .session
        .lock()
        .map_err(|_| AppError::ProcessingFailure("editor state is unavailable".into()))?;
    *session = Some(crate::application::EditorSession {
        source: loaded,
        document_id: request_id,
        analysis: None,
    });
    drop(session);
    clear_pending_open(&state, request_id);
    Ok(result)
}

fn stale_analysis(document_id: u64, request_id: u64) -> AnalysisResult {
    AnalysisResult {
        analysis: None,
        document_id,
        request_id,
        processing_time_ms: 0.0,
        is_current: false,
    }
}

#[tauri::command]
pub async fn analyze_image(
    document_id: u64,
    request_id: u64,
    state: State<'_, AppState>,
) -> Result<AnalysisResult, AppError> {
    state
        .latest_analysis_request
        .store(request_id, Ordering::Release);
    let _analysis_permit = state.analysis_gate.lock().await;

    if state.latest_analysis_request.load(Ordering::Acquire) != request_id
        || state.pending_open_request.load(Ordering::Acquire) != 0
    {
        return Ok(stale_analysis(document_id, request_id));
    }

    let (source, cached) = {
        let session = state
            .session
            .lock()
            .map_err(|_| AppError::AnalysisFailure)?;
        let session = session.as_ref().ok_or(AppError::NoImageOpen)?;
        if session.document_id != document_id {
            return Ok(stale_analysis(document_id, request_id));
        }
        (session.source.preview.clone(), session.analysis.clone())
    };

    if let Some(analysis) = cached {
        return Ok(AnalysisResult {
            analysis: Some(analysis),
            document_id,
            request_id,
            processing_time_ms: 0.0,
            is_current: true,
        });
    }

    let started = Instant::now();
    let analysis = tauri::async_runtime::spawn_blocking(move || analyze_image_quality(&source))
        .await
        .map_err(|_| AppError::AnalysisFailure)?;
    let is_current = state.latest_analysis_request.load(Ordering::Acquire) == request_id
        && state.pending_open_request.load(Ordering::Acquire) == 0;
    if !is_current {
        return Ok(stale_analysis(document_id, request_id));
    }

    let mut session = state
        .session
        .lock()
        .map_err(|_| AppError::AnalysisFailure)?;
    let session = session.as_mut().ok_or(AppError::NoImageOpen)?;
    if session.document_id != document_id {
        return Ok(stale_analysis(document_id, request_id));
    }
    session.analysis = Some(analysis.clone());
    Ok(AnalysisResult {
        analysis: Some(analysis),
        document_id,
        request_id,
        processing_time_ms: started.elapsed().as_secs_f64() * 1_000.0,
        is_current: true,
    })
}

fn stale_open_result(
    metadata: crate::domain::ImageMetadata,
    request_id: u64,
    started: Instant,
) -> OpenImageResult {
    OpenImageResult {
        metadata,
        original_preview_data_url: String::new(),
        preview_data_url: String::new(),
        processing_time_ms: started.elapsed().as_secs_f64() * 1_000.0,
        document_id: request_id,
        is_current: false,
    }
}

fn clear_pending_open(state: &AppState, request_id: u64) {
    let _ = state.pending_open_request.compare_exchange(
        request_id,
        0,
        Ordering::AcqRel,
        Ordering::Acquire,
    );
}

fn stale_preview(request_id: u64, operation_count: usize) -> PreviewResult {
    PreviewResult {
        preview_data_url: String::new(),
        request_id,
        processing_time_ms: 0.0,
        is_current: false,
        operation_count,
    }
}

#[tauri::command]
pub async fn render_preview(
    operations: Vec<EditOperation>,
    document_id: u64,
    request_id: u64,
    state: State<'_, AppState>,
) -> Result<PreviewResult, AppError> {
    let mut pipeline = EditPipeline::default();
    pipeline.replace(operations)?;
    let operation_count = pipeline.operations().len();
    let validated_operations = pipeline.operations().to_vec();

    if state.pending_open_request.load(Ordering::Acquire) != 0 {
        return Ok(stale_preview(request_id, operation_count));
    }

    state
        .latest_preview_request
        .store(request_id, Ordering::Release);
    let _preview_permit = state.preview_gate.lock().await;

    if state.latest_preview_request.load(Ordering::Acquire) != request_id
        || state.pending_open_request.load(Ordering::Acquire) != 0
    {
        return Ok(stale_preview(request_id, operation_count));
    }

    let source = {
        let session = state
            .session
            .lock()
            .map_err(|_| AppError::ProcessingFailure("editor state is unavailable".into()))?;
        let session = session.as_ref().ok_or(AppError::NoImageOpen)?;
        if session.document_id != document_id {
            return Ok(stale_preview(request_id, operation_count));
        }
        session.source.preview.clone()
    };
    let engine = {
        let registry = state.components.lock().map_err(|_| {
            AppError::ComponentInitializationFailure("registry is unavailable".into())
        })?;
        RestorationEngineFactory::create(registry.active_engine())
    };

    let started = Instant::now();
    let processed = tauri::async_runtime::spawn_blocking(move || {
        engine.process(source.as_ref(), &validated_operations)
    })
    .await
    .map_err(|_| AppError::ProcessingFailure("preview worker stopped".into()))??;

    let is_current = state.latest_preview_request.load(Ordering::Acquire) == request_id
        && state.pending_open_request.load(Ordering::Acquire) == 0
        && state
            .session
            .lock()
            .map_err(|_| AppError::ProcessingFailure("editor state is unavailable".into()))?
            .as_ref()
            .is_some_and(|session| session.document_id == document_id);
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
        operation_count,
    })
}

#[tauri::command]
pub async fn export_image(
    output_path: String,
    operations: Vec<EditOperation>,
    state: State<'_, AppState>,
) -> Result<ExportResult, AppError> {
    let _export_permit = state.export_gate.lock().await;
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
    let engine = {
        let registry = state.components.lock().map_err(|_| {
            AppError::ComponentInitializationFailure("registry is unavailable".into())
        })?;
        RestorationEngineFactory::create(registry.active_engine())
    };

    let started = Instant::now();
    let (saved_path, width, height) = tauri::async_runtime::spawn_blocking(move || {
        let processed = engine.process(source.as_ref(), &validated_operations)?;
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
