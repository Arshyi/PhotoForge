//! The EXIF, XMP and IPTC of a TIFF picture, read by seeking.
//!
//! The picture's directories are found by [`super::structure`]; what is built from them is the
//! same block, under the same rules, as for a DNG (see `crate::metadata::tiff`), so one
//! photograph's metadata does not depend on which of the two it was saved as.
use super::structure::{kind_width, Directory, Order, TiffFile};
use crate::metadata::tiff::{assemble, keeps_exif, keeps_ifd0, Parts, RawField, MAX_FIELD_BYTES};
use crate::metadata::SourceMetadata;
use std::io::{Read, Seek};

const TAG_XMP: u16 = 700;
const TAG_IPTC_NAA: u16 = 33723;
const TAG_PHOTOSHOP: u16 = 34377;
const TAG_EXIF_IFD: u16 = 34665;
const TAG_GPS_IFD: u16 = 34853;
const TAG_INTEROP_IFD: u16 = 0xA005;
const TAG_MAKER_NOTE: u16 = 0x927C;
const TAG_DNG_PRIVATE_DATA: u16 = 0xC634;

/// The fields of `directory` that `wanted` names, with their values when they are small enough
/// to be carried and can be read.
fn fields<R: Read + Seek>(
    tiff: &mut TiffFile<R>,
    directory: &Directory,
    wanted: impl Fn(u16) -> bool,
) -> Vec<RawField> {
    let mut found = Vec::new();
    for entry in directory.entries.values() {
        if !wanted(entry.tag) {
            continue;
        }
        let declared = kind_width(entry.kind)
            .unwrap_or(1)
            .saturating_mul(u64::from(entry.count));
        let value = if declared <= MAX_FIELD_BYTES as u64 {
            tiff.value_bytes(entry).ok()
        } else {
            None
        };
        found.push(RawField {
            tag: entry.tag,
            kind: entry.kind,
            count: entry.count,
            value,
        });
    }
    found
}

/// The directory a pointer entry of `from` names, if it names one that can be read.
fn pointed<R: Read + Seek>(
    tiff: &mut TiffFile<R>,
    from: &Directory,
    tag: u16,
) -> Option<Directory> {
    let entry = from.get(tag)?;
    if entry.count != 1 || !matches!(entry.kind, 4 | 13) {
        return None;
    }
    let offset = tiff.first_u32(from, tag).ok()??;
    if offset == 0 {
        return None;
    }
    tiff.directory(offset).ok()
}

/// A value of one of the field types given, up to `cap` bytes.
fn value<R: Read + Seek>(
    tiff: &mut TiffFile<R>,
    directory: &Directory,
    tag: u16,
    kinds: &[u16],
    cap: usize,
) -> Option<Vec<u8>> {
    let entry = directory.get(tag)?;
    if !kinds.contains(&entry.kind) {
        return None;
    }
    let declared = kind_width(entry.kind)?.saturating_mul(u64::from(entry.count));
    if declared == 0 || declared > cap as u64 {
        return None;
    }
    tiff.value_bytes(entry).ok()
}

/// The metadata of the picture whose first directory is `ifd0`. The orientation is read, not
/// applied; whether the pixels were turned is for the caller to say.
pub fn read<R: Read + Seek>(tiff: &mut TiffFile<R>, ifd0: &Directory) -> Option<SourceMetadata> {
    let exif = pointed(tiff, ifd0, TAG_EXIF_IFD);
    let interop = exif
        .as_ref()
        .and_then(|directory| pointed(tiff, directory, TAG_INTEROP_IFD));
    let gps = pointed(tiff, ifd0, TAG_GPS_IFD);
    let maker_note = ifd0.has(TAG_DNG_PRIVATE_DATA)
        || exif
            .as_ref()
            .is_some_and(|directory| directory.has(TAG_MAKER_NOTE));
    let parts = Parts {
        big_endian: tiff.order() == Order::Big,
        ifd0: fields(tiff, ifd0, keeps_ifd0),
        exif: exif.as_ref().map(|d| fields(tiff, d, keeps_exif)),
        interop: interop.as_ref().map(|d| fields(tiff, d, |_| true)),
        gps: gps.as_ref().map(|d| fields(tiff, d, |_| true)),
        maker_note,
        xmp: value(
            tiff,
            ifd0,
            TAG_XMP,
            &[1, 2, 7],
            crate::metadata::xmp::MAX_XMP_BYTES,
        ),
        iptc_naa: value(
            tiff,
            ifd0,
            TAG_IPTC_NAA,
            &[1, 2, 4, 7],
            crate::metadata::iptc::MAX_IPTC_BYTES,
        ),
        photoshop: value(tiff, ifd0, TAG_PHOTOSHOP, &[1, 7], 1 << 20),
    };
    Some(assemble(parts))
}
