//! Layer tree, deterministic compositor, and project container.
//!
//! The layer document is pure data: identifiers, geometry, parameters, and mask
//! coverage. Pixel buffers live in the session store and are referenced by
//! identifier, so the tree stays cheap to clone, undo, and serialize.

mod blend;
mod composite;
mod model;
mod project;
mod recovery;
mod selection;
mod store;
mod transform;
mod workflow;

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
pub use recovery::{
    discard_all as discard_recovery_snapshots, discard_managed_snapshot,
    discard_snapshot as discard_recovery_snapshot, list_snapshots as list_recovery_snapshots,
    read_managed_snapshot, read_snapshot as read_recovery_snapshot,
    write_snapshot as write_recovery_snapshot, RecoveryRecord, MAX_RECOVERY_SNAPSHOTS,
    RECOVERY_EXTENSION,
};
pub use selection::{
    intersect_with_layer_bounds, layer_mask_to_selection, mask_space, selection_to_layer_mask,
};
pub use store::{preview_dimensions, LayerPixelStore, ResolvedPixels, PREVIEW_MAX_DIMENSION};
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
        content: LayerContent::Pixel {
            pixel_id: pixel_id.into(),
            width,
            height,
        },
    }
}
