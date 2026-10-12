//! OpenEXR: a document read from, and written as, scene-linear floating-point pictures.
//!
//! An EXR is the format of renders and compositing: linear light, half or single precision,
//! values above one, premultiplied alpha. PhotoForge's high-precision document is the same kind of
//! thing, so this is a conversion of convention, not of data. The rules for both directions are in
//! `docs/openexr.md`.
pub mod colour;
pub mod export;
pub mod import;
#[cfg(test)]
pub(crate) mod testing;

pub use export::{export_exr, ExrExport, ExrExportReport, ExrOptions};
pub use import::{import_exr, ExrImportReport, ImportedExr, MAX_EXR_BYTES};
