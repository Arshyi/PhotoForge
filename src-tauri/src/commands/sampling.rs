//! Immutable sources for tools that sample the displayed layer composite.
use crate::application::AppState;
use crate::error::AppError;
use crate::layers::{render_layers, LayerDocument, RenderOptions, ResolvedPixels};
use image::{DynamicImage, GenericImageView};
use std::sync::atomic::Ordering;
use std::sync::Arc;

enum Pixels {
    Opened(Arc<DynamicImage>),
    Layered {
        document: LayerDocument,
        pixels: ResolvedPixels,
        scale: f64,
    },
}

pub(super) struct SamplingSource {
    pub full_dimensions: (u32, u32),
    pixels: Pixels,
}

impl SamplingSource {
    /// Only call in a blocking worker; resolving the handles is cheap, rendering
    /// them is not. The original session image is never replaced or modified.
    pub(super) fn render(self) -> Result<Arc<DynamicImage>, AppError> {
        match self.pixels {
            Pixels::Opened(source) => Ok(source),
            Pixels::Layered {
                document,
                pixels,
                scale,
            } => Ok(Arc::new(DynamicImage::ImageRgba8(render_layers(
                &document.layers,
                document.canvas_width,
                document.canvas_height,
                &pixels,
                RenderOptions {
                    scale,
                    cancel: None,
                },
            )?))),
        }
    }
}

pub(super) fn is_current(state: &AppState, document_id: u64) -> Result<bool, AppError> {
    let session = state
        .session
        .lock()
        .map_err(|_| AppError::ProcessingFailure("editor state is unavailable".into()))?;
    Ok(state.pending_open_request.load(Ordering::Acquire) == 0
        && session
            .as_ref()
            .is_some_and(|session| session.document_id == document_id))
}

/// Check identity and capture layer handles under the same session/store lock
/// order used by opens. No worker can accidentally sample a newly opened file
/// using a previous document's tree (whose pixel IDs may have been reused).
pub(super) fn capture(
    state: &AppState,
    document_id: u64,
    layer_document: Option<LayerDocument>,
    preview: bool,
) -> Result<SamplingSource, AppError> {
    if let Some(document) = &layer_document {
        document.validate()?;
    }
    let session = state
        .session
        .lock()
        .map_err(|_| AppError::ProcessingFailure("editor state is unavailable".into()))?;
    let session = session.as_ref().ok_or(AppError::NoImageOpen)?;
    if session.document_id != document_id || state.pending_open_request.load(Ordering::Acquire) != 0
    {
        return Err(AppError::NoImageOpen);
    }
    if let Some(document) = layer_document {
        let store = state
            .layers
            .lock()
            .map_err(|_| AppError::ProcessingFailure("layer store is unavailable".into()))?;
        if store.canvas() != (document.canvas_width, document.canvas_height) {
            return Err(AppError::InvalidLayerDocument(
                "sampling canvas differs from the open document".into(),
            ));
        }
        let pixels = store.resolve(&document.referenced_pixel_ids(), preview)?;
        Ok(SamplingSource {
            full_dimensions: (document.canvas_width, document.canvas_height),
            pixels: Pixels::Layered {
                document,
                pixels,
                scale: if preview { store.preview_scale() } else { 1.0 },
            },
        })
    } else {
        Ok(SamplingSource {
            full_dimensions: session.source.original.dimensions(),
            pixels: Pixels::Opened(if preview {
                session.source.preview.clone()
            } else {
                session.source.original.clone()
            }),
        })
    }
}
