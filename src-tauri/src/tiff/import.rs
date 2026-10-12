//! Reading a TIFF as a document.
//!
//! A TIFF is a container of one or more pictures, each stored as strips or tiles of rows, each
//! row of samples of 1 to 64 bits, compressed one of several ways and optionally differenced.
//! This reader takes the part of that which photographs and scans are written in, by name, and
//! refuses the rest by name: what it cannot read is never read wrongly.
//!
//! The file is a stranger's, so it is admitted from its directory before any pixel is decoded:
//! the size of the picture, of the buffer it becomes, and of the largest strip or tile that
//! has to be held while it is made, are checked against the resource budget; every offset and
//! length is checked against the file; and every decoder is told how many bytes it may produce.
//!
//! Colour follows the Photoshop importer's rule, so one file gives one result by either door:
//! 8-bit pictures with no profile, or an sRGB one, stay 8-bit; 16-bit, floating-point, and
//! pictures with another RGB profile become high-precision documents converted through the
//! profile; 16-bit with no profile is read as sRGB; floating point with none as linear light.
//! The orientation tag is applied to the pixels. Everything the picture carries that PhotoForge
//! has no place for is left out **and named**.
use super::codec;
use super::structure::{Directory, Order, TiffFile, MAX_SEGMENTS};
use crate::color::{srgb_decode, FloatImage, FloatRgba};
use crate::color_management::IccRowTransform;
use crate::error::AppError;
use crate::layers::{validate_dimensions, LayerContent, LayerDocument, LoadedProject};
use crate::operations::structure::new_layer;
use crate::operations::support::{Clock, SystemClock};
use crate::pixel::DocumentPrecision;
use crate::psd::{ImportLimits, ReportEntry, ReportKind};
use serde::Serialize;
use std::io::{Read, Seek};
use std::path::Path;

/// The largest file read. Classic TIFF cannot address more than this.
pub const MAX_TIFF_BYTES: u64 = 4 << 30;

const TAG_NEW_SUBFILE_TYPE: u16 = 254;
const TAG_WIDTH: u16 = 256;
const TAG_LENGTH: u16 = 257;
const TAG_BITS: u16 = 258;
const TAG_COMPRESSION: u16 = 259;
const TAG_PHOTOMETRIC: u16 = 262;
const TAG_FILL_ORDER: u16 = 266;
const TAG_STRIP_OFFSETS: u16 = 273;
const TAG_ORIENTATION: u16 = 274;
const TAG_SAMPLES: u16 = 277;
const TAG_ROWS_PER_STRIP: u16 = 278;
const TAG_STRIP_COUNTS: u16 = 279;
const TAG_PLANAR: u16 = 284;
const TAG_PREDICTOR: u16 = 317;
const TAG_COLOUR_MAP: u16 = 320;
const TAG_TILE_WIDTH: u16 = 322;
const TAG_TILE_LENGTH: u16 = 323;
const TAG_TILE_OFFSETS: u16 = 324;
const TAG_TILE_COUNTS: u16 = 325;
const TAG_EXTRA_SAMPLES: u16 = 338;
const TAG_SAMPLE_FORMAT: u16 = 339;
const TAG_PHOTOSHOP_LAYERS: u16 = 37724;
const TAG_ICC: u16 = 34675;
const TAG_SUB_IFDS: u16 = 330;

fn refuse(message: impl Into<String>) -> AppError {
    AppError::foreign_file(message.into())
}

/// What an import did, in the shape the Photoshop import report has so one dialog shows both.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TiffImportReport {
    /// Always "TIFF".
    pub format: String,
    pub width: u32,
    pub height: u32,
    /// The depth of the file's samples: 1 to 64.
    pub bit_depth: u16,
    /// "RGB" or "Grayscale" (a palette is read as RGB).
    pub color_mode: String,
    pub layers_imported: usize,
    pub groups_imported: usize,
    pub entries: Vec<ReportEntry>,
}

impl TiffImportReport {
    fn push(&mut self, kind: ReportKind, detail: impl Into<String>) {
        self.entries.push(ReportEntry {
            kind,
            layer: None,
            detail: detail.into(),
        });
    }

    pub fn count(&self, kind: ReportKind) -> usize {
        self.entries.iter().filter(|e| e.kind == kind).count()
    }
}

