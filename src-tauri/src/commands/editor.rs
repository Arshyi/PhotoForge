use crate::application::AppState;
use crate::components::RestorationEngineFactory;
use crate::domain::{
    AnalysisResult, EditOperation, EditPipeline, ExportResult, OpenImageResult, PreviewResult,
};
use crate::error::AppError;
use crate::image_processing::{
    analyze_image_quality, apply_pipeline_float, prepare_preview_operations,
};
use crate::infrastructure::{encode_preview, load_image, save_float_png16, save_image};
use crate::layers::LayerPixelStore;
use image::GenericImageView;
use std::path::PathBuf;
use std::sync::atomic::Ordering;
use std::time::Instant;
use tauri::State;

/// All image/project/recovery opens share one replacement protocol. Holding the
/// session mutex while publishing and committing prevents an older worker from
/// winning the race after a newer open has started.
pub(super) struct OpenRequest<'a> {
    state: &'a AppState,
    request_id: u64,
}

impl<'a> OpenRequest<'a> {
    pub(super) fn begin(state: &'a AppState, request_id: u64) -> Result<Self, AppError> {
        if request_id == 0 {
            return Err(AppError::ProcessingFailure(
                "open request identifiers must be nonzero".into(),
            ));
        }
        let _session = state
            .session
            .lock()
            .map_err(|_| AppError::ProcessingFailure("editor state is unavailable".into()))?;
        state
            .latest_open_request
            .store(request_id, Ordering::Release);
        state
            .pending_open_request
            .store(request_id, Ordering::Release);
        state.latest_preview_request.store(0, Ordering::Release);
        state.latest_analysis_request.store(0, Ordering::Release);
        state.latest_plan_request.store(0, Ordering::Release);
        state.latest_histogram_request.store(0, Ordering::Release);
        state.latest_layer_request.store(0, Ordering::Release);
        state.latest_mask_request.store(0, Ordering::Release);
        state.mask_cancelled.store(true, Ordering::Release);
        Ok(Self { state, request_id })
    }

    pub(super) fn is_current(&self) -> bool {
        self.state.latest_open_request.load(Ordering::Acquire) == self.request_id
    }

    pub(super) fn commit(
        &self,
        source: crate::infrastructure::LoadedImage,
        layers: LayerPixelStore,
    ) -> Result<bool, AppError> {
        let mut session = self
            .state
            .session
            .lock()
            .map_err(|_| AppError::ProcessingFailure("editor state is unavailable".into()))?;
        let mut store = self
            .state
            .layers
            .lock()
            .map_err(|_| AppError::ProcessingFailure("layer store is unavailable".into()))?;
        if !self.is_current() {
            return Ok(false);
        }
        // No fallible work after either piece of live state has been replaced.
        *store = layers;
        *session = Some(crate::application::EditorSession {
            source,
            document_id: self.request_id,
            analysis: None,
        });
        Ok(true)
    }
}

impl Drop for OpenRequest<'_> {
    fn drop(&mut self) {
        clear_pending_open(self.state, self.request_id);
    }
}

#[tauri::command]
pub async fn open_image(
    path: String,
    request_id: u64,
    state: State<'_, AppState>,
) -> Result<OpenImageResult, AppError> {
    let started = Instant::now();
    let request = OpenRequest::begin(&state, request_id)?;

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

    // Stage the replacement store; allocation/validation failures must preserve
    // both the old source image and all of its live/history pixel buffers.
    let mut store = LayerPixelStore::default();
    let (width, height) = loaded.original.dimensions();
    store.reset(width, height)?;
    let background_pixel_id = store.register(loaded.original.to_rgba8())?;

    let result = OpenImageResult {
        metadata: loaded.metadata.clone(),
        original_preview_data_url: preview_data_url.clone(),
        preview_data_url,
        processing_time_ms: started.elapsed().as_secs_f64() * 1_000.0,
        document_id: request_id,
        is_current: true,
        background_pixel_id,
    };

    if !request.commit(loaded, store)? {
        return Ok(stale_open_result(result.metadata, request_id, started));
    }
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
    layer_document: Option<crate::layers::LayerDocument>,
    operations: Option<Vec<EditOperation>>,
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

    // Cache is the latest completed analysis for planners, not a reusable
    // source-image result: a supplied layer tree/pipeline may have changed.
    // Recompute rather than risk returning a cache with ambiguous provenance.
    {
        let mut session = state
            .session
            .lock()
            .map_err(|_| AppError::AnalysisFailure)?;
        let session = session.as_mut().ok_or(AppError::NoImageOpen)?;
        if session.document_id != document_id {
            return Ok(stale_analysis(document_id, request_id));
        }
        session.analysis = None;
    }
    let source = super::sampling::capture(&state, document_id, layer_document, true)?;
    let full_dimensions = source.full_dimensions;
    let operations = operations.unwrap_or_default();
    let started = Instant::now();
    let analysis = tauri::async_runtime::spawn_blocking(move || {
        let source = source.render()?;
        let prepared =
            prepare_preview_operations(&operations, full_dimensions, source.dimensions())?;
        let processed = crate::image_processing::apply_pipeline(&source, &prepared)?;
        Ok::<_, AppError>(analyze_image_quality(&processed))
    })
    .await
    .map_err(|_| AppError::AnalysisFailure)??;
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
        background_pixel_id: String::new(),
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

    let (source, full_source_dimensions, preview_source_dimensions) = {
        let session = state
            .session
            .lock()
            .map_err(|_| AppError::ProcessingFailure("editor state is unavailable".into()))?;
        let session = session.as_ref().ok_or(AppError::NoImageOpen)?;
        if session.document_id != document_id {
            return Ok(stale_preview(request_id, operation_count));
        }
        (
            session.source.preview.clone(),
            session.source.original.dimensions(),
            session.source.preview.dimensions(),
        )
    };
    let engine = {
        let registry = state.components.lock().map_err(|_| {
            AppError::ComponentInitializationFailure("registry is unavailable".into())
        })?;
        RestorationEngineFactory::create(registry.active_engine())
    };

    let started = Instant::now();
    let processed = tauri::async_runtime::spawn_blocking(move || {
        let preview_operations = prepare_preview_operations(
            &validated_operations,
            full_source_dimensions,
            preview_source_dimensions,
        )?;
        engine.process(source.as_ref(), &preview_operations)
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

/// Exports the current document through the explicit float boundary as a true
/// 16-bit PNG. Legacy operations bridge through their existing RGBA8 engine;
/// `raw_development` operations remain linear until this final quantisation.
#[tauri::command]
pub async fn export_developed_png16(
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

    let _export_permit = state.export_gate.lock().await;
    let started = Instant::now();
    let (saved_path, width, height) = tauri::async_runtime::spawn_blocking(move || {
        let developed = apply_pipeline_float(source.as_ref(), &validated_operations)?;
        let dimensions = developed.dimensions();
        let saved_path = save_float_png16(&developed, &original_path, &output_path)?;
        Ok::<_, AppError>((saved_path, dimensions.0, dimensions.1))
    })
    .await
    .map_err(|_| AppError::ProcessingFailure("16-bit export worker stopped".into()))??;

    Ok(ExportResult {
        output_path: saved_path.to_string_lossy().into_owned(),
        width,
        height,
        processing_time_ms: started.elapsed().as_secs_f64() * 1_000.0,
    })
}
