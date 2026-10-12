//! Writing a TIFF.
//!
//! One picture, RGB or RGBA, 8 or 16 bits a sample, little-endian, in strips of about a quarter
//! of a megabyte, optionally Deflate-compressed with horizontal differencing (the combination
//! Photoshop and libtiff write and every reader of the format reads). An ICC profile is embedded
//! when the caller has one. The camera's EXIF, XMP and IPTC go where TIFF puts them: the
//! descriptive tags in the first directory, the exposure and the place in their own directories,
//! the packet and the stream as tags.
//!
//! The strips are written as they are made. The directory comes after them, because its offsets
//! are of what has been written, and the header is told where it is last.
use crate::error::AppError;
use crate::metadata::tiff::RawField;
use std::io::{Seek, SeekFrom, Write};

/// About how much picture one strip holds before compression.
const STRIP_TARGET_BYTES: u64 = 256 * 1024;

const TAG_WIDTH: u16 = 256;
const TAG_LENGTH: u16 = 257;
const TAG_BITS: u16 = 258;
const TAG_COMPRESSION: u16 = 259;
const TAG_PHOTOMETRIC: u16 = 262;
const TAG_STRIP_OFFSETS: u16 = 273;
const TAG_SAMPLES: u16 = 277;
const TAG_ROWS_PER_STRIP: u16 = 278;
const TAG_STRIP_COUNTS: u16 = 279;
const TAG_PLANAR: u16 = 284;
const TAG_PREDICTOR: u16 = 317;
const TAG_EXTRA_SAMPLES: u16 = 338;
const TAG_XMP: u16 = 700;
const TAG_IPTC: u16 = 33723;
const TAG_ICC: u16 = 34675;
const TAG_EXIF_IFD: u16 = 34665;
const TAG_GPS_IFD: u16 = 34853;
const TAG_INTEROP_IFD: u16 = 0xA005;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Compression {
    None,
    #[default]
    Deflate,
}

/// What is written.
pub struct Spec<'a> {
    pub width: u32,
    pub height: u32,
    /// 8 or 16.
    pub bits: u16,
    /// Four samples a pixel, the fourth straight (unassociated) alpha; otherwise three.
    pub alpha: bool,
    pub compression: Compression,
    pub icc: Option<&'a [u8]>,
    pub metadata: Option<&'a crate::metadata::Prepared>,
}

/// What came of it.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Written {
    pub bytes: u64,
    pub placed: crate::metadata::container::Placed,
    /// The EXIF carried a maker note, which cannot be moved out of its block.
    pub maker_note_dropped: bool,
}

#[derive(Debug, Clone)]
struct Item {
    tag: u16,
    kind: u16,
    count: u32,
    value: Vec<u8>,
    /// For a pointer: the directory it names.
    link: Option<usize>,
    offset: u32,
}

impl Item {
    fn new(tag: u16, kind: u16, count: u32, value: Vec<u8>) -> Self {
        Self {
            tag,
            kind,
            count,
            value,
            link: None,
            offset: 0,
        }
    }

    fn short(tag: u16, value: u16) -> Self {
        Self::new(tag, 3, 1, value.to_le_bytes().to_vec())
    }

    fn long(tag: u16, value: u32) -> Self {
        Self::new(tag, 4, 1, value.to_le_bytes().to_vec())
    }

    fn shorts(tag: u16, values: &[u16]) -> Self {
        Self::new(
            tag,
            3,
            values.len() as u32,
            values.iter().flat_map(|v| v.to_le_bytes()).collect(),
        )
    }

    fn longs(tag: u16, values: &[u32]) -> Self {
        Self::new(
            tag,
            4,
            values.len() as u32,
            values.iter().flat_map(|v| v.to_le_bytes()).collect(),
        )
    }

    fn pointer(tag: u16, target: usize) -> Self {
        let mut item = Self::new(tag, 4, 1, vec![0; 4]);
        item.link = Some(target);
        item
    }

    fn from_field(field: &RawField) -> Option<Self> {
        let value = field.value.clone()?;
        Some(Self::new(field.tag, field.kind, field.count, value))
    }
}

fn io(_: std::io::Error) -> AppError {
    AppError::ExportFailure
}

/// Horizontal differencing of one row, the inverse of what a reader undoes.
fn difference_row(row: &mut [u8], samples: usize, bits: u16) {
    if bits == 8 {
        for index in (samples..row.len()).rev() {
            row[index] = row[index].wrapping_sub(row[index - samples]);
        }
    } else {
        let words = row.len() / 2;
        for index in (samples..words).rev() {
            let current = u16::from_le_bytes([row[index * 2], row[index * 2 + 1]]);
            let before = (index - samples) * 2;
            let earlier = u16::from_le_bytes([row[before], row[before + 1]]);
            let bytes = current.wrapping_sub(earlier).to_le_bytes();
            row[index * 2] = bytes[0];
            row[index * 2 + 1] = bytes[1];
        }
    }
}

