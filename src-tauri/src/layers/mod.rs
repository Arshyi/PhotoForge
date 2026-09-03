//! Layer tree, deterministic compositor, and project container.
//!
//! The layer document is pure data: identifiers, geometry, parameters, and mask
//! coverage. Pixel buffers live in the session store and are referenced by
//! identifier, so the tree stays cheap to clone, undo, and serialize.

mod blend;
mod composite;
mod model;
mod project;
mod selection;
mod store;
mod transform;

pub use blend::{composite_pixel, BlendMode};
pub use composite::{render_document, render_layers, PixelSource, RenderOptions};
pub use model::{
    validate_dimensions, Layer, LayerContent, LayerDocument, LayerKind, LayerMask, LayerMetadata,
    LAYER_SCHEMA_VERSION, MAX_GROUP_DEPTH, MAX_LAYERS,
};
pub use project::{
    decode_project, encode_project, load_project, save_project, LoadedProject, ProjectManifest,
    MAX_PROJECT_BYTES, PROJECT_EXTENSION, PROJECT_FORMAT_VERSION,
};
pub use selection::{
    intersect_with_layer_bounds, layer_mask_to_selection, mask_space, selection_to_layer_mask,
};
pub use store::{preview_dimensions, LayerPixelStore, ResolvedPixels, PREVIEW_MAX_DIMENSION};
pub use transform::{LayerBounds, LayerTransform};
