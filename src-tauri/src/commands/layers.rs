use crate::application::AppState;
use crate::domain::{EditOperation, ExportProfile, ExportResult, ImageMetadata, PreviewResult};
use crate::error::AppError;
use crate::image_processing::{apply_pipeline, prepare_preview_operations};
use crate::infrastructure::{encode_preview, load_image, save_image_with_profile, LoadedImage};
use crate::layers::{
    layer_mask_to_selection, preview_dimensions, render_layers, selection_to_layer_mask, BlendMode,
    Layer, LayerDocument, LayerKind, LayerPixelStore, LoadedProject, RenderOptions, ResolvedPixels,
};
use crate::mask::{MaskBitmap, MaskSnapshot};
use image::{imageops, DynamicImage, Rgba, RgbaImage};
use serde::{Deserialize, Serialize};
use std::path::PathBuf;
use std::sync::atomic::Ordering;
use std::sync::Arc;
use std::time::Instant;
use tauri::State;

/// Largest thumbnail edge the Layers panel may request.
const MAX_THUMBNAIL_EDGE: u32 = 256;

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct LayerPixelsResult {
    pub pixel_id: String,
    pub width: u32,
    pub height: u32,
    pub filename: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct LayerThumbnailResult {
    pub layer_id: String,
    pub data_url: String,
    pub width: u32,
    pub height: u32,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ProjectSaveResult {
    pub output_path: String,
    pub bytes: u64,
    pub processing_time_ms: f64,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ProjectLoadResult {
    pub document_id: u64,
    pub is_current: bool,
    pub metadata: ImageMetadata,
    pub original_preview_data_url: String,
    pub preview_data_url: String,
    pub document: LayerDocument,
    pub operations: Vec<EditOperation>,
    pub canvas_width: u32,
    pub canvas_height: u32,
    pub application_version: String,
    pub created_at: String,
    pub modified_at: String,
    pub processing_time_ms: f64,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct LayerStoreReport {
    pub buffers: usize,
    pub released: usize,
    pub bytes: u64,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct LayerMaskResult {
    pub snapshot: MaskSnapshot,
    pub width: u32,
    pub height: u32,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct LayerTransformRequest {
    pub document: LayerDocument,
    pub layer_id: String,
}

fn session_document_id(state: &AppState) -> Result<u64, AppError> {
    let session = state
        .session
        .lock()
        .map_err(|_| AppError::ProcessingFailure("editor state is unavailable".into()))?;
    Ok(session.as_ref().ok_or(AppError::NoImageOpen)?.document_id)
}

/// Clones the `Arc` handles a render needs, then releases the store lock so no
/// pixel work ever happens while the session is locked.
fn resolve_pixels(
    state: &AppState,
    document: &LayerDocument,
    preview: bool,
) -> Result<(ResolvedPixels, f64), AppError> {
    let store = state
        .layers
        .lock()
        .map_err(|_| AppError::ProcessingFailure("layer store is unavailable".into()))?;
    let ids = document.referenced_pixel_ids();
    let resolved = store.resolve(&ids, preview)?;
    let scale = if preview { store.preview_scale() } else { 1.0 };
    Ok((resolved, scale))
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

/// Renders the visible composite of a layered document and then applies the
/// document-level pipeline on top of it.
///
/// Layer compositing and the document pipeline are deliberately separate
/// stages: per-layer transforms position layers inside the canvas, while
/// document crop, straighten, and perspective reshape the finished canvas.
#[tauri::command]
pub async fn render_layer_composite(
    document: LayerDocument,
    operations: Vec<EditOperation>,
    document_id: u64,
    request_id: u64,
    state: State<'_, AppState>,
) -> Result<PreviewResult, AppError> {
    document.validate()?;
    for operation in &operations {
        operation.validate()?;
    }
    let operation_count = operations.len();

    if state.pending_open_request.load(Ordering::Acquire) != 0 {
        return Ok(stale_preview(request_id, operation_count));
    }
    state
        .latest_layer_request
        .store(request_id, Ordering::Release);
    let _permit = state.layer_gate.lock().await;

    if state.latest_layer_request.load(Ordering::Acquire) != request_id
        || state.pending_open_request.load(Ordering::Acquire) != 0
        || session_document_id(&state)? != document_id
    {
        return Ok(stale_preview(request_id, operation_count));
    }

    let (resolved, scale) = resolve_pixels(&state, &document, true)?;
    let canvas = (document.canvas_width, document.canvas_height);
    let preview_canvas = preview_dimensions(canvas.0, canvas.1);

    let started = Instant::now();
    let composited = tauri::async_runtime::spawn_blocking(move || {
        let rendered = render_layers(
            &document.layers,
            document.canvas_width,
            document.canvas_height,
            &resolved,
            RenderOptions {
                scale,
                cancel: None,
            },
        )?;
        let prepared = prepare_preview_operations(&operations, canvas, preview_canvas)?;
        apply_pipeline(&DynamicImage::ImageRgba8(rendered), &prepared)
    })
    .await
    .map_err(|_| AppError::ProcessingFailure("layer render worker stopped".into()))??;

    let is_current = state.latest_layer_request.load(Ordering::Acquire) == request_id
        && state.pending_open_request.load(Ordering::Acquire) == 0
        && session_document_id(&state)? == document_id;
    let preview_data_url = if is_current {
        encode_preview(&composited)?
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

/// Renders the full-resolution composite and writes it with the chosen export
/// profile. The editable project is never modified by exporting.
#[tauri::command]
pub async fn export_layer_composite(
    output_path: String,
    document: LayerDocument,
    operations: Vec<EditOperation>,
    profile: ExportProfile,
    state: State<'_, AppState>,
) -> Result<ExportResult, AppError> {
    document.validate()?;
    for operation in &operations {
        operation.validate()?;
    }
    let _permit = state.export_gate.lock().await;

    let original_path = {
        let session = state
            .session
            .lock()
            .map_err(|_| AppError::ProcessingFailure("editor state is unavailable".into()))?;
        session
            .as_ref()
            .ok_or(AppError::NoImageOpen)?
            .source
            .path
            .clone()
    };
    let (resolved, _) = resolve_pixels(&state, &document, false)?;
    let output_path = PathBuf::from(output_path);

    let started = Instant::now();
    let (saved_path, width, height) = tauri::async_runtime::spawn_blocking(move || {
        let rendered = render_layers(
            &document.layers,
            document.canvas_width,
            document.canvas_height,
            &resolved,
            RenderOptions::default(),
        )?;
        let processed = apply_pipeline(&DynamicImage::ImageRgba8(rendered), &operations)?;
        let (width, height) = (processed.width(), processed.height());
        let saved = save_image_with_profile(&processed, &original_path, &output_path, profile)?;
        Ok::<_, AppError>((saved, width, height))
    })
    .await
    .map_err(|_| AppError::ProcessingFailure("layer export worker stopped".into()))??;

    Ok(ExportResult {
        output_path: saved_path.to_string_lossy().into_owned(),
        width,
        height,
        processing_time_ms: started.elapsed().as_secs_f64() * 1_000.0,
    })
}

/// Imports an image file as a new pixel buffer for an existing document.
#[tauri::command]
pub async fn import_layer_image(
    path: String,
    state: State<'_, AppState>,
) -> Result<LayerPixelsResult, AppError> {
    let input_path = PathBuf::from(path);
    let loaded = tauri::async_runtime::spawn_blocking(move || load_image(&input_path))
        .await
        .map_err(|_| AppError::ProcessingFailure("layer import worker stopped".into()))??;
    let filename = loaded.metadata.filename.clone();
    let image = loaded.original.to_rgba8();
    let (width, height) = (image.width(), image.height());

    let mut store = state
        .layers
        .lock()
        .map_err(|_| AppError::ProcessingFailure("layer store is unavailable".into()))?;
    let pixel_id = store.register(image)?;
    Ok(LayerPixelsResult {
        pixel_id,
        width,
        height,
        filename: Some(filename),
    })
}

/// Registers a fully transparent buffer for a new empty pixel layer.
#[tauri::command]
pub async fn create_layer_pixels(
    width: u32,
    height: u32,
    state: State<'_, AppState>,
) -> Result<LayerPixelsResult, AppError> {
    crate::layers::validate_dimensions(width, height)?;
    let image = RgbaImage::from_pixel(width, height, Rgba([0, 0, 0, 0]));
    let mut store = state
        .layers
        .lock()
        .map_err(|_| AppError::ProcessingFailure("layer store is unavailable".into()))?;
    let pixel_id = store.register(image)?;
    Ok(LayerPixelsResult {
        pixel_id,
        width,
        height,
        filename: None,
    })
}

fn find_layer(document: &LayerDocument, layer_id: &str) -> Result<Layer, AppError> {
    document
        .find(layer_id)
        .cloned()
        .ok_or_else(|| AppError::LayerNotFound(layer_id.to_string()))
}

/// Renders a subset of the tree into one new pixel buffer.
///
/// Merge and flatten both land here. The result is canvas sized with an
/// identity transform, because a merged layer no longer has the individual
/// placements of the layers that produced it.
async fn render_subset_into_buffer(
    document: LayerDocument,
    layers: Vec<Layer>,
    state: &AppState,
) -> Result<LayerPixelsResult, AppError> {
    let (resolved, _) = resolve_pixels(state, &document, false)?;
    let canvas_width = document.canvas_width;
    let canvas_height = document.canvas_height;
    let rendered = tauri::async_runtime::spawn_blocking(move || {
        render_layers(
            &layers,
            canvas_width,
            canvas_height,
            &resolved,
            RenderOptions::default(),
        )
    })
    .await
    .map_err(|_| AppError::ProcessingFailure("layer merge worker stopped".into()))??;

    let mut store = state
        .layers
        .lock()
        .map_err(|_| AppError::ProcessingFailure("layer store is unavailable".into()))?;
    let pixel_id = store.register(rendered)?;
    Ok(LayerPixelsResult {
        pixel_id,
        width: canvas_width,
        height: canvas_height,
        filename: None,
    })
}

/// Validates that a merge request can be represented by one ordinary pixel
/// layer without silently changing the visible composite. A contiguous normal
/// source-over range is associative and can be rendered against transparency;
/// a range that omits its backdrop cannot safely bake blend-dependent layers,
/// adjustments, or pass-through groups.
fn merge_targets(document: &LayerDocument, layer_ids: &[String]) -> Result<Vec<Layer>, AppError> {
    if layer_ids.is_empty() {
        return Err(AppError::InvalidLayerDocument(
            "merging requires at least one layer".into(),
        ));
    }
    let paths: Vec<Vec<usize>> = layer_ids
        .iter()
        .map(|id| {
            document
                .path_to(id)
                .ok_or_else(|| AppError::LayerNotFound(id.clone()))
        })
        .collect::<Result<_, _>>()?;
    let parent_path = paths[0][..paths[0].len() - 1].to_vec();
    if paths
        .iter()
        .any(|path| path.len() != parent_path.len() + 1 || path[..path.len() - 1] != parent_path)
    {
        return Err(AppError::InvalidLayerDocument(
            "merging requires layers from one sibling stack".into(),
        ));
    }
    let parent = if parent_path.is_empty() {
        None
    } else {
        document.layer_at(&parent_path)
    };
    let siblings: &[Layer] = match parent {
        Some(layer) => layer.children(),
        None => &document.layers,
    };
    let mut indices: Vec<usize> = paths.iter().map(|path| path[path.len() - 1]).collect();
    indices.sort_unstable();
    if indices.windows(2).any(|pair| pair[0] == pair[1])
        || indices
            .windows(2)
            .any(|pair| pair[1] != pair[0].saturating_add(1))
    {
        return Err(AppError::InvalidLayerDocument(
            "merging requires a contiguous sibling range".into(),
        ));
    }
    let selected: Vec<Layer> = indices
        .iter()
        .map(|index| {
            siblings
                .get(*index)
                .cloned()
                .ok_or_else(|| AppError::LayerNotFound(layer_ids[0].clone()))
        })
        .collect::<Result<_, _>>()?;

    let omitted_backdrop = indices[0] > 0 || parent.is_some_and(|layer| layer.is_pass_through());
    if omitted_backdrop
        && selected.iter().any(|layer| {
            layer.kind() == LayerKind::Adjustment
                || layer.blend_mode != BlendMode::Normal
                || layer.is_pass_through()
        })
    {
        return Err(AppError::InvalidLayerDocument(
            "this merge depends on layers beneath the selection because of a blend mode, adjustment layer, or pass-through group".into(),
        ));
    }
    Ok(selected)
}

/// Merges the named layers, bottom to top, into a single pixel buffer.
#[tauri::command]
pub async fn merge_layer_pixels(
    document: LayerDocument,
    layer_ids: Vec<String>,
    state: State<'_, AppState>,
) -> Result<LayerPixelsResult, AppError> {
    document.validate()?;
    // Preserve the document's own bottom-to-top order rather than the order the
    // caller listed identifiers in, so a merge can never reorder pixels.
    let ordered = merge_targets(&document, &layer_ids)?;
    render_subset_into_buffer(document, ordered, &state).await
}

/// Flattens every visible layer into a single pixel buffer.
#[tauri::command]
pub async fn flatten_layer_document(
    document: LayerDocument,
    state: State<'_, AppState>,
) -> Result<LayerPixelsResult, AppError> {
    document.validate()?;
    let layers = document.layers.clone();
    render_subset_into_buffer(document, layers, &state).await
}

/// Bakes a layer's transform into a new canvas-sized buffer so the layer can
/// return to an identity transform without losing its placement.
#[tauri::command]
pub async fn rasterize_layer_transform(
    request: LayerTransformRequest,
    state: State<'_, AppState>,
) -> Result<LayerPixelsResult, AppError> {
    request.document.validate()?;
    let mut layer = find_layer(&request.document, &request.layer_id)?;
    if layer.kind() != LayerKind::Pixel {
        return Err(AppError::InvalidLayerDocument(
            "only pixel layers can have their transform rasterized".into(),
        ));
    }
    // Bake placement and mask only. Opacity, blend mode and visibility remain
    // editable on the resulting layer and must not be applied a second time.
    layer.visible = true;
    layer.opacity = 1.0;
    layer.blend_mode = crate::layers::BlendMode::Normal;
    render_subset_into_buffer(request.document, vec![layer], &state).await
}

/// Applies operations destructively to one pixel layer, producing a new buffer
/// and leaving the original untouched so undo stays cheap.
#[tauri::command]
pub async fn apply_operations_to_layer(
    document: LayerDocument,
    layer_id: String,
    operations: Vec<EditOperation>,
    state: State<'_, AppState>,
) -> Result<LayerPixelsResult, AppError> {
    document.validate()?;
    for operation in &operations {
        operation.validate()?;
        if !operation.supports_adjustment_layer() {
            return Err(AppError::InvalidOperation(format!(
                "{} changes document geometry and cannot be applied to a single layer",
                operation.kind()
            )));
        }
    }
    let layer = find_layer(&document, &layer_id)?;
    if layer.locked {
        return Err(AppError::LayerLocked(layer_id));
    }
    let pixel_id = layer
        .pixel_id()
        .ok_or_else(|| {
            AppError::InvalidLayerDocument("only pixel layers accept destructive edits".into())
        })?
        .to_string();

    let source = {
        let store = state
            .layers
            .lock()
            .map_err(|_| AppError::ProcessingFailure("layer store is unavailable".into()))?;
        store.full(&pixel_id)?
    };

    let processed = tauri::async_runtime::spawn_blocking(move || {
        apply_pipeline(
            &DynamicImage::ImageRgba8(source.as_ref().clone()),
            &operations,
        )
    })
    .await
    .map_err(|_| AppError::ProcessingFailure("layer edit worker stopped".into()))??
    .to_rgba8();
    let (width, height) = (processed.width(), processed.height());

    let mut store = state
        .layers
        .lock()
        .map_err(|_| AppError::ProcessingFailure("layer store is unavailable".into()))?;
    let pixel_id = store.register(processed)?;
    Ok(LayerPixelsResult {
        pixel_id,
        width,
        height,
        filename: None,
    })
}

/// Renders the thumbnail shown for one layer in the Layers panel.
#[tauri::command]
pub async fn render_layer_thumbnail(
    document: LayerDocument,
    layer_id: String,
    max_edge: u32,
    state: State<'_, AppState>,
) -> Result<LayerThumbnailResult, AppError> {
    document.validate()?;
    let max_edge = max_edge.clamp(16, MAX_THUMBNAIL_EDGE);
    let layer = find_layer(&document, &layer_id)?;

    let rendered = match layer.kind() {
        // An adjustment layer has no pixels of its own; the panel shows a typed
        // badge instead of a thumbnail.
        LayerKind::Adjustment => {
            return Ok(LayerThumbnailResult {
                layer_id,
                data_url: String::new(),
                width: 0,
                height: 0,
            })
        }
        LayerKind::Pixel => {
            let pixel_id = layer.pixel_id().unwrap_or_default().to_string();
            let store = state
                .layers
                .lock()
                .map_err(|_| AppError::ProcessingFailure("layer store is unavailable".into()))?;
            let resolved = store.resolve(std::slice::from_ref(&pixel_id), true)?;
            drop(store);
            crate::layers::PixelSource::resolve(&resolved, &pixel_id)?
                .as_ref()
                .clone()
        }
        LayerKind::Group => {
            let (resolved, scale) = resolve_pixels(&state, &document, true)?;
            let children = layer.children().to_vec();
            let canvas = (document.canvas_width, document.canvas_height);
            tauri::async_runtime::spawn_blocking(move || {
                render_layers(
                    &children,
                    canvas.0,
                    canvas.1,
                    &resolved,
                    RenderOptions {
                        scale,
                        cancel: None,
                    },
                )
            })
            .await
            .map_err(|_| AppError::ProcessingFailure("thumbnail worker stopped".into()))??
        }
    };

    let (width, height) = rendered.dimensions();
    let longest = width.max(height).max(1);
    let (target_width, target_height) = if longest <= max_edge {
        (width, height)
    } else {
        let ratio = f64::from(max_edge) / f64::from(longest);
        (
            ((f64::from(width) * ratio).round() as u32).max(1),
            ((f64::from(height) * ratio).round() as u32).max(1),
        )
    };
    let thumbnail = if (target_width, target_height) == (width, height) {
        rendered
    } else {
        imageops::thumbnail(&rendered, target_width, target_height)
    };

    Ok(LayerThumbnailResult {
        layer_id,
        data_url: encode_preview(&DynamicImage::ImageRgba8(thumbnail))?,
        width: target_width,
        height: target_height,
    })
}

/// Converts the active canvas selection into a mask in one layer's own space.
#[tauri::command]
pub async fn layer_mask_from_selection(
    document: LayerDocument,
    layer_id: String,
    selection: MaskSnapshot,
) -> Result<LayerMaskResult, AppError> {
    document.validate()?;
    let layer = find_layer(&document, &layer_id)?;
    let decoded = selection.decode()?;
    let mask = tauri::async_runtime::spawn_blocking(move || {
        selection_to_layer_mask(
            &decoded,
            &layer,
            document.canvas_width,
            document.canvas_height,
        )
    })
    .await
    .map_err(|_| AppError::ProcessingFailure("mask conversion worker stopped".into()))??;
    Ok(LayerMaskResult {
        width: mask.width(),
        height: mask.height(),
        snapshot: MaskSnapshot::encode(&mask),
    })
}

/// Converts one layer's mask back into a canvas-space selection.
#[tauri::command]
pub async fn selection_from_layer_mask(
    document: LayerDocument,
    layer_id: String,
) -> Result<LayerMaskResult, AppError> {
    document.validate()?;
    let layer = find_layer(&document, &layer_id)?;
    let mask = layer
        .mask
        .as_ref()
        .ok_or_else(|| AppError::InvalidLayerDocument("that layer has no mask".into()))?
        .snapshot
        .decode()?;
    let selection = tauri::async_runtime::spawn_blocking(move || {
        layer_mask_to_selection(&mask, &layer, document.canvas_width, document.canvas_height)
    })
    .await
    .map_err(|_| AppError::ProcessingFailure("mask conversion worker stopped".into()))??;
    Ok(LayerMaskResult {
        width: selection.width(),
        height: selection.height(),
        snapshot: MaskSnapshot::encode(&selection),
    })
}

/// Builds a uniform mask for a layer: white reveals everything, black hides it.
#[tauri::command]
pub async fn create_layer_mask(
    document: LayerDocument,
    layer_id: String,
    filled: bool,
) -> Result<LayerMaskResult, AppError> {
    document.validate()?;
    let layer = find_layer(&document, &layer_id)?;
    let (width, height) =
        crate::layers::mask_space(&layer, document.canvas_width, document.canvas_height);
    let mask = if filled {
        MaskBitmap::full(width, height)?
    } else {
        MaskBitmap::empty(width, height)?
    };
    Ok(LayerMaskResult {
        width,
        height,
        snapshot: MaskSnapshot::encode(&mask),
    })
}

/// Revalidates a layer document at the trust boundary.
#[tauri::command]
pub async fn validate_layer_document(document: LayerDocument) -> Result<usize, AppError> {
    document.validate()?;
    Ok(document.layer_count())
}

/// Releases pixel buffers no longer reachable from the document or its history.
#[tauri::command]
pub async fn retain_layer_pixels(
    pixel_ids: Vec<String>,
    document_id: u64,
    state: State<'_, AppState>,
) -> Result<LayerStoreReport, AppError> {
    let session = state
        .session
        .lock()
        .map_err(|_| AppError::ProcessingFailure("editor state is unavailable".into()))?;
    let mut store = state
        .layers
        .lock()
        .map_err(|_| AppError::ProcessingFailure("layer store is unavailable".into()))?;
    let is_current = session
        .as_ref()
        .is_some_and(|session| session.document_id == document_id)
        && state.pending_open_request.load(Ordering::Acquire) == 0;
    let released = if is_current {
        store.retain(&pixel_ids)
    } else {
        0
    };
    Ok(LayerStoreReport {
        buffers: store.buffer_count(),
        released,
        bytes: store.total_bytes(),
    })
}

#[tauri::command]
pub async fn layer_store_report(state: State<'_, AppState>) -> Result<LayerStoreReport, AppError> {
    let store = state
        .layers
        .lock()
        .map_err(|_| AppError::ProcessingFailure("layer store is unavailable".into()))?;
    Ok(LayerStoreReport {
        buffers: store.buffer_count(),
        released: 0,
        bytes: store.total_bytes(),
    })
}

/// Writes an editable project file. Pixel buffers are pulled from the session
/// store rather than trusted from the request.
#[tauri::command]
pub async fn save_layer_project(
    output_path: String,
    document: LayerDocument,
    operations: Vec<EditOperation>,
    created_at: String,
    modified_at: String,
    state: State<'_, AppState>,
) -> Result<ProjectSaveResult, AppError> {
    document.validate()?;
    let _permit = state.export_gate.lock().await;
    let path = PathBuf::from(output_path);

    let buffers = {
        let store = state
            .layers
            .lock()
            .map_err(|_| AppError::ProcessingFailure("layer store is unavailable".into()))?;
        let mut buffers = Vec::new();
        for pixel_id in document.referenced_pixel_ids() {
            let image = store.full(&pixel_id)?;
            buffers.push((pixel_id, image));
        }
        buffers
    };

    let started = Instant::now();
    let (saved_path, bytes) = tauri::async_runtime::spawn_blocking(move || {
        let borrowed: Vec<(String, &RgbaImage)> = buffers
            .iter()
            .map(|(id, image)| (id.clone(), image.as_ref()))
            .collect();
        let bytes = crate::layers::save_project(
            &path,
            &document,
            &operations,
            &borrowed,
            env!("CARGO_PKG_VERSION"),
            &created_at,
            &modified_at,
        )?;
        Ok::<_, AppError>((path, bytes))
    })
    .await
    .map_err(|_| AppError::ProcessingFailure("project save worker stopped".into()))??;

    Ok(ProjectSaveResult {
        output_path: saved_path.to_string_lossy().into_owned(),
        bytes,
        processing_time_ms: started.elapsed().as_secs_f64() * 1_000.0,
    })
}

/// A fully staged replacement. Preparing it never touches the current session.
struct PreparedProject {
    store: LayerPixelStore,
    source: LoadedImage,
    result: ProjectLoadResult,
}

fn prepare_project(
    loaded: LoadedProject,
    path: PathBuf,
    request_id: u64,
) -> Result<PreparedProject, AppError> {
    loaded.document.validate()?;
    let (width, height) = (loaded.document.canvas_width, loaded.document.canvas_height);
    let mut store = LayerPixelStore::default();
    store.reset(width, height)?;
    for (pixel_id, image) in loaded.pixels {
        store.register_with_id(&pixel_id, image)?;
    }
    let resolved = store.resolve(&loaded.document.referenced_pixel_ids(), false)?;
    let original = Arc::new(DynamicImage::ImageRgba8(render_layers(
        &loaded.document.layers,
        width,
        height,
        &resolved,
        RenderOptions::default(),
    )?));
    let (preview_width, preview_height) = preview_dimensions(width, height);
    let preview = if (preview_width, preview_height) == (width, height) {
        Arc::clone(&original)
    } else {
        Arc::new(original.thumbnail(preview_width, preview_height))
    };
    let original_preview_data_url = encode_preview(&preview)?;
    let operations = prepare_preview_operations(
        &loaded.document_operations,
        (width, height),
        (preview_width, preview_height),
    )?;
    let preview_data_url = if operations.is_empty() {
        original_preview_data_url.clone()
    } else {
        encode_preview(&apply_pipeline(&preview, &operations)?)?
    };
    let metadata = ImageMetadata {
        filename: path
            .file_name()
            .and_then(|name| name.to_str())
            .unwrap_or("PhotoForge project")
            .into(),
        width,
        height,
        format: "PhotoForge".into(),
        file_size: std::fs::metadata(&path)
            .map(|value| value.len())
            .unwrap_or(0),
        color_space: "sRGB".into(),
        bit_depth: 8,
        has_alpha: true,
        created_at: Some(loaded.created_at.clone()).filter(|value| !value.is_empty()),
        modified_at: Some(loaded.modified_at.clone()).filter(|value| !value.is_empty()),
        camera_model: None,
        exif_available: false,
    };
    Ok(PreparedProject {
        source: LoadedImage {
            path,
            original,
            preview,
            metadata: metadata.clone(),
        },
        store,
        result: ProjectLoadResult {
            document_id: request_id,
            is_current: false,
            metadata,
            original_preview_data_url,
            preview_data_url,
            canvas_width: width,
            canvas_height: height,
            document: loaded.document,
            operations: loaded.document_operations,
            application_version: loaded.application_version,
            created_at: loaded.created_at,
            modified_at: loaded.modified_at,
            processing_time_ms: 0.0,
        },
    })
}

async fn open_project(
    path: String,
    request_id: u64,
    recovery: bool,
    state: &AppState,
) -> Result<ProjectLoadResult, AppError> {
    let started = Instant::now();
    let request = super::editor::OpenRequest::begin(state, request_id)?;
    let input_path = PathBuf::from(path);
    let mut prepared = tauri::async_runtime::spawn_blocking(move || {
        let loaded = if recovery {
            crate::layers::read_managed_snapshot(&input_path)?
        } else {
            crate::layers::load_project(&input_path)?
        };
        prepare_project(loaded, input_path, request_id)
    })
    .await
    .map_err(|_| AppError::ProcessingFailure("project load worker stopped".into()))??;
    prepared.result.is_current = request.commit(prepared.source, prepared.store)?;
    if !prepared.result.is_current {
        prepared.result.original_preview_data_url.clear();
        prepared.result.preview_data_url.clear();
    }
    prepared.result.processing_time_ms = started.elapsed().as_secs_f64() * 1_000.0;
    Ok(prepared.result)
}

/// Reads an editable project and atomically installs its source and pixel store.
#[tauri::command]
pub async fn load_layer_project(
    path: String,
    request_id: u64,
    state: State<'_, AppState>,
) -> Result<ProjectLoadResult, AppError> {
    open_project(path, request_id, false, &state).await
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RecoveryListResult {
    pub snapshots: Vec<crate::layers::RecoveryRecord>,
}

/// Writes a periodic recovery snapshot of the working document.
///
/// The snapshot lands in the local recovery folder under its own extension and
/// never touches the user's project file.
#[tauri::command]
pub async fn write_recovery_snapshot(
    document: LayerDocument,
    operations: Vec<EditOperation>,
    project_path: Option<String>,
    document_name: String,
    saved_at: String,
    state: State<'_, AppState>,
) -> Result<crate::layers::RecoveryRecord, AppError> {
    document.validate()?;
    let buffers = {
        let store = state
            .layers
            .lock()
            .map_err(|_| AppError::ProcessingFailure("layer store is unavailable".into()))?;
        let mut buffers = Vec::new();
        for pixel_id in document.referenced_pixel_ids() {
            buffers.push((pixel_id.clone(), store.full(&pixel_id)?));
        }
        buffers
    };

    tauri::async_runtime::spawn_blocking(move || {
        let borrowed: Vec<(String, &RgbaImage)> = buffers
            .iter()
            .map(|(id, image)| (id.clone(), image.as_ref()))
            .collect();
        crate::layers::write_recovery_snapshot(
            &document,
            &operations,
            &borrowed,
            project_path.as_deref(),
            &document_name,
            env!("CARGO_PKG_VERSION"),
            &saved_at,
            None,
        )
    })
    .await
    .map_err(|_| AppError::ProcessingFailure("recovery worker stopped".into()))?
}

#[tauri::command]
pub async fn list_recovery_snapshots() -> Result<RecoveryListResult, AppError> {
    Ok(RecoveryListResult {
        snapshots: crate::layers::list_recovery_snapshots(None)?,
    })
}

/// Restores a snapshot and rebinds the session pixel store to it.
#[tauri::command]
pub async fn restore_recovery_snapshot(
    path: String,
    request_id: u64,
    state: State<'_, AppState>,
) -> Result<ProjectLoadResult, AppError> {
    open_project(path, request_id, true, &state).await
}

#[tauri::command]
pub async fn discard_recovery_snapshot(path: Option<String>) -> Result<usize, AppError> {
    match path {
        Some(path) => {
            crate::layers::discard_managed_snapshot(&PathBuf::from(path))?;
            Ok(1)
        }
        None => crate::layers::discard_recovery_snapshots(None),
    }
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct LayerWorkflowPlan {
    /// Resolved layer identifier per step; empty where a step has no target.
    pub targets: Vec<String>,
    pub steps: usize,
}

/// Resolves every selector in a layer workflow against the current document
/// before any of it runs, so a replay never half-applies or silently retargets.
#[tauri::command]
pub async fn plan_layer_workflow(
    document: LayerDocument,
    steps: Vec<crate::layers::LayerWorkflowStep>,
) -> Result<LayerWorkflowPlan, AppError> {
    document.validate()?;
    let targets = crate::layers::plan_layer_steps(&document, &steps)?;
    for (step, target) in steps.iter().zip(&targets) {
        if !target.is_empty() {
            crate::layers::check_layer_step_kind(&document, step, target)?;
        }
    }
    Ok(LayerWorkflowPlan {
        steps: steps.len(),
        targets,
    })
}

/// Registers every layer command on a Tauri builder.
///
/// Exposed so integration tests can stand up an application that dispatches
/// these commands the way the running program does — through the IPC boundary,
/// with real argument deserialization and real managed state — rather than by
/// calling the functions directly and proving nothing about the wiring. The
/// shipped binary registers its commands in `lib.rs` and never calls this.
#[doc(hidden)]
pub fn register_layer_commands<R: tauri::Runtime>(builder: tauri::Builder<R>) -> tauri::Builder<R> {
    builder.invoke_handler(tauri::generate_handler![
        render_layer_composite,
        export_layer_composite,
        import_layer_image,
        create_layer_pixels,
        merge_layer_pixels,
        flatten_layer_document,
        rasterize_layer_transform,
        apply_operations_to_layer,
        render_layer_thumbnail,
        layer_mask_from_selection,
        selection_from_layer_mask,
        create_layer_mask,
        validate_layer_document,
        retain_layer_pixels,
        layer_store_report,
        save_layer_project,
        load_layer_project,
        write_recovery_snapshot,
        list_recovery_snapshots,
        restore_recovery_snapshot,
        discard_recovery_snapshot,
        plan_layer_workflow,
        super::editor::analyze_image,
        super::professional::generate_histogram,
        super::professional::inspect_image_pixel,
        super::professional::create_point_operation,
        super::mask::magic_wand_selection,
        super::mask::color_range_selection,
        super::mask::refine_selection_mask
    ])
}

#[cfg(test)]
mod project_session_tests {
    use super::*;
    use crate::layers::test_pixel_layer;

    fn project(pixel_id: &str, value: u8) -> LoadedProject {
        LoadedProject {
            document: LayerDocument {
                schema_version: crate::layers::LAYER_SCHEMA_VERSION,
                canvas_width: 2,
                canvas_height: 2,
                layers: vec![test_pixel_layer("background", pixel_id, 2, 2)],
                active_layer_id: Some("background".into()),
            },
            document_operations: vec![],
            pixels: vec![(
                pixel_id.into(),
                RgbaImage::from_pixel(2, 2, Rgba([value, 0, 0, 255])),
            )],
            application_version: "test".into(),
            created_at: String::new(),
            modified_at: String::new(),
        }
    }

    #[test]
    fn failed_staging_preserves_the_previous_session_and_all_pixel_buffers() {
        let state = AppState::default();
        let prepared = prepare_project(project("px1", 10), "old.photoforge".into(), 1).unwrap();
        let old = super::super::editor::OpenRequest::begin(&state, 1).unwrap();
        assert!(old.commit(prepared.source, prepared.store).unwrap());
        drop(old);
        {
            let _new = super::super::editor::OpenRequest::begin(&state, 2).unwrap();
            let mut invalid = project("px2", 20);
            // One valid staged buffer is followed by an invalid registration.
            invalid
                .pixels
                .push(("../escape".into(), RgbaImage::new(2, 2)));
            assert!(prepare_project(invalid, "bad.photoforge".into(), 2).is_err());
        }
        assert_eq!(state.pending_open_request.load(Ordering::Acquire), 0);
        assert_eq!(session_document_id(&state).unwrap(), 1);
        let store = state.layers.lock().unwrap();
        assert!(store.contains("px1"));
        assert!(!store.contains("px2"));
        assert_eq!(store.full("px1").unwrap().get_pixel(0, 0).0[0], 10);
    }

    #[test]
    fn stale_open_cannot_commit_or_clear_the_newer_requests_pending_flag() {
        let state = AppState::default();
        let old = super::super::editor::OpenRequest::begin(&state, 1).unwrap();
        let old_prepared = prepare_project(project("px1", 10), "old.photoforge".into(), 1).unwrap();
        let current = super::super::editor::OpenRequest::begin(&state, 2).unwrap();
        assert!(!old.commit(old_prepared.source, old_prepared.store).unwrap());
        drop(old);
        assert_eq!(state.pending_open_request.load(Ordering::Acquire), 2);
        assert!(state.session.lock().unwrap().is_none());
        let prepared = prepare_project(project("px2", 20), "new.photoforge".into(), 2).unwrap();
        assert!(current.commit(prepared.source, prepared.store).unwrap());
        drop(current);
        assert_eq!(state.pending_open_request.load(Ordering::Acquire), 0);
        assert_eq!(session_document_id(&state).unwrap(), 2);
        assert!(state.layers.lock().unwrap().contains("px2"));
    }

    #[test]
    fn zero_open_identifier_is_rejected_without_setting_pending() {
        let state = AppState::default();
        assert!(super::super::editor::OpenRequest::begin(&state, 0).is_err());
        assert_eq!(state.pending_open_request.load(Ordering::Acquire), 0);
        assert!(state.session.lock().unwrap().is_none());
    }
}
