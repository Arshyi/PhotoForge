mod bitmap;
mod color_range;
mod diagnostics;
mod feather;
mod flood_fill;
mod geometry;
mod operations;
mod persistence;
mod progress;
mod rasterize;
mod refine;
#[cfg(test)]
mod stress;
mod transform;

pub(crate) use bitmap::checked_length as checked_mask_length;
pub use bitmap::MaskBitmap;
pub(crate) use color_range::select_with_progress as select_color_range_with_progress;
pub use color_range::{select as select_color_range, ColorRangeOptions};
pub use diagnostics::{inspect as mask_diagnostics, MaskDiagnostics};
pub use feather::feather;
pub(crate) use flood_fill::select_with_progress as select_magic_wand_with_progress;
pub use flood_fill::{select as select_magic_wand, Connectivity, WandOptions};
pub use geometry::{Point, SelectionShape};
pub(crate) use operations::apply_with_progress as apply_mask_operation_with_progress;
pub(crate) use operations::compose_with_progress;
pub(crate) use operations::work_units as mask_operation_work_units;
pub use operations::{apply as apply_mask_operation, compose, CompositionMode, MaskOperation};
pub use persistence::{
    export_png, import_png, load_mask, save_mask, MaskFile, MaskMetadata, MaskSnapshot,
    MASK_FORMAT_VERSION,
};
pub use progress::{
    request_cancel as request_mask_progress_cancel, snapshot as mask_progress_snapshot,
    MaskProgress, MaskProgressCallback, MaskProgressHandle, MaskProgressState, MaskWorkContext,
    PlannedMaskProgress, SharedMaskProgress,
};
pub use rasterize::rasterize;
pub use refine::align_to_image_edges;
pub(crate) use refine::align_to_image_edges_with_progress;
pub(crate) use transform::remap_between_chains_with_progress;
pub use transform::{remap_between_chains, GeometryChain, GeometryStep, MAX_GEOMETRY_STEPS};