/// Writes the directories one after another, preceded by every value that did not fit in its
/// entry, from `start`. Returns the offset of the first directory.
fn write_directories<W: Write + Seek>(
    out: &mut W,
    start: u64,
    directories: &mut [Vec<Item>],
) -> Result<u64, AppError> {
    for directory in directories.iter_mut() {
        directory.sort_by_key(|item| item.tag);
    }
    // Values first, on even offsets.
    let mut cursor = start + (start & 1);
    for directory in directories.iter_mut() {
        for item in directory.iter_mut() {
            if item.link.is_none() && item.value.len() > 4 {
                item.offset = u32::try_from(cursor).map_err(|_| AppError::ExportFailure)?;
                cursor += item.value.len() as u64;
                cursor += cursor & 1;
            }
        }
    }
    let values_end = cursor;
    let mut at = Vec::with_capacity(directories.len());
    let mut next = values_end;
    for directory in directories.iter() {
        at.push(next);
        next += 2 + 12 * directory.len() as u64 + 4;
    }
    if next > u64::from(u32::MAX) {
        return Err(AppError::ExportFailure);
    }
    if start & 1 == 1 {
        out.write_all(&[0]).map_err(io)?;
    }
    let mut written = start + (start & 1);
    for directory in directories.iter() {
        for item in directory {
            if item.link.is_none() && item.value.len() > 4 {
                out.write_all(&item.value).map_err(io)?;
                written += item.value.len() as u64;
                if written & 1 == 1 {
                    out.write_all(&[0]).map_err(io)?;
                    written += 1;
                }
            }
        }
    }
    debug_assert_eq!(written, values_end);
    for directory in directories.iter() {
        out.write_all(&(directory.len() as u16).to_le_bytes())
            .map_err(io)?;
        for item in directory {
            out.write_all(&item.tag.to_le_bytes()).map_err(io)?;
            out.write_all(&item.kind.to_le_bytes()).map_err(io)?;
            out.write_all(&item.count.to_le_bytes()).map_err(io)?;
            if let Some(target) = item.link {
                let offset = u32::try_from(*at.get(target).ok_or(AppError::ExportFailure)?)
                    .map_err(|_| AppError::ExportFailure)?;
                out.write_all(&offset.to_le_bytes()).map_err(io)?;
            } else if item.value.len() <= 4 {
                let mut inline = [0_u8; 4];
                inline[..item.value.len()].copy_from_slice(&item.value);
                out.write_all(&inline).map_err(io)?;
            } else {
                out.write_all(&item.offset.to_le_bytes()).map_err(io)?;
            }
        }
        out.write_all(&0_u32.to_le_bytes()).map_err(io)?;
    }
    Ok(at[0])
}

/// Completes the account of an export with what writing the metadata did and could not do.
pub fn account(plan: &mut crate::metadata::Plan, written: &Written) {
    if written.maker_note_dropped {
        plan.report.removed(
            "the camera maker's private notes (their offsets are into the block they came from, so a TIFF cannot hold them)",
        );
    }
    crate::metadata::report_unplaced(&mut plan.report, &plan.prepared, written.placed);
}

