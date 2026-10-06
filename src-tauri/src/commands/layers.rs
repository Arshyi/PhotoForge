use crate::application::AppState;
use crate::domain::{EditOperation, ExportProfile, ExportResult, ImageMetadata, PreviewResult};
use crate::error::AppError;
use crate::image_processing::high_precision::pipeline_typed;
use crate::image_processing::{apply_pipeline, prepare_preview_operations};
use crate::infrastructure::{
    encode_preview, load_image, save_color_image_streaming, save_image_with_profile, LoadedImage,
};
use crate::layers::{
    layer_mask_to_selection, preview_dimensions, selection_to_layer_mask, BlendMode, Layer,
    LayerDocument, LayerKind, LayerPixelStore, LoadedProject, RenderOptions, ResolvedPixels,
};
use crate::layers::{
    render_document_streaming, render_document_typed, render_document_typed_cached,
    DEFAULT_TILE_SIZE,
};
use crate::mask::{MaskBitmap, MaskSnapshot};
use crate::pixel::{DocumentPrecision, PixelBuffer};
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
    pub raw: Option<crate::raw::RawLayerSource>,
    /// Set when the pixels are a region or reduced copy of a larger file.
    pub origin: Option<crate::source::SourceOrigin>,
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
    // The interactive path is the one that re-renders the same document over
    // and over — a panel opening, an undo, a pointer release that changes
    // nothing. Export and merge deliberately do not share it: they run once
    // per document and would only evict tiles the interface still wants.
    let cache = Arc::clone(&state.render_cache);
    let composited = tauri::async_runtime::spawn_blocking(move || {
        let _job = crate::resources::acquire_job(None)?;
        let rendered = render_document_typed_cached(
            &document,
            &resolved,
            RenderOptions {
                scale,
                cancel: None,
            },
            Some(cache.as_ref()),
        )?;
        let prepared = prepare_preview_operations(&operations, canvas, preview_canvas)?;
        let processed = pipeline_typed(rendered, &prepared, None)?;
        Ok::<_, AppError>(DynamicImage::ImageRgba8(
            processed.encoded8().as_ref().clone(),
        ))
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
    color: Option<crate::color_management::ColorExportOptions>,
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
        let _job = crate::resources::acquire_job(None)?;
        // Protect both RAW inputs and explicit smart links, including content
        // nested inside a source stack. An export must never replace a file
        // that the current project still treats as an input authority.
        for layer in document.iter_all() {
            if let Some(raw) = &layer.raw {
                if let Some(path) = raw.linked_path() {
                    if std::fs::canonicalize(path).ok() == std::fs::canonicalize(&output_path).ok()
                        && output_path.exists()
                    {
                        return Err(AppError::InvalidOutputPath);
                    }
                }
            }
        }
        for source in document.smart_sources.values() {
            if let Some(link) = &source.link {
                if std::fs::canonicalize(&link.path).ok()
                    == std::fs::canonicalize(&output_path).ok()
                    && output_path.exists()
                {
                    return Err(AppError::InvalidOutputPath);
                }
            }
        }
        let streams_directly = document.precision == DocumentPrecision::LinearSrgbF32
            && operations.is_empty()
            && output_path
                .extension()
                .and_then(|extension| extension.to_str())
                .is_some_and(|extension| extension.eq_ignore_ascii_case("png"));
        if streams_directly {
            let saved = save_color_image_streaming(
                document.canvas_width,
                document.canvas_height,
                &original_path,
                &output_path,
                profile,
                color.unwrap_or(crate::color_management::ColorExportOptions {
                    bit_depth: 8,
                    ..Default::default()
                }),
                None,
                |emit| {
                    render_document_streaming(
                        &document,
                        &resolved,
                        RenderOptions::default(),
                        DEFAULT_TILE_SIZE,
                        0,
                        emit,
                    )
                    .map(|_| ())
                },
            )?;
            return Ok((saved, document.canvas_width, document.canvas_height));
        }

        let rendered = render_document_typed(&document, &resolved, RenderOptions::default())?;
        if document.precision == DocumentPrecision::LinearSrgbF32 {
            crate::resources::ResourceEstimate::pipeline(
                document.canvas_width,
                document.canvas_height,
                &operations,
                crate::layers::PixelSource::resident_bytes(&resolved) + rendered.bytes(),
            )?;
        }
        let processed = pipeline_typed(rendered, &operations, None)?;
        let (width, height) = processed.dimensions();
        let saved = match processed {
            PixelBuffer::LinearRgbaF32(image) => crate::infrastructure::save_color_image(
                &image,
                &original_path,
                &output_path,
                profile,
                color.unwrap_or(crate::color_management::ColorExportOptions {
                    bit_depth: 8,
                    ..Default::default()
                }),
                None,
            )?,
            PixelBuffer::EncodedSrgba8(image) => {
                if let Some(color) = color {
                    crate::infrastructure::save_color_image(
                        &crate::color::FloatImage::from_rgba8(&image)?,
                        &original_path,
                        &output_path,
                        profile,
                        color,
                        None,
                    )?
                } else {
                    save_image_with_profile(
                        &DynamicImage::ImageRgba8(image.as_ref().clone()),
                        &original_path,
                        &output_path,
                        profile,
                    )?
                }
            }
        };
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
    if std::path::Path::new(&path)
        .extension()
        .and_then(|s| s.to_str())
        .is_some_and(|s| s.eq_ignore_ascii_case("dng"))
    {
        let result = super::raw::open_raw_layer(path, None, true, state).await?;
        return Ok(LayerPixelsResult {
            pixel_id: result.pixel_id,
            width: result.width,
            height: result.height,
            filename: Some(result.source.reference.filename.clone()),
            raw: Some(result.source),
            origin: None,
        });
    }
    let input_path = PathBuf::from(path);
    let loaded = tauri::async_runtime::spawn_blocking(move || {
        let _job = crate::resources::acquire_job(None)?;
        load_image(&input_path)
    })
    .await
    .map_err(|_| AppError::ProcessingFailure("layer import worker stopped".into()))??;
    let filename = loaded.metadata.filename.clone();
    let image = match loaded.working {
        Some(working) => PixelBuffer::LinearRgbaF32(working),
        None => loaded.original.to_rgba8().into(),
    };
    let (width, height) = image.dimensions();

    let mut store = state
        .layers
        .lock()
        .map_err(|_| AppError::ProcessingFailure("layer store is unavailable".into()))?;
    let pixel_id = store.register_typed(image)?;
    Ok(LayerPixelsResult {
        pixel_id,
        width,
        height,
        filename: Some(filename),
        raw: None,
        origin: None,
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
        raw: None,
        origin: None,
    })
}

pub(crate) fn find_layer(document: &LayerDocument, layer_id: &str) -> Result<Layer, AppError> {
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
pub(crate) async fn render_subset_into_buffer(
    document: LayerDocument,
    layers: Vec<Layer>,
    state: &AppState,
) -> Result<LayerPixelsResult, AppError> {
    let (resolved, _) = resolve_pixels(state, &document, false)?;
    let canvas_width = document.canvas_width;
    let canvas_height = document.canvas_height;
    let rendered = tauri::async_runtime::spawn_blocking(move || {
        let _job = crate::resources::acquire_job(None)?;
        let mut subset = document;
        subset.layers = layers;
        subset.active_layer_id = None;
        render_document_typed(&subset, &resolved, RenderOptions::default())
    })
    .await
    .map_err(|_| AppError::ProcessingFailure("layer merge worker stopped".into()))??;

    let mut store = state
        .layers
        .lock()
        .map_err(|_| AppError::ProcessingFailure("layer store is unavailable".into()))?;
    let pixel_id = store.register_typed(rendered)?;
    Ok(LayerPixelsResult {
        pixel_id,
        width: canvas_width,
        height: canvas_height,
        filename: None,
        raw: None,
        origin: None,
    })
}

/// Validates that a merge request can be represented by one ordinary pixel
/// layer without silently changing the visible composite. A contiguous normal
/// source-over range is associative and can be rendered against transparency;
/// a range that omits its backdrop cannot safely bake blend-dependent layers,
/// adjustments, or pass-through groups.
pub(crate) fn merge_targets(
    document: &LayerDocument,
    layer_ids: &[String],
) -> Result<Vec<Layer>, AppError> {
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
    apply_edit_to_layer(&document, &layer_id, operations, &state).await
}

/// Applies operations destructively to one pixel layer and registers the result
/// as a new buffer. Shared by the command above and by the operation engine, so
/// a macro, a plugin or a planner edits a layer by exactly the path the Layers
/// panel does.
pub(crate) async fn apply_edit_to_layer(
    document: &LayerDocument,
    layer_id: &str,
    operations: Vec<EditOperation>,
    state: &AppState,
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
    let layer = find_layer(document, layer_id)?;
    if layer.locked {
        return Err(AppError::LayerLocked(layer.name.clone()));
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
        let source = store.full_typed(&pixel_id)?;
        if document.precision == DocumentPrecision::LinearSrgbF32 {
            PixelBuffer::LinearRgbaF32(source.linear()?)
        } else {
            PixelBuffer::EncodedSrgba8(source.encoded8())
        }
    };

    let processed = tauri::async_runtime::spawn_blocking(move || {
        let _job = crate::resources::acquire_job(None)?;
        pipeline_typed(source, &operations, None)
    })
    .await
    .map_err(|_| AppError::ProcessingFailure("layer edit worker stopped".into()))??;
    let (width, height) = processed.dimensions();

    let mut store = state
        .layers
        .lock()
        .map_err(|_| AppError::ProcessingFailure("layer store is unavailable".into()))?;
    let pixel_id = store.register_typed(processed)?;
    Ok(LayerPixelsResult {
        pixel_id,
        width,
        height,
        filename: None,
        raw: None,
        origin: None,
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
        // Neither a shape nor a text layer has a buffer to crop a thumbnail
        // from, so each is rendered on its own over the canvas and thumbnailed
        // from that. Rendering the layer alone is what makes the thumbnail show
        // the layer rather than whatever happens to sit behind it.
        LayerKind::Shape | LayerKind::Text | LayerKind::SmartObject => {
            let mut solo = document.clone();
            solo.layers = vec![layer.clone()];
            solo.active_layer_id = None;
            // One lock for both, released before rendering: holding the store
            // across a render would block every other layer command for the
            // duration of it.
            let (resolved, scale) = {
                let store = state.layers.lock().map_err(|_| {
                    AppError::ProcessingFailure("layer store is unavailable".into())
                })?;
                (
                    store.resolve(&solo.referenced_pixel_ids(), true)?,
                    store.preview_scale(),
                )
            };
            let typed = crate::layers::render_document_typed(
                &solo,
                &resolved,
                RenderOptions {
                    scale,
                    cancel: None,
                },
            )?;
            (*typed.encoded8()).clone()
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
            let mut thumbnail_document = document.clone();
            thumbnail_document.layers = layer.children().to_vec();
            thumbnail_document.active_layer_id = None;
            tauri::async_runtime::spawn_blocking(move || {
                render_document_typed(
                    &thumbnail_document,
                    &resolved,
                    RenderOptions {
                        scale,
                        cancel: None,
                    },
                )
                .map(|image| (*image.encoded8()).clone())
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
    mask_for_layer(&document, &layer_id, &selection).await
}

/// Converts a canvas selection into a mask in one layer's own space. Shared with
/// the operation engine for the same reason as `apply_edit_to_layer`.
pub(crate) async fn mask_for_layer(
    document: &LayerDocument,
    layer_id: &str,
    selection: &MaskSnapshot,
) -> Result<LayerMaskResult, AppError> {
    document.validate()?;
    let layer = find_layer(document, layer_id)?;
    let decoded = selection.decode()?;
    let layer = crate::layers::mask_geometry_layer(document, &layer)?;
    let (canvas_width, canvas_height) = (document.canvas_width, document.canvas_height);
    let mask = tauri::async_runtime::spawn_blocking(move || {
        selection_to_layer_mask(&decoded, &layer, canvas_width, canvas_height)
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
    let layer = crate::layers::mask_geometry_layer(&document, &layer)?;
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
    let layer = crate::layers::mask_geometry_layer(&document, &layer)?;
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
            let image = store.full_typed(&pixel_id)?;
            buffers.push((pixel_id, image));
        }
        buffers
    };

    let started = Instant::now();
    let (saved_path, bytes) = tauri::async_runtime::spawn_blocking(move || {
        let _job = crate::resources::acquire_job(None)?;
        let bytes = crate::layers::save_project_typed(
            &path,
            &document,
            &operations,
            &buffers,
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
    for (pixel_id, image) in loaded.linear_pixels {
        store.register_typed_with_id(&pixel_id, image.into())?;
    }
    let resolved = store.resolve(&loaded.document.referenced_pixel_ids(), false)?;
    let typed = render_document_typed(&loaded.document, &resolved, RenderOptions::default())?;
    let working = match &typed {
        PixelBuffer::LinearRgbaF32(image) => Some(Arc::clone(image)),
        _ => None,
    };
    let original = Arc::new(DynamicImage::ImageRgba8(typed.encoded8().as_ref().clone()));
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
    let preview_data_url = if let Some(working) = &working {
        let prepared = working.resized(preview_width, preview_height)?;
        let processed =
            crate::image_processing::high_precision::pipeline(prepared, &operations, None)?;
        encode_preview(&DynamicImage::ImageRgba8(processed.to_rgba8()))?
    } else if operations.is_empty() {
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
        bit_depth: if working.is_some() { 32 } else { 8 },
        has_alpha: true,
        created_at: Some(loaded.created_at.clone()).filter(|value| !value.is_empty()),
        modified_at: Some(loaded.modified_at.clone()).filter(|value| !value.is_empty()),
        camera_model: None,
        exif_available: false,
        raw: None,
        origin: None,
    };
    Ok(PreparedProject {
        source: LoadedImage {
            path,
            original,
            preview,
            metadata: metadata.clone(),
            working,
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
        let _job = crate::resources::acquire_job(None)?;
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

/// Opens a new transparent, high-precision document without creating a file.
/// Like project loading, the replacement is staged before the session changes,
/// and a superseded request never replaces a more recent open/new operation.
#[tauri::command]
pub async fn create_blank_layer_document(
    width: u32,
    height: u32,
    request_id: u64,
    state: State<'_, AppState>,
) -> Result<ProjectLoadResult, AppError> {
    crate::layers::validate_dimensions(width, height)?;
    let started = Instant::now();
    let request = super::editor::OpenRequest::begin(&state, request_id)?;
    let mut prepared = tauri::async_runtime::spawn_blocking(move || {
        let _job = crate::resources::acquire_job(None)?;
        let mut document = LayerDocument::new(width, height);
        document.precision = crate::pixel::DocumentPrecision::LinearSrgbF32;
        let loaded = LoadedProject {
            document,
            document_operations: Vec::new(),
            pixels: Vec::new(),
            linear_pixels: Vec::new(),
            application_version: env!("CARGO_PKG_VERSION").into(),
            created_at: String::new(),
            modified_at: String::new(),
        };
        // An empty path denotes an unsaved source, not a real input file.
        let mut prepared = prepare_project(loaded, PathBuf::new(), request_id)?;
        prepared.source.metadata.filename = "Untitled".into();
        prepared.result.metadata.filename = "Untitled".into();
        Ok::<_, AppError>(prepared)
    })
    .await
    .map_err(|_| AppError::ProcessingFailure("new document worker stopped".into()))??;
    prepared.result.is_current = request.commit(prepared.source, prepared.store)?;
    if !prepared.result.is_current {
        prepared.result.original_preview_data_url.clear();
        prepared.result.preview_data_url.clear();
    }
    prepared.result.processing_time_ms = started.elapsed().as_secs_f64() * 1_000.0;
    Ok(prepared.result)
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
            buffers.push((pixel_id.clone(), store.full_typed(&pixel_id)?));
        }
        buffers
    };

    tauri::async_runtime::spawn_blocking(move || {
        let _job = crate::resources::acquire_job(None)?;
        crate::layers::write_recovery_snapshot_typed(
            &document,
            &operations,
            &buffers,
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
        create_blank_layer_document,
        write_recovery_snapshot,
        list_recovery_snapshots,
        restore_recovery_snapshot,
        discard_recovery_snapshot,
        plan_layer_workflow,
        super::editor::analyze_image,
        super::editor::open_raw_image,
        super::raw::develop_raw_layer,
        super::professional::generate_histogram,
        super::professional::inspect_image_pixel,
        super::professional::create_point_operation,
        super::mask::magic_wand_selection,
        super::mask::color_range_selection,
        super::mask::refine_selection_mask,
        super::smart::convert_layers_to_smart_object,
        super::smart::import_smart_object,
        super::smart::update_smart_source,
        super::smart::inspect_smart_links,
        super::smart::relink_smart_source,
        super::text::inspect_document_fonts,
        super::text::rasterize_semantic_layer
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
                precision: Default::default(),
                canvas_width: 2,
                canvas_height: 2,
                layers: vec![test_pixel_layer("background", pixel_id, 2, 2)],
                active_layer_id: Some("background".into()),
                smart_sources: Default::default(),
            },
            document_operations: vec![],
            linear_pixels: vec![],
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
