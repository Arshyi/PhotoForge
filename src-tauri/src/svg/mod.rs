//! SVG: the shape layers of a document written as SVG, and a bounded subset of SVG read as shape
//! layers.
//!
//! SVG is text from a stranger, and an SVG reader that is not careful is a network client, a
//! script host and an entity-expansion bomb at once. The rules for both directions are in
//! `docs/svg.md`; the short form is that nothing here fetches, runs or follows anything.
pub mod export;
pub mod import;

pub use export::{export_svg, SvgExport, SvgOptions, SvgReport};
pub use import::{import_svg, import_svg_text, ImportedSvg, SvgImportReport, MAX_SVG_BYTES};
