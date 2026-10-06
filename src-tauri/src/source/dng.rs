//! DNG: what the planner is told about a camera RAW, and a bounded preview of one.
//!
//! A DNG is the one format here whose decoder can read part of the picture without
//! reading the rest. Its pixel payload is stored as strips or tiles, each
//! compressed on its own, so a window of the sensor is produced by decoding only
//! the segments that overlap it. The raw pipeline does that (`raw::dng`), and
//! develops the window with enough surrounding photosites that the result equals
//! the same rectangle cut from a development of the whole sensor.
//!
//! Three limits are stated here rather than left to be discovered:
//!
//! * The **compressed file is read whole**. A region bounds the decoded sensor and
//!   everything developed from it, not the read of the file, which is at most
//!   [`crate::raw::RAW_MAX_FILE_BYTES`]. The planner counts it as a fixed cost.
//! * A file stored as **one strip** has one segment, the whole sensor, so its
//!   window decode holds the whole decoded sensor at two or four bytes a photosite
//!   while it works. The planner prices exactly that, from the layout.
//! * **Auto white balance** is a statistic over the whole sensor. A region asks
//!   for it by streaming the sensor one segment at a time; the work is whole-sensor
//!   even though the memory is not.
//!
//! No reduced copy is offered for a DNG. A bounded *preview* is, because choosing a
//! region needs one; it is built by decimating as segments are read.
use crate::color::{DevelopmentParameters, FloatImage};
use crate::error::AppError;
use crate::raw::{self, develop, dng, RawError};
use crate::resources::admission::{DecodeCapabilities, ReducedDecode, RegionDecode};
use std::path::Path;
use std::sync::atomic::AtomicBool;

/// What a header says about a DNG, without decoding a sample.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DngInfo {
    pub width: u32,
    pub height: u32,
    pub bits: u8,
    pub tiled: bool,
    /// Photosites in one strip or tile: the most decoded at once.
    pub segment_pixels: u64,
    pub segments: usize,
}

pub fn map_raw(error: RawError) -> AppError {
    match error {
        RawError::Cancelled => AppError::RenderCancelled,
        RawError::FileTooLarge => AppError::OutOfMemoryRisk,
        other => AppError::RawInspection(other.to_string()),
    }
}

/// The path in the form the RAW path guard accepts.
///
/// The probe canonicalises, and on Windows that yields `\?\C:\...`, whose leading
/// backslashes are indistinguishable from a network share to a guard that refuses
/// those on purpose. A verbatim UNC path keeps its prefix and is still refused.
fn guarded(path: &Path) -> std::path::PathBuf {
    std::path::PathBuf::from(raw::presentable_path(path))
}

/// Reads the layout of the DNG at `path`.
///
/// This reads the file, which is the cost of the format: its directory is found by
/// offsets, and the planner counts the file as resident whenever a DNG is opened.
/// The bytes are dropped before this returns.
pub fn read_info(path: &Path) -> Result<DngInfo, AppError> {
    let bytes = raw::read_source_bytes(&guarded(path)).map_err(map_raw)?;
    let layout = dng::inspect_layout(&bytes).map_err(map_raw)?;
    Ok(DngInfo {
        width: layout.width,
        height: layout.height,
        bits: layout.bits,
        tiled: layout.tiled,
        segment_pixels: layout.segment_pixels(),
        segments: layout.segment_count(),
    })
}

/// The most photosites a window can hold beyond its region, for any region on a
/// sensor of this size.
///
/// A window is its region grown by the demosaic margin (two photosites) on every
/// side, and moved out by one more to start on an even photosite. Each axis can
/// therefore grow by at most five, and the extra area is bounded by the strips
/// along the two axes plus the corner.
pub fn window_overhead_photosites(width: u32, height: u32) -> u64 {
    let grow = u64::from(develop::demosaic_margin(
        crate::raw::demosaic::Quality::High,
    )) * 2
        + 1;
    grow * (u64::from(width) + u64::from(height)) + grow * grow
}

/// What the planner is told this decoder can do.
pub fn capabilities(info: &DngInfo) -> DecodeCapabilities {
    let segment = info
        .segment_pixels
        .saturating_mul(dng::SEGMENT_TRANSIENT_BYTES_PER_PHOTOSITE);
    let margin = window_overhead_photosites(info.width, info.height)
        .saturating_mul(develop::DEVELOP_BYTES_PER_PHOTOSITE);
    DecodeCapabilities {
        region: RegionDecode::Segments {
            fixed_bytes: segment.saturating_add(margin),
            bytes_per_pixel: develop::DEVELOP_BYTES_PER_PHOTOSITE,
        },
        reduced: ReducedDecode::None,
    }
}

/// A picture of the whole sensor no larger than `max_edge` on a side, developed
/// with the default parameters, held one segment at a time.
pub fn preview(
    path: &Path,
    max_edge: u32,
    cancel: Option<&AtomicBool>,
) -> Result<FloatImage, AppError> {
    let bytes = raw::read_source_bytes(&guarded(path)).map_err(map_raw)?;
    let developed =
        develop::develop_preview(&bytes, &DevelopmentParameters::default(), max_edge, cancel)
            .map_err(map_raw)?;
    Ok(developed.image)
}
