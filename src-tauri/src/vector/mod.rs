//! Vector geometry and its rasterisation.
//!
//! Shapes are stored as shapes. A rectangle keeps its width, height and corner
//! radius; a star keeps its point count. They become coordinates only when
//! something needs to draw them, which is what lets the user go on editing the
//! shape rather than its remains.
//!
//! Coverage is floating point throughout. See `raster` for why that ruled out
//! the obvious dependency.
pub mod path;
pub mod raster;
pub mod stroke;

pub use path::{PathCommand, ShapeGeometry, VectorPath, MAX_PATH_COMMANDS, MAX_POLYGON_SIDES};
pub use raster::{rasterize, rasterize_path, CoverageMask, FillRule};
pub use stroke::{stroke_outlines, LineCap, LineJoin, StrokeStyle, MAX_STROKE_WIDTH};
