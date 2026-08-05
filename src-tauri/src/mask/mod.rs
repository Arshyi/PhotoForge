mod bitmap;
mod color_range;
mod diagnostics;
mod feather;
mod flood_fill;
mod geometry;
mod operations;
mod persistence;
mod rasterize;
mod refine;

pub use bitmap::MaskBitmap;
pub use color_range::{select as select_color_range, ColorRangeOptions};
pub use diagnostics::{inspect as mask_diagnostics, MaskDiagnostics};
pub use flood_fill::{select as select_magic_wand, Connectivity, WandOptions};
pub use geometry::{Point, SelectionShape};
pub use operations::{apply as apply_mask_operation, compose, CompositionMode, MaskOperation};
pub use persistence::{
    export_png, import_png, load_mask, save_mask, MaskFile, MaskMetadata, MaskSnapshot,
    MASK_FORMAT_VERSION,
};
pub use rasterize::rasterize;
pub use refine::align_to_image_edges;