/// An imported document, as the project loader would hand it over.
pub struct ImportedTiff {
    pub project: LoadedProject,
    pub report: TiffImportReport,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Kind {
    WhiteIsZero,
    BlackIsZero,
    Rgb,
    Palette,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Alpha {
    None,
    Straight,
    Premultiplied,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Geometry {
    Strips { rows: u32 },
    Tiles { width: u32, height: u32 },
}

/// Everything about how the picture is stored, checked.
struct Layout {
    order: Order,
    width: u32,
    height: u32,
    samples: usize,
    bits: u16,
    float: bool,
    kind: Kind,
    colour_samples: usize,
    alpha: Alpha,
    planar: bool,
    compression: u16,
    predictor: u16,
    orientation: u8,
    geometry: Geometry,
    offsets: Vec<u32>,
    counts: Vec<u32>,
    palette: Vec<[u16; 3]>,
}

impl Layout {
    fn planes(&self) -> usize {
        if self.planar {
            self.samples
        } else {
            1
        }
    }

    /// Samples in one stored pixel of a segment.
    fn group(&self) -> usize {
        if self.planar {
            1
        } else {
            self.samples
        }
    }

    fn row_bytes(&self, pixels: u32) -> u64 {
        (u64::from(pixels) * self.group() as u64 * u64::from(self.bits)).div_ceil(8)
    }

    fn per_plane(&self) -> u64 {
        match self.geometry {
            Geometry::Strips { rows } => u64::from(self.height.div_ceil(rows)),
            Geometry::Tiles { width, height } => {
                u64::from(self.width.div_ceil(width)) * u64::from(self.height.div_ceil(height))
            }
        }
    }

    /// The most bytes one decoded segment holds (saturating: a tile of four billion by four
    /// billion is a number, not an overflow, and is refused by the budget).
    fn largest_segment(&self) -> u64 {
        match self.geometry {
            Geometry::Strips { rows } => self.row_bytes(self.width).saturating_mul(u64::from(rows)),
            Geometry::Tiles { width, height } => {
                self.row_bytes(width).saturating_mul(u64::from(height))
            }
        }
    }

    fn out_size(&self) -> (u32, u32) {
        if self.orientation >= 5 {
            (self.height, self.width)
        } else {
            (self.width, self.height)
        }
    }
}

/// One strip or tile, located in the picture.
struct Segment {
    index: usize,
    plane: usize,
    x: u32,
    y: u32,
    /// The part inside the picture.
    width: u32,
    height: u32,
    /// What is stored: a tile is whole, and so is a strip, however the picture ends.
    stored_width: u32,
    stored_height: u32,
}

/// The `index`th strip or tile, planes one after another. `index` is below
/// `per_plane() * planes()`, which `read_layout` has checked against the file's own tables.
fn segment(layout: &Layout, index: usize) -> Segment {
    let per_plane = layout.per_plane() as usize;
    let plane = index / per_plane;
    let local = index % per_plane;
    match layout.geometry {
        Geometry::Strips { rows } => {
            let y = local as u32 * rows;
            let height = rows.min(layout.height - y);
            Segment {
                index,
                plane,
                x: 0,
                y,
                width: layout.width,
                height,
                stored_width: layout.width,
                stored_height: height,
            }
        }
        Geometry::Tiles { width, height } => {
            let across = layout.width.div_ceil(width) as usize;
            let (tx, ty) = ((local % across) as u32, (local / across) as u32);
            let (x, y) = (tx * width, ty * height);
            Segment {
                index,
                plane,
                x,
                y,
                width: width.min(layout.width - x),
                height: height.min(layout.height - y),
                stored_width: width,
                stored_height: height,
            }
        }
    }
}

fn photometric_name(code: u32) -> String {
    match code {
        4 => "a transparency mask".to_string(),
        5 => "CMYK (separated colour)".to_string(),
        6 => "YCbCr".to_string(),
        8..=10 => "CIE L*a*b*".to_string(),
        32803 => "a camera sensor mosaic (a RAW image; open it as a RAW file)".to_string(),
        34892 => "linear RAW".to_string(),
        32844 | 32845 => "LogLuv".to_string(),
        other => format!("photometric interpretation {other}"),
    }
}

fn read_layout<R: Read + Seek>(
    tiff: &mut TiffFile<R>,
    directory: &Directory,
) -> Result<Layout, AppError> {
    let order = tiff.order();
    let first = |tiff: &mut TiffFile<R>, tag: u16| tiff.first_u32(directory, tag);
    let list = |tiff: &mut TiffFile<R>, tag: u16| -> Result<Vec<u32>, AppError> {
        match directory.get(tag) {
            Some(entry) => {
                if u64::from(entry.count) > MAX_SEGMENTS {
                    return Err(refuse(format!(
                        "a table in the file has {} entries; at most {MAX_SEGMENTS} are read",
                        entry.count
                    )));
                }
                tiff.u32s(entry)
            }
            None => Ok(Vec::new()),
        }
    };

    let width = first(tiff, TAG_WIDTH)?.ok_or_else(|| refuse("the picture has no width"))?;
    let height = first(tiff, TAG_LENGTH)?.ok_or_else(|| refuse("the picture has no height"))?;
    if width == 0 || height == 0 {
        return Err(refuse("the picture has no area"));
    }
    validate_dimensions(width, height)?;

    let samples = first(tiff, TAG_SAMPLES)?.unwrap_or(1) as usize;
    if samples == 0 || samples > 16 {
        return Err(refuse(format!("{samples} samples per pixel is not read")));
    }

    let depths = list(tiff, TAG_BITS)?;
    let bits = match depths.first() {
        None => 1,
        Some(&first_depth) => {
            if depths.iter().any(|depth| *depth != first_depth) {
                return Err(refuse(
                    "the samples of this picture have different depths, which is not read",
                ));
            }
            first_depth as u16
        }
    };
    let formats = list(tiff, TAG_SAMPLE_FORMAT)?;
    let format = match formats.first() {
        None => 1,
        Some(&first_format) => {
            if formats.iter().any(|value| *value != first_format) {
                return Err(refuse(
                    "the samples of this picture have different formats, which is not read",
                ));
            }
            first_format
        }
    };
    let float = match format {
        1 => false,
        2 => return Err(refuse("signed integer samples are not read")),
        3 => true,
        _ => return Err(refuse("samples of an undefined format are not read")),
    };
    let depth_ok = if float {
        matches!(bits, 16 | 32 | 64)
    } else {
        matches!(bits, 1 | 2 | 4 | 8 | 16 | 32)
    };
    if !depth_ok {
        return Err(refuse(format!(
            "{bits}-bit {} samples are not read",
            if float { "floating-point" } else { "integer" }
        )));
    }

    let compression = first(tiff, TAG_COMPRESSION)?.unwrap_or(1) as u16;
    if !codec::is_supported(compression) {
        return Err(refuse(format!(
            "this picture is compressed with {}, which is not read (none, LZW, Deflate and PackBits are)",
            codec::compression_name(compression)
        )));
    }
    if first(tiff, TAG_FILL_ORDER)?.unwrap_or(1) != 1 {
        return Err(refuse(
            "this picture's bits are stored least significant first, which is not read",
        ));
    }

    let photometric = first(tiff, TAG_PHOTOMETRIC)?.ok_or_else(|| {
        refuse("the picture does not say how its samples are to be read as colour")
    })?;
    let kind = match photometric {
        0 => Kind::WhiteIsZero,
        1 => Kind::BlackIsZero,
        2 => Kind::Rgb,
        3 => Kind::Palette,
        other => {
            return Err(refuse(format!(
                "this picture is stored as {}, which is not read",
                photometric_name(other)
            )))
        }
    };
    let colour_samples = if kind == Kind::Rgb { 3 } else { 1 };
    if samples < colour_samples {
        return Err(refuse(
            "the picture has fewer samples per pixel than its colour model needs",
        ));
    }
    if matches!(kind, Kind::Palette) && (float || bits > 8) {
        return Err(refuse(
            "a palette picture can have at most 8 bits per index",
        ));
    }
    let planar = match first(tiff, TAG_PLANAR)?.unwrap_or(1) {
        1 => false,
        2 => samples > 1,
        _ => {
            return Err(refuse(
                "the picture's planar configuration is not one that is read",
            ))
        }
    };
    if bits < 8 && (samples != 1 || planar) {
        return Err(refuse(format!(
            "{bits}-bit samples are read only as a single grey or palette channel"
        )));
    }

    let predictor = first(tiff, TAG_PREDICTOR)?.unwrap_or(1) as u16;
    match (predictor, float, bits) {
        (1, _, _) => {}
        (2, false, 8 | 16 | 32) => {}
        (3, true, 16 | 32 | 64) => {}
        _ => {
            return Err(refuse(format!(
                "predictor {predictor} on {bits}-bit {} samples is not read",
                if float { "floating-point" } else { "integer" }
            )))
        }
    }

    let extra = list(tiff, TAG_EXTRA_SAMPLES)?;
    let alpha = if samples > colour_samples {
        match extra.first() {
            Some(1) => Alpha::Premultiplied,
            Some(2) => Alpha::Straight,
            _ => Alpha::None,
        }
    } else {
        Alpha::None
    };

    let orientation = first(tiff, TAG_ORIENTATION)?
        .filter(|value| (1..=8).contains(value))
        .unwrap_or(1) as u8;

    let tiled = directory.has(TAG_TILE_WIDTH) || directory.has(TAG_TILE_OFFSETS);
    let (geometry, offsets, counts) = if tiled {
        let tile_width = first(tiff, TAG_TILE_WIDTH)?.unwrap_or(0);
        let tile_height = first(tiff, TAG_TILE_LENGTH)?.unwrap_or(0);
        if tile_width == 0 || tile_height == 0 {
            return Err(refuse("the picture's tiles have no size"));
        }
        (
            Geometry::Tiles {
                width: tile_width,
                height: tile_height,
            },
            list(tiff, TAG_TILE_OFFSETS)?,
            list(tiff, TAG_TILE_COUNTS)?,
        )
    } else {
        let rows = first(tiff, TAG_ROWS_PER_STRIP)?
            .unwrap_or(u32::MAX)
            .clamp(1, height);
        (
            Geometry::Strips { rows },
            list(tiff, TAG_STRIP_OFFSETS)?,
            list(tiff, TAG_STRIP_COUNTS)?,
        )
    };

    let mut palette = Vec::new();
    if kind == Kind::Palette {
        let entries = 1_usize << bits;
        let map = list(tiff, TAG_COLOUR_MAP)?;
        if map.len() < entries * 3 {
            return Err(refuse("the colour map is shorter than the palette"));
        }
        for index in 0..entries {
            palette.push([
                map[index] as u16,
                map[entries + index] as u16,
                map[entries * 2 + index] as u16,
            ]);
        }
    }

    let layout = Layout {
        order,
        width,
        height,
        samples,
        bits,
        float,
        kind,
        colour_samples,
        alpha,
        planar,
        compression,
        predictor,
        orientation,
        geometry,
        offsets,
        counts,
        palette,
    };
    let needed = layout
        .per_plane()
        .checked_mul(layout.planes() as u64)
        .ok_or_else(|| refuse("the picture's segment count overflows"))?;
    if needed > MAX_SEGMENTS {
        return Err(refuse(format!(
            "the picture is stored in {needed} strips or tiles; at most {MAX_SEGMENTS} are read"
        )));
    }
    if (layout.offsets.len() as u64) < needed {
        return Err(refuse(
            "the picture has fewer strips or tiles than its size needs",
        ));
    }
    if layout.compression != codec::NONE && (layout.counts.len() as u64) < needed {
        return Err(refuse(
            "a compressed picture does not say how long its strips are",
        ));
    }
    // The strips are parts of the file and do not overlap, so what they declare together cannot
    // be more than the file. A table that says otherwise is one that would have the same bytes
    // read and decoded millions of times.
    let declared: u64 = layout
        .counts
        .iter()
        .take(needed as usize)
        .map(|count| u64::from(*count))
        .sum();
    if declared > tiff.length() {
        return Err(refuse(
            "the picture's strips declare more bytes than the file holds (they overlap, or run past its end)",
        ));
    }
    Ok(layout)
}

/// Where a pixel of the file goes in the upright picture.
fn place(orientation: u8, width: u32, height: u32, x: u32, y: u32) -> (u32, u32) {
    match orientation {
        2 => (width - 1 - x, y),
        3 => (width - 1 - x, height - 1 - y),
        4 => (x, height - 1 - y),
        5 => (y, x),
        6 => (height - 1 - y, x),
        7 => (height - 1 - y, width - 1 - x),
        8 => (y, width - 1 - x),
        _ => (x, y),
    }
}

fn half_to_f32(bits: u16) -> f32 {
    let sign = if bits & 0x8000 != 0 { -1.0_f32 } else { 1.0 };
    let exponent = i32::from((bits >> 10) & 0x1F);
    let fraction = f32::from(bits & 0x03FF);
    match exponent {
        0 => sign * fraction * 2.0_f32.powi(-24),
        31 => {
            if fraction == 0.0 {
                sign * f32::INFINITY
            } else {
                f32::NAN
            }
        }
        _ => sign * (1.0 + fraction / 1024.0) * 2.0_f32.powi(exponent - 15),
    }
}

/// One sample of a decoded row, as a number: integers as a fraction of full scale, floats as
/// they are.
fn sample(row: &[u8], index: usize, layout: &Layout) -> f32 {
    let order = layout.order;
    if layout.float {
        // Bytes the floating-point predictor has undone are big-endian whatever the file is.
        let order = if layout.predictor == 3 {
            Order::Big
        } else {
            order
        };
        return match layout.bits {
            16 => {
                let at = index * 2;
                half_to_f32(order.u16([row[at], row[at + 1]]))
            }
            32 => {
                let at = index * 4;
                f32::from_bits(order.u32([row[at], row[at + 1], row[at + 2], row[at + 3]]))
            }
            _ => {
                let at = index * 8;
                let high = u64::from(order.u32([row[at], row[at + 1], row[at + 2], row[at + 3]]));
                let low =
                    u64::from(order.u32([row[at + 4], row[at + 5], row[at + 6], row[at + 7]]));
                let value = match order {
                    Order::Little => (low << 32) | high,
                    Order::Big => (high << 32) | low,
                };
                f64::from_bits(value) as f32
            }
        };
    }
    match layout.bits {
        8 => f32::from(row[index]) / 255.0,
        16 => {
            let at = index * 2;
            f32::from(order.u16([row[at], row[at + 1]])) / 65535.0
        }
        32 => {
            let at = index * 4;
            (f64::from(order.u32([row[at], row[at + 1], row[at + 2], row[at + 3]]))
                / 4_294_967_295.0) as f32
        }
        bits => {
            let bits = usize::from(bits);
            let value = packed_index(row, index, bits);
            value as f32 / ((1_usize << bits) - 1) as f32
        }
    }
}

/// The integer at `index` of a row of 1-, 2-, 4- or 8-bit values, most significant bit first.
fn packed_index(row: &[u8], index: usize, bits: usize) -> usize {
    if bits == 8 {
        return usize::from(row[index]);
    }
    let per_byte = 8 / bits;
    let byte = row[index / per_byte];
    let shift = 8 - bits * (index % per_byte + 1);
    usize::from((byte >> shift) & ((1_u8 << bits) - 1))
}

/// The buffer a picture is assembled in, at the precision the document will have.
enum Raster {
    Eight(Vec<u8>),
    Float(Vec<FloatRgba>),
}

impl Raster {
    fn set(&mut self, pixel: usize, channel: usize, value: f32) {
        match self {
            Self::Eight(bytes) => {
                bytes[pixel * 4 + channel] = (value.clamp(0.0, 1.0) * 255.0).round() as u8;
            }
            Self::Float(pixels) => {
                let target = &mut pixels[pixel];
                match channel {
                    0 => target.red = value,
                    1 => target.green = value,
                    2 => target.blue = value,
                    _ => target.alpha = value,
                }
            }
        }
    }
}

/// What the colour of the file means, decided once.
struct Plan {
    transform: Option<IccRowTransform>,
    /// A document of linear float, not 8-bit sRGB.
    float: bool,
    /// Samples are sRGB-encoded and have to be decoded to linear light.
    decode_srgb: bool,
    description: String,
}

fn plan_colour(layout: &Layout, profile: Option<Vec<u8>>, report: &mut TiffImportReport) -> Plan {
    // A palette or grey picture's own profile (a grey profile, usually) does not describe
    // RGB, and an RGB transform of it would be wrong: only RGB pictures are managed.
    let mut transform = None;
    let mut srgb = false;
    if let Some(bytes) = &profile {
        if layout.kind == Kind::Rgb {
            match IccRowTransform::new(bytes) {
                Ok(found) => {
                    srgb = crate::psd::import::is_srgb_equivalent(&found);
                    transform = Some(found);
                }
                Err(_) => report.push(
                    ReportKind::Approximated,
                    "The embedded colour profile could not be read, so the colour was taken as sRGB.",
                ),
            }
        } else {
            report.push(
                ReportKind::Note,
                "The embedded profile describes a grey picture and was not applied; the picture was read as sRGB.",
            );
        }
    }
    let high = layout.bits > 8 || layout.float;
    let (transform, float, decode_srgb, description) = match (transform, srgb, high) {
        (Some(_), true, false) => (
            None,
            false,
            false,
            "8-bit sRGB (the embedded profile is equivalent to sRGB)".to_string(),
        ),
        (Some(_), true, true) if !layout.float => (
            None,
            true,
            true,
            format!(
                "{} with an sRGB profile, as a high-precision document",
                depth_name(layout)
            ),
        ),
        (Some(found), _, false) => (
            Some(found),
            true,
            false,
            "8-bit with an embedded profile, converted to a high-precision document".to_string(),
        ),
        (Some(found), _, true) => (
            Some(found),
            true,
            false,
            format!(
                "{} with an embedded profile, converted to a high-precision document",
                depth_name(layout)
            ),
        ),
        (None, _, false) => (None, false, false, "8-bit sRGB".to_string()),
        (None, _, true) if layout.float => (
            None,
            true,
            false,
            format!(
                "{} without a profile, read as linear light",
                depth_name(layout)
            ),
        ),
        (None, _, true) => (
            None,
            true,
            true,
            format!(
                "{} without a profile, read as sRGB, as a high-precision document",
                depth_name(layout)
            ),
        ),
    };
    Plan {
        transform,
        float,
        decode_srgb,
        description,
    }
}

fn depth_name(layout: &Layout) -> String {
    if layout.float {
        format!("{}-bit floating-point", layout.bits)
    } else {
        format!("{}-bit", layout.bits)
    }
}

fn plural(count: u64) -> &'static str {
    if count == 1 {
        ""
    } else {
        "s"
    }
}

#[derive(Default)]
struct Findings {
    non_finite: u64,
}

/// Reads a TIFF from `path` into a document and its report.
pub fn import_tiff(path: &Path, limits: ImportLimits) -> Result<ImportedTiff, AppError> {
    let metadata = std::fs::metadata(path).map_err(|_| AppError::DecodeFailure)?;
    if !metadata.is_file() {
        return Err(AppError::UnsupportedImageFormat);
    }
    if metadata.len() > MAX_TIFF_BYTES {
        return Err(refuse(format!(
            "the file is {} bytes; the largest TIFF read is {MAX_TIFF_BYTES}",
            metadata.len()
        )));
    }
    let file = std::fs::File::open(path).map_err(|_| AppError::Permission)?;
    import_from(file, limits)
}

/// As [`import_tiff`], from any seekable source.
pub fn import_from<R: Read + Seek>(
    reader: R,
    limits: ImportLimits,
) -> Result<ImportedTiff, AppError> {
    let mut tiff = TiffFile::open(reader)?;
    let directories = tiff.directories()?;

    // The picture is the first directory that is a full-resolution image: not a reduced copy,
    // not a mask, and with its strips or tiles.
    let mut main = None;
    for (index, directory) in directories.iter().enumerate() {
        let subfile = tiff
            .first_u32(directory, TAG_NEW_SUBFILE_TYPE)?
            .unwrap_or(0);
        let stored = directory.has(TAG_STRIP_OFFSETS) || directory.has(TAG_TILE_OFFSETS);
        if subfile & 0b101 == 0 && stored && directory.has(TAG_WIDTH) && directory.has(TAG_LENGTH) {
            main = Some(index);
            break;
        }
    }
    let Some(main) = main else {
        let inside = directories
            .iter()
            .any(|directory| directory.has(TAG_SUB_IFDS));
        return Err(refuse(if inside {
            "the pictures in this file are stored in sub-directories, as a camera RAW (DNG) file does; open it as a RAW file"
        } else {
            "the file holds no full-resolution picture"
        }));
    };
    let directory = &directories[main];
    let layout = read_layout(&mut tiff, directory)?;
    let (width, height) = (layout.width, layout.height);
    let (out_width, out_height) = layout.out_size();

    let mut report = TiffImportReport {
        format: "TIFF".to_string(),
        width: out_width,
        height: out_height,
        bit_depth: layout.bits,
        color_mode: if layout.kind == Kind::Rgb || layout.kind == Kind::Palette {
            "RGB"
        } else {
            "Grayscale"
        }
        .to_string(),
        layers_imported: 1,
        ..TiffImportReport::default()
    };

    // The profile, then the plan the colour will be read under.
    let profile = match directory.get(TAG_ICC) {
        Some(entry) if u64::from(entry.count) <= super::structure::MAX_VALUE_BYTES => {
            tiff.value_bytes(entry).ok()
        }
        _ => None,
    };
    let plan = plan_colour(&layout, profile, &mut report);

    // Admission, before a buffer exists.
    let pixels = u64::from(width) * u64::from(height);
    let bytes_per_pixel: u64 = if plan.float { 16 } else { 4 };
    let raster_bytes = pixels
        .checked_mul(bytes_per_pixel)
        .ok_or_else(|| refuse("the picture's size overflows"))?;
    let largest = layout.largest_segment();
    let cap = largest.saturating_mul(2).saturating_add(65_536);
    let peak = raster_bytes
        .saturating_add(largest)
        .saturating_add(cap)
        .saturating_add(if plan.transform.is_some() {
            u64::from(out_width) * 32
        } else {
            0
        });
    if raster_bytes > limits.entry_bytes || raster_bytes > limits.store_bytes {
        return Err(refuse(format!(
            "the picture is {width} × {height} pixels, which as a document needs {} MB; the memory budget allows {} MB for one picture",
            raster_bytes.div_ceil(1 << 20),
            limits.entry_bytes.min(limits.store_bytes) >> 20
        )));
    }
    if peak > limits.job_bytes {
        return Err(refuse(format!(
            "reading this picture would need about {} MB at its peak; the memory budget allows {} MB",
            peak.div_ceil(1 << 20),
            limits.job_bytes >> 20
        )));
    }

    // The raster, transparent black until written.
    let count = (out_width as usize) * (out_height as usize);
    let mut raster = if plan.float {
        let mut buffer = Vec::new();
        buffer
            .try_reserve_exact(count)
            .map_err(|_| AppError::OutOfMemoryRisk)?;
        buffer.resize(
            count,
            FloatRgba {
                red: 0.0,
                green: 0.0,
                blue: 0.0,
                alpha: 1.0,
            },
        );
        Raster::Float(buffer)
    } else {
        let mut buffer = Vec::new();
        buffer
            .try_reserve_exact(count * 4)
            .map_err(|_| AppError::OutOfMemoryRisk)?;
        for _ in 0..count {
            buffer.extend_from_slice(&[0, 0, 0, 255]);
        }
        Raster::Eight(buffer)
    };

    let mut findings = Findings::default();
    let total = layout.per_plane() as usize * layout.planes();
    for number in 0..total {
        let segment = segment(&layout, number);
        let expected = layout.row_bytes(segment.stored_width) * u64::from(segment.stored_height);
        let expected = usize::try_from(expected).map_err(|_| AppError::OutOfMemoryRisk)?;
        let offset = u64::from(layout.offsets[segment.index]);
        let declared = match layout.counts.get(segment.index) {
            Some(&count) if layout.compression != codec::NONE => u64::from(count),
            Some(&count) if count != 0 => u64::from(count),
            _ => expected as u64,
        };
        let compressed = tiff.read_at(offset, declared, cap)?;
        let mut data =
            codec::decompress(layout.compression, &compressed, expected).map_err(refuse)?;
        drop(compressed);
        let row_bytes = layout.row_bytes(segment.stored_width) as usize;
        if layout.predictor != 1 && row_bytes > 0 {
            for row in data.chunks_exact_mut(row_bytes) {
                match layout.predictor {
                    2 => codec::undo_horizontal(row, layout.group(), layout.bits, layout.order),
                    _ => codec::undo_floating_point(
                        row,
                        layout.group(),
                        usize::from(layout.bits / 8),
                    ),
                }
            }
        }
        write_segment(
            &layout,
            &segment,
            &data,
            row_bytes,
            &mut raster,
            &mut findings,
        );
    }

    finish(&layout, &plan, &mut raster, (out_width, out_height))?;

    // The report.
    report.push(ReportKind::Note, plan.description.clone());
    if layout.orientation != 1 {
        report.push(
            ReportKind::Note,
            format!(
                "The picture was stored sideways (orientation {}); it was turned upright.",
                layout.orientation
            ),
        );
    }
    match layout.kind {
        Kind::Palette => report.push(
            ReportKind::Note,
            "A palette picture; its colours were expanded to RGB.",
        ),
        Kind::WhiteIsZero => report.push(
            ReportKind::Note,
            "Stored with white as zero; the values were inverted.",
        ),
        _ => {}
    }
    if layout.bits < 8 {
        report.push(
            ReportKind::Note,
            format!(
                "Stored at {} bit{} per sample; read as 8-bit.",
                layout.bits,
                plural(u64::from(layout.bits))
            ),
        );
    }
    let extras = layout.samples - layout.colour_samples - usize::from(layout.alpha != Alpha::None);
    if extras > 0 {
        report.push(
            ReportKind::NotImported,
            format!(
                "{extras} extra channel{} (spot colour, mask or alpha with no stated meaning) not read as colour.",
                plural(extras as u64)
            ),
        );
    }
    let others = directories.len() - 1;
    if others > 0 {
        report.push(
            ReportKind::NotImported,
            format!(
                "{others} other image{} in the file (further pages or reduced-resolution copies) {} not imported.",
                plural(others as u64),
                if others == 1 { "was" } else { "were" }
            ),
        );
    }
    if directory.has(TAG_PHOTOSHOP_LAYERS) {
        report.push(
            ReportKind::NotImported,
            "The file was saved by Photoshop with its layers; the flattened picture was imported, not the layers.",
        );
    }
    if findings.non_finite > 0 {
        report.push(
            ReportKind::Approximated,
            format!(
                "{} sample{} held a value that is not a number or is infinite; not-a-number became zero and infinity the largest finite value.",
                findings.non_finite,
                plural(findings.non_finite)
            ),
        );
    }

    // The document.
    let now = SystemClock.now();
    let mut document = LayerDocument::new(out_width, out_height);
    document.precision = if plan.float {
        DocumentPrecision::LinearSrgbF32
    } else {
        DocumentPrecision::LegacySrgb8
    };
    let mut layer = new_layer(
        "l1",
        "Background",
        LayerContent::Pixel {
            pixel_id: "px1".to_string(),
            width: out_width,
            height: out_height,
        },
        &now,
    );
    layer.visible = true;
    document.active_layer_id = Some(layer.id.clone());
    document.layers = vec![layer];
    document.validate()?;

    let (pixels8, pixels_float) = match raster {
        Raster::Eight(bytes) => {
            let image = image::RgbaImage::from_raw(out_width, out_height, bytes)
                .ok_or(AppError::CorruptImage)?;
            (vec![("px1".to_string(), image)], Vec::new())
        }
        Raster::Float(pixels) => {
            let image = FloatImage::new(out_width, out_height, pixels)?;
            (Vec::new(), vec![("px1".to_string(), image)])
        }
    };

    // The photograph's own EXIF, XMP and IPTC, with the orientation spent on the pixels.
    let mut source_metadata = super::metadata::read(&mut tiff, directory);
    if let Some(found) = source_metadata.as_mut() {
        found.orientation_applied = true;
    }
    let source_metadata = source_metadata.filter(|found| !found.is_empty());

    let project = LoadedProject {
        document,
        document_operations: Vec::new(),
        pixels: pixels8,
        linear_pixels: pixels_float,
        application_version: env!("CARGO_PKG_VERSION").into(),
        created_at: now.clone(),
        modified_at: now,
        source_metadata,
    };
    Ok(ImportedTiff { project, report })
}

/// Writes the pixels of one decoded segment into the raster, at their upright positions.
fn write_segment(
    layout: &Layout,
    segment: &Segment,
    data: &[u8],
    row_bytes: usize,
    raster: &mut Raster,
    findings: &mut Findings,
) {
    let (out_width, _) = layout.out_size();
    let indexed = layout.kind == Kind::Palette;
    let invert = layout.kind == Kind::WhiteIsZero;
    for row in 0..segment.height as usize {
        let line = &data[row * row_bytes..(row + 1) * row_bytes];
        let y = segment.y + row as u32;
        for column in 0..segment.width as usize {
            let x = segment.x + column as u32;
            let (dx, dy) = place(layout.orientation, layout.width, layout.height, x, y);
            let pixel = dy as usize * out_width as usize + dx as usize;
            let group = layout.group();
            let mut value_of = |channel: usize| -> f32 {
                let raw = sample(line, column * group + channel, layout);
                if raw.is_finite() {
                    raw
                } else {
                    findings.non_finite += 1;
                    if raw.is_nan() {
                        0.0
                    } else if raw > 0.0 {
                        f32::MAX
                    } else {
                        f32::MIN
                    }
                }
            };
            if layout.planar {
                // One channel of the picture per segment.
                let channel = segment.plane;
                if channel < layout.colour_samples {
                    let mut value = value_of(0);
                    if invert {
                        value = 1.0 - value;
                    }
                    if layout.colour_samples == 1 {
                        for target in 0..3 {
                            raster.set(pixel, target, value);
                        }
                    } else {
                        raster.set(pixel, channel, value);
                    }
                } else if channel == layout.colour_samples && layout.alpha != Alpha::None {
                    raster.set(pixel, 3, value_of(0));
                }
                continue;
            }
            if indexed {
                let index = packed_index(line, column, usize::from(layout.bits));
                let colour = layout.palette.get(index).copied().unwrap_or([0, 0, 0]);
                for (target, component) in colour.iter().enumerate() {
                    raster.set(pixel, target, f32::from(*component) / 65535.0);
                }
            } else if layout.colour_samples == 1 {
                let mut value = value_of(0);
                if invert {
                    value = 1.0 - value;
                }
                for target in 0..3 {
                    raster.set(pixel, target, value);
                }
            } else {
                for channel in 0..3 {
                    raster.set(pixel, channel, value_of(channel));
                }
            }
            if layout.alpha != Alpha::None {
                raster.set(pixel, 3, value_of(layout.colour_samples));
            }
        }
    }
}

/// Everything that needs the whole picture: premultiplied alpha made straight, and colour taken
/// to the document's space.
fn finish(
    layout: &Layout,
    plan: &Plan,
    raster: &mut Raster,
    (width, _height): (u32, u32),
) -> Result<(), AppError> {
    let premultiplied = layout.alpha == Alpha::Premultiplied;
    match raster {
        Raster::Eight(bytes) => {
            if premultiplied {
                for pixel in bytes.chunks_exact_mut(4) {
                    let alpha = u32::from(pixel[3]);
                    if alpha == 0 {
                        pixel[..3].fill(0);
                    } else if alpha < 255 {
                        for channel in &mut pixel[..3] {
                            let straight = (u32::from(*channel) * 255 + alpha / 2) / alpha;
                            *channel = straight.min(255) as u8;
                        }
                    }
                }
            }
        }
        Raster::Float(pixels) => {
            if premultiplied {
                for pixel in pixels.iter_mut() {
                    if pixel.alpha > 0.0 {
                        pixel.red /= pixel.alpha;
                        pixel.green /= pixel.alpha;
                        pixel.blue /= pixel.alpha;
                    } else {
                        pixel.red = 0.0;
                        pixel.green = 0.0;
                        pixel.blue = 0.0;
                    }
                }
            }
            if let Some(transform) = &plan.transform {
                let row_pixels = width as usize;
                let mut encoded = vec![0.0_f32; row_pixels * 4];
                let mut linear = vec![0.0_f32; row_pixels * 4];
                for row in pixels.chunks_mut(row_pixels) {
                    for (pixel, slot) in row.iter().zip(encoded.chunks_exact_mut(4)) {
                        slot.copy_from_slice(&[pixel.red, pixel.green, pixel.blue, pixel.alpha]);
                    }
                    let used = row.len() * 4;
                    transform.apply(&encoded[..used], &mut linear[..used])?;
                    for (pixel, slot) in row.iter_mut().zip(linear.chunks_exact(4)) {
                        pixel.red = slot[0];
                        pixel.green = slot[1];
                        pixel.blue = slot[2];
                        pixel.alpha = slot[3];
                    }
                }
            } else if plan.decode_srgb {
                for pixel in pixels.iter_mut() {
                    pixel.red = srgb_decode(pixel.red);
                    pixel.green = srgb_decode(pixel.green);
                    pixel.blue = srgb_decode(pixel.blue);
                }
            }
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests;
