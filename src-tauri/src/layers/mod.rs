//! Layer tree, deterministic compositor, and project container.
//!
//! The layer document is pure data: identifiers, geometry, parameters, and mask
//! coverage. Pixel buffers live in the session store and are referenced by
//! identifier, so the tree stays cheap to clone, undo, and serialize.

mod blend;
pub mod cache;
mod composite;
mod disk_cache;
mod linear;
mod model;
mod project;
mod recovery;
mod selection;
mod store;
pub mod tiled;
pub mod tiles;
mod transform;
mod workflow;

pub use blend::{composite_pixel, BlendMode};
pub use cache::{CacheStats, DocumentFingerprint, TileCache, DEFAULT_CACHE_BYTES};
pub use composite::{
    render_document, render_layers, PixelSource, RenderOptions, MAX_RENDER_THREADS,
};
pub use disk_cache::{DiskCacheStats, DiskTileCache, MAX_DISK_TILE_BYTES};
pub use linear::{render_document_float, render_document_typed, render_document_typed_cached};
pub use model::{
    validate_dimensions, Layer, LayerContent, LayerDocument, LayerKind, LayerMask, LayerMetadata,
    LAYER_SCHEMA_VERSION, MAX_GROUP_DEPTH, MAX_LAYERS,
};
pub use project::{
    decode_project, encode_project, encode_project_typed, load_project, save_project,
    save_project_typed, LoadedProject, ProjectManifest, MAX_PROJECT_BYTES, PROJECT_EXTENSION,
    PROJECT_FORMAT_VERSION,
};
pub use recovery::{
    discard_all as discard_recovery_snapshots, discard_managed_snapshot,
    discard_snapshot as discard_recovery_snapshot, list_snapshots as list_recovery_snapshots,
    read_managed_snapshot, read_snapshot as read_recovery_snapshot,
    write_snapshot as write_recovery_snapshot,
    write_snapshot_typed as write_recovery_snapshot_typed, RecoveryRecord, MAX_RECOVERY_SNAPSHOTS,
    RECOVERY_EXTENSION,
};
pub use selection::{
    intersect_with_layer_bounds, layer_mask_to_selection, mask_space, selection_to_layer_mask,
};
pub use store::{preview_dimensions, LayerPixelStore, ResolvedPixels, PREVIEW_MAX_DIMENSION};
pub use tiled::{
    render_document_streaming, render_document_streaming_cached, render_document_tiled,
    render_document_tiled_cached, render_document_tiled_default,
    render_document_tiled_with_threads, render_region, TiledStats,
};
pub use tiles::{Region, TileGrid, DEFAULT_TILE_SIZE};
pub use transform::{LayerBounds, LayerInterpolation, LayerTransform};
pub use workflow::{
    check_kind as check_layer_step_kind, plan_against as plan_layer_steps,
    resolve as resolve_layer, validate_layer_steps, validate_planner_layer_steps, LayerSelector,
    LayerWorkflowStep, LAYER_WORKFLOW_SCHEMA_VERSION, MAX_LAYER_WORKFLOW_STEPS,
};

/// Builds a plain pixel layer. Test-only helper shared across modules.
#[cfg(test)]
pub fn test_pixel_layer(id: &str, pixel_id: &str, width: u32, height: u32) -> Layer {
    Layer {
        id: id.into(),
        name: id.into(),
        visible: true,
        locked: false,
        opacity: 1.0,
        blend_mode: BlendMode::Normal,
        transform: LayerTransform::default(),
        mask: None,
        collapsed: false,
        metadata: LayerMetadata::default(),
        raw: None,
        content: LayerContent::Pixel {
            pixel_id: pixel_id.into(),
            width,
            height,
        },
    }
}
