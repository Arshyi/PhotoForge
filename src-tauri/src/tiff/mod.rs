//! TIFF: a picture read from, and written as, a TIFF file.
//!
//! The reader takes the part of the format that photographs and scans are written in (strips and
//! tiles, 1 to 64 bits a sample, none, PackBits, LZW and Deflate, both predictors, grey, RGB and
//! palette pictures with alpha, an ICC profile, EXIF, XMP and IPTC) and refuses the rest by
//! name. The writer produces one RGB or RGBA picture at 8 or 16 bits. Neither depends on a
//! TIFF library: the format is read here, bounded, as every other stranger's file is, and the
//! rules for both directions are in `docs/tiff.md`.
pub mod codec;
pub mod export;
pub mod import;
pub mod metadata;
pub mod structure;
#[cfg(test)]
pub(crate) mod testing;

pub use export::{account, write, Compression, Spec, Written};
pub use import::{import_from, import_tiff, ImportedTiff, TiffImportReport, MAX_TIFF_BYTES};
