//! Reading Photoshop documents (PSD and PSB).
//!
//! This is the structural reader: it finds the header, the resources, every layer record and
//! where each channel's pixels are, and decodes a channel when asked. It builds no
//! PhotoForge document and holds no pixels; mapping what it finds onto layers, and saying
//! what could not be carried over, is the import's job.
//!
//! # Why in-house
//!
//! A Photoshop file is a hostile-input surface with a large, irregular format. The
//! crates that read it were weighed in the Phase 15 brief: none had a verified account of
//! 16/32-bit layers, PSB or blend modes, and none was written to be bounded. This reader
//! is small, takes the file's own declared sizes as claims to be checked rather than
//! allocations to be made, and is exercised by truncation and mutation sweeps (`tests`).
//!
//! # What it does not do
//!
//! It does not read CMYK, Lab, indexed, bitmap, duotone or multichannel *pixels* (the
//! header is read and the mode reported, so the import can refuse them with a reason). It
//! does not read text, vector, smart-object or effect data: it records that a layer
//! carries them.
pub mod adjustment;
pub mod blend;
pub mod document;
pub mod error;
pub mod import;
pub mod pixels;
pub mod reader;
pub mod tree;

pub use document::{
    read_document, ChannelInfo, ColorMode, Divider, Document, Header, LayerRecord, LayerSection,
    MaskData, RealMask, Rect, Resources, Version,
};
pub use error::PsdError;
pub use import::{import_psd, ImportLimits, ImportReport, Imported, ReportEntry, ReportKind};
pub use pixels::{decode_channel, decode_merged_image, ChannelSpec, Compression, MergedSpec};
pub use reader::Reader;
pub use tree::{build_tree, Node};

#[cfg(test)]
mod testing;
#[cfg(test)]
mod tests;