/// Writes the picture `rows` produces, and the metadata of `spec`, to `out`.
///
/// `rows(y, row)` fills `row` with the samples of row `y`: `width × (3 or 4)` values of 8 bits,
/// or of 16 bits as little-endian pairs.
pub fn write<W: Write + Seek>(
    out: &mut W,
    spec: &Spec<'_>,
    mut rows: impl FnMut(u32, &mut Vec<u8>) -> Result<(), AppError>,
) -> Result<Written, AppError> {
    if spec.width == 0 || spec.height == 0 || !matches!(spec.bits, 8 | 16) {
        return Err(AppError::ExportFailure);
    }
    let samples = if spec.alpha { 4_usize } else { 3 };
    let bytes_per_sample = usize::from(spec.bits / 8);
    let row_bytes = spec.width as usize * samples * bytes_per_sample;
    let rows_per_strip =
        (STRIP_TARGET_BYTES / row_bytes as u64).clamp(1, u64::from(spec.height)) as u32;
    let deflate = spec.compression == Compression::Deflate;

    out.write_all(&[b'I', b'I', 42, 0, 0, 0, 0, 0])
        .map_err(io)?;
    let mut offsets: Vec<u32> = Vec::new();
    let mut counts: Vec<u32> = Vec::new();
    let mut position = 8_u64;
    let mut row = Vec::with_capacity(row_bytes);
    let mut strip = Vec::with_capacity(row_bytes * rows_per_strip as usize);
    for y in 0..spec.height {
        row.clear();
        rows(y, &mut row)?;
        if row.len() != row_bytes {
            return Err(AppError::ExportFailure);
        }
        if deflate {
            difference_row(&mut row, samples, spec.bits);
        }
        strip.extend_from_slice(&row);
        if (y + 1) % rows_per_strip == 0 || y + 1 == spec.height {
            let encoded;
            let payload: &[u8] = if deflate {
                encoded = miniz_oxide::deflate::compress_to_vec_zlib(&strip, 6);
                &encoded
            } else {
                &strip
            };
            offsets.push(u32::try_from(position).map_err(|_| AppError::ExportFailure)?);
            counts.push(u32::try_from(payload.len()).map_err(|_| AppError::ExportFailure)?);
            out.write_all(payload).map_err(io)?;
            position += payload.len() as u64;
            strip.clear();
        }
    }

    // The directories.
    let mut ifd0 = vec![
        Item::long(TAG_WIDTH, spec.width),
        Item::long(TAG_LENGTH, spec.height),
        Item::shorts(TAG_BITS, &vec![spec.bits; samples]),
        Item::short(TAG_COMPRESSION, if deflate { 8 } else { 1 }),
        Item::short(TAG_PHOTOMETRIC, 2),
        Item::longs(TAG_STRIP_OFFSETS, &offsets),
        Item::short(TAG_SAMPLES, samples as u16),
        Item::long(TAG_ROWS_PER_STRIP, rows_per_strip),
        Item::longs(TAG_STRIP_COUNTS, &counts),
        Item::short(TAG_PLANAR, 1),
    ];
    if deflate {
        ifd0.push(Item::short(TAG_PREDICTOR, 2));
    }
    if spec.alpha {
        // Unassociated alpha: the colour is not multiplied by it.
        ifd0.push(Item::short(TAG_EXTRA_SAMPLES, 2));
    }
    if let Some(profile) = spec.icc {
        ifd0.push(Item::new(
            TAG_ICC,
            7,
            profile.len() as u32,
            profile.to_vec(),
        ));
    }

    let mut result = Written::default();
    let mut directories: Vec<Vec<Item>> = Vec::new();
    let mut exif_items: Vec<Item> = Vec::new();
    let mut gps_items: Vec<Item> = Vec::new();
    let mut interop_items: Vec<Item> = Vec::new();
    if let Some(prepared) = spec.metadata {
        if let Some(block) = &prepared.exif {
            if let Some(split) = crate::metadata::tiff::split(block) {
                result.maker_note_dropped = split.maker_note;
                for field in &split.ifd0 {
                    if ifd0.iter().any(|item| item.tag == field.tag) {
                        continue;
                    }
                    if let Some(item) = Item::from_field(field) {
                        ifd0.push(item);
                    }
                }
                exif_items = split.exif.iter().filter_map(Item::from_field).collect();
                interop_items = split.interop.iter().filter_map(Item::from_field).collect();
                gps_items = split.gps.iter().filter_map(Item::from_field).collect();
                result.placed.exif = true;
            }
        }
        if let Some(xmp) = &prepared.xmp {
            ifd0.push(Item::new(TAG_XMP, 1, xmp.len() as u32, xmp.clone()));
            result.placed.xmp = true;
        }
        if let Some(iptc) = &prepared.iptc {
            ifd0.push(Item::new(TAG_IPTC, 7, iptc.len() as u32, iptc.clone()));
            result.placed.iptc = true;
        }
    }
    // Directory 0 is the picture's; the others are numbered as they are added.
    directories.push(Vec::new());
    let mut next = 1;
    let mut exif_index = None;
    if !exif_items.is_empty() || !interop_items.is_empty() {
        exif_index = Some(next);
        next += 1;
        if !interop_items.is_empty() {
            exif_items.push(Item::pointer(TAG_INTEROP_IFD, next));
        }
        ifd0.push(Item::pointer(TAG_EXIF_IFD, exif_index.unwrap_or(0)));
    }
    let mut interop_index = None;
    if !interop_items.is_empty() {
        interop_index = Some(next);
        next += 1;
    }
    let mut gps_index = None;
    if !gps_items.is_empty() {
        gps_index = Some(next);
        ifd0.push(Item::pointer(TAG_GPS_IFD, next));
    }
    directories[0] = ifd0;
    if exif_index.is_some() {
        directories.push(exif_items);
    }
    if interop_index.is_some() {
        directories.push(interop_items);
    }
    if gps_index.is_some() {
        directories.push(gps_items);
    }

    let first = write_directories(out, position, &mut directories)?;
    out.seek(SeekFrom::Start(4)).map_err(io)?;
    out.write_all(
        &u32::try_from(first)
            .map_err(|_| AppError::ExportFailure)?
            .to_le_bytes(),
    )
    .map_err(io)?;
    let end = out.seek(SeekFrom::End(0)).map_err(io)?;
    result.bytes = end;
    Ok(result)
}

#[cfg(test)]
mod tests;
