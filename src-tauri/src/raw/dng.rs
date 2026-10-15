//! DNG sensor decoding.
//!
//! Adobe's Digital Negative is a published, openly specified TIFF/EP
//! derivative, which is why PhotoForge can decode it from first principles
//! instead of linking a copyleft decoder. This module turns a DNG file into a
//! `SensorImage`: the camera's own CFA samples, together with the levels,
//! pattern, matrices, and orientation needed to develop them honestly.
//!
//! It reads what the specification defines and refuses the rest. A file whose
//! compression, photometric interpretation, or CFA layout is not implemented is
//! reported as unsupported by name rather than decoded approximately.

use super::ljpeg;
use super::tiff::{self, Ifd, TiffFile};
use super::{raw_max_pixels, RawCaptureMetadata, RawError};
use crate::source::Rect;

// Baseline TIFF tags.
const TAG_NEW_SUBFILE_TYPE: u16 = 254;
const TAG_IMAGE_WIDTH: u16 = 256;
const TAG_IMAGE_LENGTH: u16 = 257;
const TAG_BITS_PER_SAMPLE: u16 = 258;
const TAG_COMPRESSION: u16 = 259;
const TAG_PHOTOMETRIC: u16 = 262;
const TAG_MAKE: u16 = 271;
const TAG_MODEL: u16 = 272;
const TAG_STRIP_OFFSETS: u16 = 273;
const TAG_ORIENTATION: u16 = 274;
const TAG_SAMPLES_PER_PIXEL: u16 = 277;
const TAG_ROWS_PER_STRIP: u16 = 278;
const TAG_STRIP_BYTE_COUNTS: u16 = 279;
const TAG_TILE_WIDTH: u16 = 322;
const TAG_TILE_LENGTH: u16 = 323;
const TAG_TILE_OFFSETS: u16 = 324;
const TAG_TILE_BYTE_COUNTS: u16 = 325;

// TIFF/EP and Exif tags.
const TAG_CFA_REPEAT_DIM: u16 = 33421;
const TAG_CFA_PATTERN: u16 = 33422;
const TAG_EXPOSURE_TIME: u16 = 33434;
const TAG_F_NUMBER: u16 = 33437;
const TAG_EXPOSURE_BIAS: u16 = 37380;
const TAG_ISO: u16 = 34855;
const TAG_DATE_TIME_ORIGINAL: u16 = 36867;
const TAG_FOCAL_LENGTH: u16 = 37386;
const TAG_LENS_MODEL: u16 = 42036;

// DNG tags.
const TAG_DNG_VERSION: u16 = 50706;
const TAG_BLACK_LEVEL_REPEAT_DIM: u16 = 50713;
const TAG_BLACK_LEVEL: u16 = 50714;
const TAG_WHITE_LEVEL: u16 = 50717;
const TAG_DEFAULT_CROP_ORIGIN: u16 = 50719;
const TAG_DEFAULT_CROP_SIZE: u16 = 50720;
const TAG_COLOR_MATRIX_1: u16 = 50721;
const TAG_COLOR_MATRIX_2: u16 = 50722;
const TAG_AS_SHOT_NEUTRAL: u16 = 50728;
const TAG_ACTIVE_AREA: u16 = 50829;
const TAG_MASKED_AREAS: u16 = 50741;

const PHOTOMETRIC_CFA: u32 = 32803;
const PHOTOMETRIC_LINEAR_RAW: u32 = 34892;
const COMPRESSION_NONE: u32 = 1;
const COMPRESSION_LOSSLESS_JPEG: u32 = 7;

/// Largest number of strips or tiles a single image may declare.
const MAX_SEGMENTS: usize = 65_536;

/// Which colour each position of the repeating CFA block records.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CfaColor {
    Red,
    Green,
    Blue,
}

/// A two-by-two colour filter array, which covers every Bayer layout.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CfaPattern {
    /// Row-major, `[top-left, top-right, bottom-left, bottom-right]`.
    pub cells: [CfaColor; 4],
}

impl CfaPattern {
    pub const RGGB: Self = Self {
        cells: [
            CfaColor::Red,
            CfaColor::Green,
            CfaColor::Green,
            CfaColor::Blue,
        ],
    };
    pub const BGGR: Self = Self {
        cells: [
            CfaColor::Blue,
            CfaColor::Green,
            CfaColor::Green,
            CfaColor::Red,
        ],
    };
    pub const GRBG: Self = Self {
        cells: [
            CfaColor::Green,
            CfaColor::Red,
            CfaColor::Blue,
            CfaColor::Green,
        ],
    };
    pub const GBRG: Self = Self {
        cells: [
            CfaColor::Green,
            CfaColor::Blue,
            CfaColor::Red,
            CfaColor::Green,
        ],
    };

    /// The colour recorded at a sensor coordinate.
    pub fn color_at(&self, x: u32, y: u32) -> CfaColor {
        self.cells[((y % 2) * 2 + (x % 2)) as usize]
    }

    /// The index into a four-element per-position array, matching `color_at`.
    pub const fn cell_index(x: u32, y: u32) -> usize {
        ((y % 2) * 2 + (x % 2)) as usize
    }

    fn from_codes(codes: &[u32]) -> Option<Self> {
        if codes.len() < 4 {
            return None;
        }
        let mut cells = [CfaColor::Green; 4];
        for (index, code) in codes.iter().take(4).enumerate() {
            cells[index] = match code {
                0 => CfaColor::Red,
                1 => CfaColor::Green,
                2 => CfaColor::Blue,
                // 3 is cyan, 4 magenta, 5 yellow, 6 white: not Bayer.
                _ => return None,
            };
        }
        Some(Self { cells })
    }

    pub fn name(&self) -> &'static str {
        match *self {
            Self::RGGB => "RGGB",
            Self::BGGR => "BGGR",
            Self::GRBG => "GRBG",
            Self::GBRG => "GBRG",
            _ => "custom",
        }
    }
}

/// A decoded sensor image, still in the camera's own colour space.
#[derive(Debug, Clone)]
pub struct SensorImage {
    /// CFA samples in row-major order, one per photosite.
    pub data: Vec<u16>,
    pub width: u32,
    pub height: u32,
    pub cfa: CfaPattern,
    /// Black level for each CFA position, in sensor units. The file's own `BlackLevel` when it
    /// declares one; otherwise the level measured on its masked photosites when it has any,
    /// otherwise zero.
    pub black_level: [f32; 4],
    /// The mean of the file's masked (optically black) photosites at each CFA position, when it
    /// declares masked areas that cover every position. Recorded whether or not the file also
    /// declares a black level, which stays authoritative; a large disagreement between the two
    /// is something a person may want to know about.
    pub masked_black_level: Option<[f32; 4]>,
    pub white_level: f32,
    pub bits_per_sample: u8,
    /// The region of the sensor that holds picture rather than calibration
    /// data, as `(left, top, width, height)`.
    pub active_area: (u32, u32, u32, u32),
    /// The camera's own suggested crop within the active area.
    pub default_crop: Option<(u32, u32, u32, u32)>,
    /// As-shot white balance, as the neutral the camera recorded.
    pub as_shot_neutral: Option<[f32; 3]>,
    /// Camera RGB to linear sRGB (D65), derived from the DNG colour matrix.
    pub camera_to_srgb: Option<[[f32; 3]; 3]>,
    pub orientation: u16,
    pub metadata: RawCaptureMetadata,
    /// Whether the file was already demosaiced (LinearRaw) rather than CFA.
    pub linear: bool,
}

/// What a caller needs to know about a sensor once its samples are gone.
///
/// Development consumes the samples; a layer then needs the camera's description
/// of itself, not two bytes per photosite it will never read again.
#[derive(Debug, Clone)]
pub struct SensorSummary {
    pub metadata: RawCaptureMetadata,
    pub cfa_name: String,
    pub bits_per_sample: u8,
    pub black_level: [f32; 4],
    pub white_level: f32,
}

/// The picture of a `width` × `height` sensor with this active area and default crop, in sensor
/// coordinates; `None` when it is the whole sensor or does not lie on it. One rule for the
/// decoded sensor and for the header alone (`inspect_geometry`).
fn picture_of(
    width: u32,
    height: u32,
    active_area: (u32, u32, u32, u32),
    default_crop: Option<(u32, u32, u32, u32)>,
) -> Option<Rect> {
    let (area_x, area_y, area_width, area_height) = active_area;
    let within = |x: u32, y: u32, w: u32, h: u32| {
        w > 0
            && h > 0
            && u64::from(x) + u64::from(w) <= u64::from(area_width)
            && u64::from(y) + u64::from(h) <= u64::from(area_height)
    };
    let (x, y, w, h) = match default_crop {
        Some((x, y, w, h)) if within(x, y, w, h) => (area_x + x, area_y + y, w, h),
        _ => (area_x, area_y, area_width, area_height),
    };
    let rect = Rect {
        x,
        y,
        width: w,
        height: h,
    };
    if !rect.is_within(width, height) {
        return None;
    }
    (rect.pixels() != u64::from(width) * u64::from(height)).then_some(rect)
}

/// Where a DNG's picture lies on its sensor and which way it is turned, read from the header
/// alone: what a region has to be chosen on, before a sample is decoded.
///
/// The picture a development is cut to (`SensorImage::picture_rect`) and turned upright by the
/// orientation tag is what the interface calls the image. A person chooses a region on *that*,
/// so the rectangle is in the picture's own coordinates, and `to_sensor` says which photosites it
/// is. The same geometry maps a sensor rectangle back, for a region saved in sensor coordinates.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PictureGeometry {
    pub sensor_width: u32,
    pub sensor_height: u32,
    /// The picture in sensor coordinates: the default crop inside the active area, the active
    /// area, or the whole sensor.
    pub rect: Rect,
    /// The EXIF orientation (1 to 8) that turns the picture upright.
    pub orientation: u8,
}

impl PictureGeometry {
    /// The part of the sensor a whole development is cut to; `None` when it is all of it.
    pub fn cut(&self) -> Option<Rect> {
        (self.rect.pixels() != u64::from(self.sensor_width) * u64::from(self.sensor_height))
            .then_some(self.rect)
    }

    /// The picture's size once it is turned upright: the size of the image a person sees.
    pub fn presented_size(&self) -> (u32, u32) {
        crate::orientation::oriented_size(self.orientation, self.rect.width, self.rect.height)
    }

    /// A rectangle chosen on the upright picture, as the rectangle of the sensor it is made of.
    /// `None` when it is empty or does not lie on the picture.
    pub fn to_sensor(&self, chosen: &Rect) -> Option<Rect> {
        let (width, height) = self.presented_size();
        if !chosen.is_within(width, height) {
            return None;
        }
        let last_x = chosen.x + chosen.width - 1;
        let last_y = chosen.y + chosen.height - 1;
        let (width, height) = (self.rect.width, self.rect.height);
        let first =
            crate::orientation::unplace(self.orientation, width, height, chosen.x, chosen.y);
        let last = crate::orientation::unplace(self.orientation, width, height, last_x, last_y);
        Some(Rect {
            x: self.rect.x + first.0.min(last.0),
            y: self.rect.y + first.1.min(last.1),
            width: first.0.abs_diff(last.0) + 1,
            height: first.1.abs_diff(last.1) + 1,
        })
    }

    /// A rectangle of the sensor that lies on the picture, as it is on the upright picture.
    /// `None` when it is empty or any part of it is outside the picture.
    pub fn from_sensor(&self, rect: &Rect) -> Option<Rect> {
        if rect.width == 0
            || rect.height == 0
            || rect.x < self.rect.x
            || rect.y < self.rect.y
            || rect.right() > self.rect.right()
            || rect.bottom() > self.rect.bottom()
        {
            return None;
        }
        let (local_x, local_y) = (rect.x - self.rect.x, rect.y - self.rect.y);
        let (width, height) = (self.rect.width, self.rect.height);
        let first = crate::orientation::place(self.orientation, width, height, local_x, local_y);
        let last = crate::orientation::place(
            self.orientation,
            width,
            height,
            local_x + rect.width - 1,
            local_y + rect.height - 1,
        );
        Some(Rect {
            x: first.0.min(last.0),
            y: first.1.min(last.1),
            width: first.0.abs_diff(last.0) + 1,
            height: first.1.abs_diff(last.1) + 1,
        })
    }
}

impl SensorImage {
    /// The part of the sensor that is the picture, in sensor coordinates, when that is not all
    /// of it: the default crop the file declares, inside its active area, or the active area
    /// itself when it declares none. `None` when the picture is the whole sensor.
    ///
    /// The active area excludes the optically black and masked photosites around the image; the
    /// default crop (an origin relative to the active area's top-left corner, and a size) is the
    /// part the camera maker's own converter shows. A crop that does not lie inside the active
    /// area is not believed, and the active area is used.
    pub fn picture_rect(&self) -> Option<Rect> {
        picture_of(self.width, self.height, self.active_area, self.default_crop)
    }

    pub fn summary(&self) -> SensorSummary {
        SensorSummary {
            metadata: self.metadata.clone(),
            cfa_name: self.cfa.name().to_string(),
            bits_per_sample: self.bits_per_sample,
            black_level: self.black_level,
            white_level: self.white_level,
        }
    }

    pub fn sample(&self, x: u32, y: u32) -> u16 {
        let index = y as usize * self.width as usize + x as usize;
        self.data.get(index).copied().unwrap_or(0)
    }

    /// The black level that applies at a sensor coordinate.
    pub fn black_at(&self, x: u32, y: u32) -> f32 {
        self.black_level[CfaPattern::cell_index(x, y)]
    }
}

/// Whether a buffer looks like a DNG this module can attempt.
pub fn is_dng(data: &[u8]) -> bool {
    let Ok(file) = tiff::parse(data) else {
        return false;
    };
    file.ifds
        .iter()
        .any(|ifd| ifd.get(TAG_DNG_VERSION).is_some())
}

/// Decodes a DNG buffer into its whole sensor image.
pub fn decode(data: &[u8]) -> Result<SensorImage, RawError> {
    decode_inner(data, None)
}

/// Decodes only the part of the sensor inside `window`.
///
/// Segments that do not overlap the window are never decoded, so decoded memory
/// follows the window and, for a tiled file, so does the work. The origin must be
/// even in both directions: the Bayer pattern repeats every two photosites, so an
/// even origin keeps every photosite's colour the same inside the window as it is
/// on the sensor.
///
/// What this does **not** save is the file itself. The caller holds the compressed
/// bytes, so a region bounds the *decoded* sensor and everything developed from it,
/// not the read of the file.
pub fn decode_window(data: &[u8], window: Rect) -> Result<SensorImage, RawError> {
    decode_inner(data, Some(window))
}

/// How the sensor image of a DNG is divided up, without decoding any of it.
pub fn inspect_layout(data: &[u8]) -> Result<SegmentLayout, RawError> {
    let file = tiff::parse(data).map_err(|error| RawError::Malformed(error.to_string()))?;
    let raw_index = choose_raw_ifd(&file).ok_or_else(|| {
        RawError::Malformed("the file contains no full-resolution sensor image".into())
    })?;
    let raw = &file.ifds[raw_index];
    let width = raw
        .u32(TAG_IMAGE_WIDTH)
        .ok_or_else(|| RawError::Malformed("the sensor image declares no width".into()))?;
    let height = raw
        .u32(TAG_IMAGE_LENGTH)
        .ok_or_else(|| RawError::Malformed("the sensor image declares no height".into()))?;
    if width == 0
        || height == 0
        || u64::from(width) > crate::resources::HARD_MAX_SOURCE_DIMENSION
        || u64::from(height) > crate::resources::HARD_MAX_SOURCE_DIMENSION
    {
        return Err(RawError::InvalidMetadata(
            "the sensor image has implausible dimensions".into(),
        ));
    }
    let bits = u8::try_from(raw.u32(TAG_BITS_PER_SAMPLE).unwrap_or(16))
        .map_err(|_| RawError::Unsupported("invalid BitsPerSample".into()))?;
    segment_layout(
        data,
        raw,
        width,
        height,
        bits,
        raw.u32(TAG_COMPRESSION).unwrap_or(COMPRESSION_NONE),
    )
}

/// Where the picture lies on the sensor and how it is turned, from the header alone: no sample
/// is decoded. The rules are those of a development (`SensorImage::picture_rect` and the
/// orientation tag), so a region chosen with this is cut from the same picture a whole open
/// would show.
pub fn inspect_geometry(data: &[u8]) -> Result<PictureGeometry, RawError> {
    let file = tiff::parse(data).map_err(|error| RawError::Malformed(error.to_string()))?;
    let raw_index = choose_raw_ifd(&file).ok_or_else(|| {
        RawError::Malformed("the file contains no full-resolution sensor image".into())
    })?;
    let raw = &file.ifds[raw_index];
    let width = raw
        .u32(TAG_IMAGE_WIDTH)
        .ok_or_else(|| RawError::Malformed("the sensor image declares no width".into()))?;
    let height = raw
        .u32(TAG_IMAGE_LENGTH)
        .ok_or_else(|| RawError::Malformed("the sensor image declares no height".into()))?;
    if width == 0
        || height == 0
        || u64::from(width) > crate::resources::HARD_MAX_SOURCE_DIMENSION
        || u64::from(height) > crate::resources::HARD_MAX_SOURCE_DIMENSION
    {
        return Err(RawError::InvalidMetadata(
            "the sensor image has implausible dimensions".into(),
        ));
    }
    let whole = Rect {
        x: 0,
        y: 0,
        width,
        height,
    };
    let rect = picture_of(
        width,
        height,
        read_active_area(raw, width, height),
        read_default_crop(raw),
    )
    .unwrap_or(whole);
    Ok(PictureGeometry {
        sensor_width: width,
        sensor_height: height,
        rect,
        orientation: u8::try_from(read_orientation(&file)).unwrap_or(1),
    })
}

/// Calls `visit(x, y, width, height, samples)` for every segment of the sensor,
/// one at a time, so a statistic over the whole sensor needs only one segment
/// in memory.
pub fn visit_sensor(
    data: &[u8],
    visit: impl FnMut(u32, u32, u32, u32, &[u16]),
) -> Result<(), RawError> {
    visit_sensor_until(data, None, visit)
}

/// As `visit_sensor`, stopping with `RawError::Cancelled` between segments once
/// `cancel` is set.
pub fn visit_sensor_until(
    data: &[u8],
    cancel: Option<&std::sync::atomic::AtomicBool>,
    mut visit: impl FnMut(u32, u32, u32, u32, &[u16]),
) -> Result<(), RawError> {
    let layout = inspect_layout(data)?;
    let whole = Rect {
        x: 0,
        y: 0,
        width: layout.width,
        height: layout.height,
    };
    visit_segments(data, &layout, &whole, cancel, |segment| {
        visit(
            segment.x,
            segment.y,
            segment.width,
            segment.height,
            &segment.samples,
        );
    })
}

fn decode_inner(data: &[u8], window: Option<Rect>) -> Result<SensorImage, RawError> {
    let file = tiff::parse(data).map_err(|error| RawError::Malformed(error.to_string()))?;
    if !file
        .ifds
        .iter()
        .any(|ifd| ifd.get(TAG_DNG_VERSION).is_some())
    {
        return Err(RawError::Malformed(
            "the file is TIFF but carries no DNG version tag".into(),
        ));
    }
    let raw_index = choose_raw_ifd(&file).ok_or_else(|| {
        RawError::Malformed("the file contains no full-resolution sensor image".into())
    })?;
    let raw = &file.ifds[raw_index];

    let width = raw
        .u32(TAG_IMAGE_WIDTH)
        .ok_or_else(|| RawError::Malformed("the sensor image declares no width".into()))?;
    let height = raw
        .u32(TAG_IMAGE_LENGTH)
        .ok_or_else(|| RawError::Malformed("the sensor image declares no height".into()))?;
    // A whole decode is bounded by the pixel limit. A window decode is bounded by
    // the window, and the sensor it is cut from need only be a plausible size.
    let area = match window {
        None => {
            super::validate_dimensions(width, height)?;
            Rect {
                x: 0,
                y: 0,
                width,
                height,
            }
        }
        Some(window) => {
            if u64::from(width) > crate::resources::HARD_MAX_SOURCE_DIMENSION
                || u64::from(height) > crate::resources::HARD_MAX_SOURCE_DIMENSION
                || width == 0
                || height == 0
            {
                return Err(RawError::InvalidMetadata(
                    "the sensor image has implausible dimensions".into(),
                ));
            }
            if !window.is_within(width, height) {
                return Err(RawError::InvalidMetadata(
                    "the requested window lies outside the sensor".into(),
                ));
            }
            if window.x % 2 != 0 || window.y % 2 != 0 {
                return Err(RawError::InvalidMetadata(
                    "a window must start on an even photosite to keep the Bayer phase".into(),
                ));
            }
            super::validate_dimensions(window.width, window.height)?;
            window
        }
    };
    crate::resources::ResourceEstimate::decode(area.width, area.height, data.len() as u64)
        .map_err(|error| RawError::InvalidMetadata(error.to_string()))?;

    let photometric = raw.u32(TAG_PHOTOMETRIC).unwrap_or(PHOTOMETRIC_CFA);
    let linear = photometric == PHOTOMETRIC_LINEAR_RAW;
    if !linear && photometric != PHOTOMETRIC_CFA {
        return Err(RawError::Unsupported(format!(
            "photometric interpretation {photometric} is not a camera sensor image"
        )));
    }
    let samples_per_pixel = raw.u32(TAG_SAMPLES_PER_PIXEL).unwrap_or(1);
    if samples_per_pixel != 1 {
        return Err(RawError::Unsupported(format!(
            "{samples_per_pixel} samples per pixel is not supported; only Bayer CFA and monochrome LinearRaw are implemented"
        )));
    }
    let bits_per_sample = u8::try_from(raw.u32(TAG_BITS_PER_SAMPLE).unwrap_or(16))
        .map_err(|_| RawError::Unsupported("invalid BitsPerSample".into()))?;
    if !matches!(bits_per_sample, 8 | 10 | 12 | 14 | 16) {
        return Err(RawError::Unsupported(format!(
            "{bits_per_sample}-bit sensor samples are not supported"
        )));
    }

    let compression = raw.u32(TAG_COMPRESSION).unwrap_or(COMPRESSION_NONE);
    let layout = segment_layout(data, raw, width, height, bits_per_sample, compression)?;
    let samples = decode_samples(data, &layout, &area)?;

    let cfa = if linear {
        CfaPattern::RGGB
    } else {
        read_cfa(raw)?
    };
    let white_level = raw
        .f64(TAG_WHITE_LEVEL)
        .map(|value| value as f32)
        .unwrap_or_else(|| ((1u32 << bits_per_sample) - 1) as f32);
    let mut black_level = read_black_level(raw);
    // The photosites behind the mask say what black is on this sensor. The declared level is
    // authoritative and is never replaced; the measurement is used only when there is none.
    let masked_black_level = if linear {
        None
    } else {
        measure_masked_black_level(
            data,
            &layout,
            &samples,
            &area,
            &read_masked_areas(raw, width, height),
            white_level,
        )
    };
    if raw.f64_vec(TAG_BLACK_LEVEL).is_none() {
        if let Some(measured) = masked_black_level {
            black_level = measured;
        }
    }
    let active_area = read_active_area(raw, width, height);
    let default_crop = read_default_crop(raw);

    let color_matrix = read_color_matrix(&file);
    let as_shot_neutral = read_as_shot_neutral(&file);
    let camera_to_srgb = color_matrix.and_then(|matrix| camera_to_srgb(&matrix));

    validate_levels(&black_level, white_level)?;

    Ok(SensorImage {
        data: samples,
        width: area.width,
        height: area.height,
        cfa,
        black_level,
        masked_black_level,
        white_level,
        bits_per_sample,
        // These describe the whole sensor, so for a window they would point at
        // the wrong place. The window *is* the picture it carries.
        active_area: if window.is_some() {
            (0, 0, area.width, area.height)
        } else {
            active_area
        },
        default_crop: if window.is_some() { None } else { default_crop },
        as_shot_neutral,
        camera_to_srgb,
        orientation: read_orientation(&file),
        metadata: read_metadata(&file),
        linear,
    })
}

/// Picks the IFD holding the full-resolution sensor data.
///
/// A DNG normally stores a small preview in IFD0 and the real sensor image in a
/// SubIFD, so the largest CFA image wins rather than the first one found.
fn choose_raw_ifd(file: &TiffFile) -> Option<usize> {
    let mut best: Option<(usize, u64)> = None;
    for (index, ifd) in file.ifds.iter().enumerate() {
        let photometric = ifd.u32(TAG_PHOTOMETRIC);
        let is_sensor = matches!(
            photometric,
            Some(PHOTOMETRIC_CFA) | Some(PHOTOMETRIC_LINEAR_RAW)
        );
        if !is_sensor {
            continue;
        }
        // NewSubFileType bit 0 marks a reduced-resolution image.
        if ifd.u32(TAG_NEW_SUBFILE_TYPE).unwrap_or(0) & 1 == 1 {
            continue;
        }
        let (Some(width), Some(height)) = (ifd.u32(TAG_IMAGE_WIDTH), ifd.u32(TAG_IMAGE_LENGTH))
        else {
            continue;
        };
        let area = u64::from(width) * u64::from(height);
        if best.is_none_or(|(_, best_area)| area > best_area) {
            best = Some((index, area));
        }
    }
    best.map(|(index, _)| index)
}

/// Reads the CFA layout, rejecting anything that is not a two-by-two Bayer
/// block rather than guessing at it.
fn read_cfa(ifd: &Ifd) -> Result<CfaPattern, RawError> {
    let dimensions = ifd
        .u32_vec(TAG_CFA_REPEAT_DIM)
        .unwrap_or_else(|| vec![2, 2]);
    if dimensions.len() < 2 || dimensions[0] != 2 || dimensions[1] != 2 {
        return Err(RawError::Unsupported(format!(
            "a {}x{} colour filter array is not a Bayer pattern; X-Trans and other layouts are not supported",
            dimensions.first().copied().unwrap_or(0),
            dimensions.get(1).copied().unwrap_or(0)
        )));
    }
    let codes = ifd
        .u32_vec(TAG_CFA_PATTERN)
        .ok_or_else(|| RawError::Malformed("the sensor image declares no CFA pattern".into()))?;
    CfaPattern::from_codes(&codes).ok_or_else(|| {
        RawError::Unsupported(
            "the colour filter array uses filters other than red, green, and blue".into(),
        )
    })
}

/// Black levels, which may be one value or one per CFA position.
fn read_black_level(ifd: &Ifd) -> [f32; 4] {
    let Some(values) = ifd.f64_vec(TAG_BLACK_LEVEL) else {
        return [0.0; 4];
    };
    let repeat = ifd
        .u32_vec(TAG_BLACK_LEVEL_REPEAT_DIM)
        .unwrap_or_else(|| vec![1, 1]);
    let (rows, columns) = (
        repeat.first().copied().unwrap_or(1).max(1),
        repeat.get(1).copied().unwrap_or(1).max(1),
    );
    let mut levels = [0.0f32; 4];
    for y in 0..2u32 {
        for x in 0..2u32 {
            // A 1x1 repeat means one level for every position; a 2x2 repeat
            // gives each position its own.
            let index = ((y % rows) * columns + (x % columns)) as usize;
            let value = values.get(index).or_else(|| values.first()).copied();
            levels[CfaPattern::cell_index(x, y)] = value.unwrap_or(0.0) as f32;
        }
    }
    levels
}

/// The most masked areas read from a file; real sensors declare a few strips.
const MAX_MASKED_AREAS: usize = 16;

/// The rectangles of masked photosites the file declares, as sensor rectangles.
///
/// `MaskedAreas` is a list of `(top, left, bottom, right)`, like `ActiveArea`. An area that is
/// empty, inverted or off the sensor is dropped rather than believed, and so is a trailing
/// partial entry.
fn read_masked_areas(ifd: &Ifd, width: u32, height: u32) -> Vec<Rect> {
    let Some(values) = ifd.u32_vec(TAG_MASKED_AREAS) else {
        return Vec::new();
    };
    values
        .chunks_exact(4)
        .take(MAX_MASKED_AREAS)
        .filter_map(|area| {
            let (top, left, bottom, right) = (area[0], area[1], area[2], area[3]);
            (left < right && top < bottom && right <= width && bottom <= height).then(|| Rect {
                x: left,
                y: top,
                width: right - left,
                height: bottom - top,
            })
        })
        .collect()
}

/// A running mean of sample values for each CFA position.
#[derive(Default)]
struct BlackAccumulator {
    sums: [u64; 4],
    counts: [u64; 4],
}

impl BlackAccumulator {
    fn add(&mut self, x: u32, y: u32, value: u16) {
        let cell = CfaPattern::cell_index(x, y);
        self.sums[cell] += u64::from(value);
        self.counts[cell] += 1;
    }

    /// The mean at each position; `None` unless every position has at least one sample, since
    /// a level for three of four positions is not a black level for the sensor.
    fn means(&self) -> Option<[f32; 4]> {
        if self.counts.contains(&0) {
            return None;
        }
        let mut means = [0.0f32; 4];
        for (mean, (sum, count)) in means.iter_mut().zip(self.sums.iter().zip(&self.counts)) {
            *mean = (*sum as f64 / *count as f64) as f32;
        }
        Some(means)
    }
}

/// The black level of a sensor, measured on its masked photosites.
///
/// `samples` holds the part of the sensor given by `held`; an area inside it is read from there,
/// and any other is decoded from the file, so a window decode measures exactly what a whole
/// decode does and the two develop identically. Nothing here fails the decode: whatever cannot
/// be read or believed is simply not measured. A measurement is believed only when it covers
/// every CFA position and every mean lies below the white level.
fn measure_masked_black_level(
    data: &[u8],
    layout: &SegmentLayout,
    samples: &[u16],
    held: &Rect,
    areas: &[Rect],
    white_level: f32,
) -> Option<[f32; 4]> {
    let mut accumulator = BlackAccumulator::default();
    for area in areas {
        if area.pixels() > raw_max_pixels() {
            return None;
        }
        let inside = area.x >= held.x
            && area.y >= held.y
            && u64::from(area.x) + u64::from(area.width)
                <= u64::from(held.x) + u64::from(held.width)
            && u64::from(area.y) + u64::from(area.height)
                <= u64::from(held.y) + u64::from(held.height);
        if inside {
            for y in area.y..area.y + area.height {
                for x in area.x..area.x + area.width {
                    let index = (y - held.y) as usize * held.width as usize + (x - held.x) as usize;
                    if let Some(value) = samples.get(index) {
                        accumulator.add(x, y, *value);
                    }
                }
            }
        } else {
            visit_segments(data, layout, area, None, |segment| {
                let left = segment.x.max(area.x);
                let right = (segment.x + segment.width).min(area.x + area.width);
                let top = segment.y.max(area.y);
                let bottom = (segment.y + segment.height).min(area.y + area.height);
                for y in top..bottom {
                    for x in left..right {
                        let index = (y - segment.y) as usize * segment.width as usize
                            + (x - segment.x) as usize;
                        if let Some(value) = segment.samples.get(index) {
                            accumulator.add(x, y, *value);
                        }
                    }
                }
            })
            .ok()?;
        }
    }
    let means = accumulator.means()?;
    means
        .iter()
        .all(|level| level.is_finite() && *level >= 0.0 && *level < white_level)
        .then_some(means)
}

fn read_active_area(ifd: &Ifd, width: u32, height: u32) -> (u32, u32, u32, u32) {
    // ActiveArea is (top, left, bottom, right).
    let full = (0, 0, width, height);
    let Some(values) = ifd.u32_vec(TAG_ACTIVE_AREA) else {
        return full;
    };
    if values.len() < 4 {
        return full;
    }
    let (top, left, bottom, right) = (values[0], values[1], values[2], values[3]);
    if left >= right || top >= bottom || right > width || bottom > height {
        return full;
    }
    (left, top, right - left, bottom - top)
}

fn read_default_crop(ifd: &Ifd) -> Option<(u32, u32, u32, u32)> {
    let origin = ifd.f64_vec(TAG_DEFAULT_CROP_ORIGIN)?;
    let size = ifd.f64_vec(TAG_DEFAULT_CROP_SIZE)?;
    if origin.len() < 2 || size.len() < 2 {
        return None;
    }
    let values = [origin[0], origin[1], size[0], size[1]];
    if values
        .iter()
        .any(|value| !value.is_finite() || *value < 0.0)
    {
        return None;
    }
    Some((
        values[0] as u32,
        values[1] as u32,
        values[2] as u32,
        values[3] as u32,
    ))
}

fn read_orientation(file: &TiffFile) -> u16 {
    file.ifds
        .iter()
        .find_map(|ifd| ifd.u32(TAG_ORIENTATION))
        .filter(|value| (1..=8).contains(value))
        .unwrap_or(1) as u16
}

fn read_as_shot_neutral(file: &TiffFile) -> Option<[f32; 3]> {
    let values = file
        .ifds
        .iter()
        .find_map(|ifd| ifd.f64_vec(TAG_AS_SHOT_NEUTRAL))?;
    if values.len() < 3 {
        return None;
    }
    let neutral = [values[0] as f32, values[1] as f32, values[2] as f32];
    // A neutral of zero would divide the image by nothing.
    neutral
        .iter()
        .all(|value| value.is_finite() && *value > 0.0)
        .then_some(neutral)
}

/// The XYZ-to-camera matrix, preferring the D65 illuminant when both are given.
fn read_color_matrix(file: &TiffFile) -> Option<[[f32; 3]; 3]> {
    let read = |tag: u16| -> Option<[[f32; 3]; 3]> {
        let values = file.ifds.iter().find_map(|ifd| ifd.f64_vec(tag))?;
        if values.len() < 9 || values.iter().any(|value| !value.is_finite()) {
            return None;
        }
        let mut matrix = [[0.0f32; 3]; 3];
        for row in 0..3 {
            for column in 0..3 {
                matrix[row][column] = values[row * 3 + column] as f32;
            }
        }
        Some(matrix)
    };
    // ColorMatrix2 is the second illuminant, conventionally the daylight one.
    read(TAG_COLOR_MATRIX_2).or_else(|| read(TAG_COLOR_MATRIX_1))
}

/// sRGB primaries to CIE XYZ under D65, from IEC 61966-2-1.
const SRGB_TO_XYZ: [[f32; 3]; 3] = [
    [0.412_456_4, 0.357_576_1, 0.180_437_5],
    [0.212_672_9, 0.715_152_2, 0.072_175_0],
    [0.019_333_9, 0.119_192, 0.950_304_1],
];

fn multiply(left: &[[f32; 3]; 3], right: &[[f32; 3]; 3]) -> [[f32; 3]; 3] {
    let mut out = [[0.0f32; 3]; 3];
    for row in 0..3 {
        for column in 0..3 {
            out[row][column] = (0..3).map(|k| left[row][k] * right[k][column]).sum();
        }
    }
    out
}

fn invert(matrix: &[[f32; 3]; 3]) -> Option<[[f32; 3]; 3]> {
    let m = matrix;
    let determinant = m[0][0] * (m[1][1] * m[2][2] - m[1][2] * m[2][1])
        - m[0][1] * (m[1][0] * m[2][2] - m[1][2] * m[2][0])
        + m[0][2] * (m[1][0] * m[2][1] - m[1][1] * m[2][0]);
    if !determinant.is_finite() || determinant.abs() < 1e-9 {
        return None;
    }
    let inverse_determinant = 1.0 / determinant;
    let mut out = [[0.0f32; 3]; 3];
    out[0][0] = (m[1][1] * m[2][2] - m[1][2] * m[2][1]) * inverse_determinant;
    out[0][1] = (m[0][2] * m[2][1] - m[0][1] * m[2][2]) * inverse_determinant;
    out[0][2] = (m[0][1] * m[1][2] - m[0][2] * m[1][1]) * inverse_determinant;
    out[1][0] = (m[1][2] * m[2][0] - m[1][0] * m[2][2]) * inverse_determinant;
    out[1][1] = (m[0][0] * m[2][2] - m[0][2] * m[2][0]) * inverse_determinant;
    out[1][2] = (m[0][2] * m[1][0] - m[0][0] * m[1][2]) * inverse_determinant;
    out[2][0] = (m[1][0] * m[2][1] - m[1][1] * m[2][0]) * inverse_determinant;
    out[2][1] = (m[0][1] * m[2][0] - m[0][0] * m[2][1]) * inverse_determinant;
    out[2][2] = (m[0][0] * m[1][1] - m[0][1] * m[1][0]) * inverse_determinant;
    Some(out)
}

/// Builds camera RGB to linear sRGB from the DNG XYZ-to-camera matrix.
///
/// The rows are normalised so that a camera-neutral signal maps to sRGB white,
/// which is the convention the DNG specification describes and the reason a
/// grey card comes out grey rather than tinted.
pub fn camera_to_srgb(xyz_to_camera: &[[f32; 3]; 3]) -> Option<[[f32; 3]; 3]> {
    let mut camera_from_srgb = multiply(xyz_to_camera, &SRGB_TO_XYZ);
    for row in camera_from_srgb.iter_mut() {
        let sum: f32 = row.iter().sum();
        if !sum.is_finite() || sum.abs() < 1e-9 {
            return None;
        }
        for value in row.iter_mut() {
            *value /= sum;
        }
    }
    invert(&camera_from_srgb)
}

fn validate_levels(black: &[f32; 4], white: f32) -> Result<(), RawError> {
    if !white.is_finite() || white <= 0.0 {
        return Err(RawError::InvalidMetadata(
            "the white level is not a positive finite value".into(),
        ));
    }
    for level in black {
        if !level.is_finite() || *level < 0.0 {
            return Err(RawError::InvalidMetadata(
                "a black level is negative or not finite".into(),
            ));
        }
        if *level >= white {
            return Err(RawError::InvalidMetadata(
                "a black level is at or above the white level, leaving no range to develop".into(),
            ));
        }
    }
    Ok(())
}

fn read_metadata(file: &TiffFile) -> RawCaptureMetadata {
    let find_ascii = |tag: u16| file.ifds.iter().find_map(|ifd| ifd.ascii(tag));
    let find_f64 = |tag: u16| file.ifds.iter().find_map(|ifd| ifd.f64(tag));
    let shutter = find_f64(TAG_EXPOSURE_TIME).map(|value| value as f32);
    RawCaptureMetadata {
        manufacturer: find_ascii(TAG_MAKE),
        model: find_ascii(TAG_MODEL),
        lens: find_ascii(TAG_LENS_MODEL),
        focal_length_mm: find_f64(TAG_FOCAL_LENGTH).map(|value| value as f32),
        aperture: find_f64(TAG_F_NUMBER).map(|value| value as f32),
        shutter_speed_seconds: shutter,
        iso: file
            .ifds
            .iter()
            .find_map(|ifd| ifd.u32(TAG_ISO))
            .filter(|value| *value > 0),
        capture_time: find_ascii(TAG_DATE_TIME_ORIGINAL),
        orientation: Some(read_orientation(file)),
        // Exposure bias is signed, so it is read separately from the
        // nonnegative fields and only kept when it is sane.
        exposure_compensation: file
            .ifds
            .iter()
            .find_map(|ifd| ifd.get(TAG_EXPOSURE_BIAS))
            .and_then(|entry| entry.first_f64())
            .filter(|value| value.is_finite() && value.abs() <= 32.0)
            .map(|value| value.abs() as f32),
        white_balance_multipliers: read_as_shot_neutral(file).map(|neutral| {
            // The neutral is what the camera must divide by; the multipliers a
            // developer applies are its reciprocal.
            [1.0 / neutral[0], 1.0 / neutral[1], 1.0 / neutral[2]]
        }),
    }
}

/// Bytes held per photosite of the segment being decoded: the decoded samples
/// and, for lossless JPEG, the decoder's own frame of the same samples.
pub const SEGMENT_TRANSIENT_BYTES_PER_PHOTOSITE: u64 = 4;

/// How the pixel payload of a sensor image is divided up.
///
/// Every strip or tile is compressed on its own, which is the property that makes
/// reading part of the sensor possible at all: a segment that lies outside the
/// window wanted is never decoded.
#[derive(Debug, Clone)]
pub struct SegmentLayout {
    pub width: u32,
    pub height: u32,
    pub bits: u8,
    pub compression: u32,
    pub tiled: bool,
    pub segment_width: u32,
    pub segment_height: u32,
    pub across: u32,
    offsets: Vec<u32>,
    counts: Vec<u32>,
    little_endian: bool,
}

impl SegmentLayout {
    pub fn segment_count(&self) -> usize {
        self.offsets.len()
    }

    /// Samples in one segment, the most that is ever decoded at once.
    pub fn segment_pixels(&self) -> u64 {
        u64::from(self.segment_width) * u64::from(self.segment_height)
    }
}

/// One decoded strip or tile, clipped to the sensor.
struct Segment {
    x: u32,
    y: u32,
    width: u32,
    height: u32,
    /// `width * height` samples, row-major.
    samples: Vec<u16>,
}

fn segment_layout(
    data: &[u8],
    ifd: &Ifd,
    width: u32,
    height: u32,
    bits: u8,
    compression: u32,
) -> Result<SegmentLayout, RawError> {
    let tiled = ifd.get(TAG_TILE_OFFSETS).is_some();
    let (offsets, counts) = if tiled {
        (
            ifd.u32_vec(TAG_TILE_OFFSETS).unwrap_or_default(),
            ifd.u32_vec(TAG_TILE_BYTE_COUNTS).unwrap_or_default(),
        )
    } else {
        (
            ifd.u32_vec(TAG_STRIP_OFFSETS).unwrap_or_default(),
            ifd.u32_vec(TAG_STRIP_BYTE_COUNTS).unwrap_or_default(),
        )
    };
    if offsets.is_empty() || offsets.len() != counts.len() {
        return Err(RawError::Malformed(
            "the sensor image has no readable strip or tile table".into(),
        ));
    }
    if offsets.len() > MAX_SEGMENTS {
        return Err(RawError::Malformed(
            "the sensor image declares an unreasonable number of strips".into(),
        ));
    }

    let (segment_width, segment_height) = if tiled {
        let tile_width = ifd.u32(TAG_TILE_WIDTH).unwrap_or(0);
        let tile_height = ifd.u32(TAG_TILE_LENGTH).unwrap_or(0);
        if tile_width == 0 || tile_height == 0 {
            return Err(RawError::Malformed("a tile has no size".into()));
        }
        (tile_width, tile_height)
    } else {
        (width, ifd.u32(TAG_ROWS_PER_STRIP).unwrap_or(height).max(1))
    };
    let across = if tiled {
        width.div_ceil(segment_width)
    } else {
        1
    };
    let down = height.div_ceil(segment_height);
    if u64::from(across) * u64::from(down) != offsets.len() as u64 {
        return Err(RawError::Malformed(
            "strip/tile dimensions and segment count disagree".into(),
        ));
    }
    // One segment is decoded whole, so its size bounds the transient. A sensor
    // stored as a single strip cannot be read in pieces, so the strip is the
    // sensor. What is bounded is the *bytes* held at once, not the pixel count of
    // a developed image: samples are two bytes each, so a segment can hold far
    // more photosites than a working image could.
    let transient = (u64::from(segment_width) * u64::from(segment_height))
        .saturating_mul(SEGMENT_TRANSIENT_BYTES_PER_PHOTOSITE);
    if transient > crate::resources::max_job_bytes() {
        return Err(RawError::FileTooLarge);
    }
    for (tag, name) in [
        (317, "Predictor"),
        (339, "SampleFormat"),
        (284, "PlanarConfiguration"),
    ] {
        if ifd.u32(tag).unwrap_or(1) != 1 {
            return Err(RawError::Unsupported(format!(
                "non-default {name} is not implemented"
            )));
        }
    }
    if ifd.get(50712).is_some() {
        return Err(RawError::Unsupported(
            "DNG LinearizationTable is not implemented".into(),
        ));
    }
    Ok(SegmentLayout {
        width,
        height,
        bits,
        compression,
        tiled,
        segment_width,
        segment_height,
        across,
        offsets,
        counts,
        little_endian: data.starts_with(b"II"),
    })
}

/// Decodes segment `index`, or `None` for one that lies wholly below the sensor.
fn decode_segment(
    data: &[u8],
    layout: &SegmentLayout,
    index: usize,
) -> Result<Option<Segment>, RawError> {
    let start = layout.offsets[index] as usize;
    let length = layout.counts[index] as usize;
    let end = start
        .checked_add(length)
        .ok_or_else(|| RawError::Malformed("a strip offset overflows".into()))?;
    let payload = data
        .get(start..end)
        .ok_or_else(|| RawError::Malformed("a strip lies outside the file".into()))?;

    let (origin_x, origin_y) = if layout.tiled {
        let column = (index as u32) % layout.across;
        let row = (index as u32) / layout.across;
        (column * layout.segment_width, row * layout.segment_height)
    } else {
        (0, index as u32 * layout.segment_height)
    };
    if origin_y >= layout.height {
        return Ok(None);
    }
    let width = layout.segment_width.min(layout.width - origin_x);
    let height = layout.segment_height.min(layout.height - origin_y);
    let mut samples = Vec::new();
    samples
        .try_reserve_exact(width as usize * height as usize)
        .map_err(|_| RawError::FileTooLarge)?;
    samples.resize(width as usize * height as usize, 0u16);

    match layout.compression {
        COMPRESSION_NONE => {
            let rows_in_payload = if layout.tiled {
                layout.segment_height
            } else {
                height
            };
            let needed = (u64::from(layout.segment_width) * u64::from(layout.bits))
                .div_ceil(8)
                .checked_mul(u64::from(rows_in_payload))
                .ok_or(RawError::FileTooLarge)?;
            if (payload.len() as u64) < needed {
                return Err(RawError::Malformed(
                    "uncompressed strip/tile payload is truncated".into(),
                ));
            }
            // TIFF pads each row to a byte boundary, so rows are addressed by
            // their own stride rather than by a running bit position.
            let row_bytes = (u64::from(layout.segment_width) * u64::from(layout.bits)).div_ceil(8) as usize;
            for row in 0..height as usize {
                let Some(row_data) = payload.get(row * row_bytes..(row + 1) * row_bytes) else {
                    break;
                };
                for column in 0..width as usize {
                    samples[row * width as usize + column] =
                        if layout.bits == 16 && !layout.little_endian {
                            u16::from_be_bytes([row_data[column * 2], row_data[column * 2 + 1]])
                        } else {
                            read_packed(row_data, column, layout.bits)
                        };
                }
            }
        }
        COMPRESSION_LOSSLESS_JPEG => {
            let frame = ljpeg::decode(payload)
                .map_err(|error| RawError::Malformed(error.to_string()))?;
            if frame.width.checked_mul(frame.components) != Some(layout.segment_width as usize)
                || frame.height < height as usize
                || frame.height > layout.segment_height as usize
            {
                return Err(RawError::Malformed(
                    "lossless JPEG frame dimensions disagree with its strip/tile".into(),
                ));
            }
            let row_samples = frame.width * frame.components;
            for row in 0..height as usize {
                for column in 0..width as usize {
                    let Some(value) = frame.samples.get(row * row_samples + column) else {
                        return Ok(Some(Segment { x: origin_x, y: origin_y, width, height, samples }));
                    };
                    samples[row * width as usize + column] = *value;
                }
            }
        }
        other => {
            return Err(RawError::Unsupported(format!(
                "DNG compression {other} is not supported; this build reads uncompressed and lossless JPEG"
            )))
        }
    }
    Ok(Some(Segment {
        x: origin_x,
        y: origin_y,
        width,
        height,
        samples,
    }))
}

/// Whether a segment's rectangle overlaps the window at all.
fn overlaps(layout: &SegmentLayout, index: usize, window: &Rect) -> bool {
    let (x, y) = if layout.tiled {
        (
            u64::from((index as u32) % layout.across) * u64::from(layout.segment_width),
            u64::from((index as u32) / layout.across) * u64::from(layout.segment_height),
        )
    } else {
        (0, index as u64 * u64::from(layout.segment_height))
    };
    let (w, h) = (
        u64::from(layout.segment_width),
        u64::from(layout.segment_height),
    );
    x < window.right()
        && x + w > u64::from(window.x)
        && y < window.bottom()
        && y + h > u64::from(window.y)
}

/// Calls `visit` for every segment that overlaps `window`, decoding no other.
fn visit_segments(
    data: &[u8],
    layout: &SegmentLayout,
    window: &Rect,
    cancel: Option<&std::sync::atomic::AtomicBool>,
    mut visit: impl FnMut(&Segment),
) -> Result<(), RawError> {
    for index in 0..layout.segment_count() {
        if !overlaps(layout, index, window) {
            continue;
        }
        if cancel.is_some_and(|flag| flag.load(std::sync::atomic::Ordering::Acquire)) {
            return Err(RawError::Cancelled);
        }
        if let Some(segment) = decode_segment(data, layout, index)? {
            visit(&segment);
        }
    }
    Ok(())
}

/// Reads the sensor samples inside `window` into a buffer of the window's size.
fn decode_samples(
    data: &[u8],
    layout: &SegmentLayout,
    window: &Rect,
) -> Result<Vec<u16>, RawError> {
    let pixels = window.pixels();
    if pixels > raw_max_pixels() {
        return Err(RawError::FileTooLarge);
    }
    let mut out = Vec::new();
    out.try_reserve_exact(pixels as usize)
        .map_err(|_| RawError::FileTooLarge)?;
    out.resize(pixels as usize, 0u16);
    let stride = window.width as usize;
    visit_segments(data, layout, window, None, |segment| {
        // Copy the overlap of the segment and the window, row by row.
        let left = segment.x.max(window.x);
        let right = (segment.x + segment.width).min(window.x + window.width);
        let top = segment.y.max(window.y);
        let bottom = (segment.y + segment.height).min(window.y + window.height);
        for y in top..bottom {
            let from =
                (y - segment.y) as usize * segment.width as usize + (left - segment.x) as usize;
            let to = (y - window.y) as usize * stride + (left - window.x) as usize;
            let count = (right - left) as usize;
            out[to..to + count].copy_from_slice(&segment.samples[from..from + count]);
        }
    })?;
    Ok(out)
}

/// Reads the `index`-th sample of `bits` width, packed most significant first.
fn read_packed(row: &[u8], index: usize, bits: u8) -> u16 {
    match bits {
        8 => row.get(index).copied().map(u16::from).unwrap_or(0),
        16 => {
            let start = index * 2;
            row.get(start..start + 2)
                // DNG stores uncompressed 16-bit samples little-endian.
                .map(|pair| u16::from_le_bytes([pair[0], pair[1]]))
                .unwrap_or(0)
        }
        _ => {
            let bit_start = index * usize::from(bits);
            let mut value = 0u32;
            for offset in 0..usize::from(bits) {
                let bit = bit_start + offset;
                let byte = row.get(bit / 8).copied().unwrap_or(0);
                let taken = (byte >> (7 - (bit % 8))) & 1;
                value = (value << 1) | u32::from(taken);
            }
            value as u16
        }
    }
}

/// Builds small, valid DNG files.
///
/// This exists so the decoder can be tested against files whose every sample is
/// known in advance, without shipping a photograph anyone else owns. It writes
/// the same structures a camera writes, so a test that passes here is a test of
/// the format rather than of a private agreement between two functions.
#[doc(hidden)]
pub mod fixtures {
    use super::*;

    /// The Exif and GPS directory pointers of the first directory.
    const EXIF_POINTER: u16 = 34665;
    const GPS_POINTER: u16 = 34853;

    pub struct DngBuilder {
        pub width: u32,
        pub height: u32,
        pub bits: u8,
        pub cfa: [u8; 4],
        pub black: Vec<u32>,
        pub black_repeat: (u32, u32),
        /// Whether `BlackLevel` is written at all; a file may declare none.
        pub write_black_level: bool,
        pub white: u32,
        pub samples: Vec<u16>,
        pub compression: u32,
        pub make: String,
        pub model: String,
        pub as_shot_neutral: Option<[f64; 3]>,
        pub color_matrix: Option<[f64; 9]>,
        pub orientation: u16,
        pub iso: Option<u32>,
        pub photometric: u32,
        pub tile_size: Option<(u32, u32)>,
        /// Extra entries of the first directory (tag, field type, bytes): XMP, IPTC, the
        /// copyright, anything the sensor reader does not use.
        pub extra_entries: Vec<(u16, u16, Vec<u8>)>,
        /// Entries of an Exif directory written after the pixels; none means no directory.
        pub exif_entries: Vec<(u16, u16, Vec<u8>)>,
        /// Entries of a GPS directory written after the pixels; none means no directory.
        pub gps_entries: Vec<(u16, u16, Vec<u8>)>,
    }

    /// Bytes of one directory whose first byte will be at `base`, with the values it points at
    /// laid down after it. Entries are written in tag order, as TIFF requires.
    fn trailing_directory(entries: &[(u16, u16, Vec<u8>)], base: usize) -> Vec<u8> {
        let mut sorted: Vec<&(u16, u16, Vec<u8>)> = entries.iter().collect();
        sorted.sort_by_key(|(tag, _, _)| *tag);
        let directory_len = 2 + sorted.len() * 12 + 4;
        let mut directory = Vec::new();
        directory.extend((sorted.len() as u16).to_le_bytes());
        let mut values: Vec<u8> = Vec::new();
        for (tag, kind, payload) in sorted {
            let width = match kind {
                3 | 8 => 2,
                4 | 9 | 11 | 13 => 4,
                5 | 10 | 12 => 8,
                _ => 1,
            };
            directory.extend(tag.to_le_bytes());
            directory.extend(kind.to_le_bytes());
            directory.extend(((payload.len() / width) as u32).to_le_bytes());
            if payload.len() <= 4 {
                let mut inline = payload.clone();
                inline.resize(4, 0);
                directory.extend(inline);
            } else {
                directory.extend(((base + directory_len + values.len()) as u32).to_le_bytes());
                values.extend(payload.iter().copied());
                if values.len() % 2 == 1 {
                    values.push(0);
                }
            }
        }
        directory.extend(0u32.to_le_bytes());
        directory.extend(values);
        directory
    }

    impl DngBuilder {
        pub fn new(width: u32, height: u32, samples: Vec<u16>) -> Self {
            Self {
                width,
                height,
                bits: 16,
                cfa: [0, 1, 1, 2],
                black: vec![0],
                black_repeat: (1, 1),
                write_black_level: true,
                white: 65535,
                samples,
                compression: COMPRESSION_NONE,
                make: "PhotoForge".into(),
                model: "Synthetic Sensor".into(),
                as_shot_neutral: None,
                color_matrix: None,
                orientation: 1,
                iso: None,
                photometric: PHOTOMETRIC_CFA,
                tile_size: None,
                extra_entries: Vec::new(),
                exif_entries: Vec::new(),
                gps_entries: Vec::new(),
            }
        }

        /// Adds an entry to the first directory.
        pub fn entry(mut self, tag: u16, field_type: u16, bytes: Vec<u8>) -> Self {
            self.extra_entries.push((tag, field_type, bytes));
            self
        }

        /// Adds an entry to the Exif directory (which is written when it has any).
        pub fn exif_entry(mut self, tag: u16, field_type: u16, bytes: Vec<u8>) -> Self {
            self.exif_entries.push((tag, field_type, bytes));
            self
        }

        /// Adds an entry to the GPS directory (which is written when it has any).
        pub fn gps_entry(mut self, tag: u16, field_type: u16, bytes: Vec<u8>) -> Self {
            self.gps_entries.push((tag, field_type, bytes));
            self
        }

        pub fn bits(mut self, bits: u8) -> Self {
            self.bits = bits;
            self
        }

        pub fn cfa(mut self, cfa: [u8; 4]) -> Self {
            self.cfa = cfa;
            self
        }

        pub fn levels(mut self, black: Vec<u32>, repeat: (u32, u32), white: u32) -> Self {
            self.black = black;
            self.black_repeat = repeat;
            self.white = white;
            self
        }

        /// A file that declares no `BlackLevel` (and no repeat dimensions for it).
        pub fn without_black_level(mut self) -> Self {
            self.write_black_level = false;
            self
        }

        pub fn neutral(mut self, neutral: [f64; 3]) -> Self {
            self.as_shot_neutral = Some(neutral);
            self
        }

        pub fn matrix(mut self, matrix: [f64; 9]) -> Self {
            self.color_matrix = Some(matrix);
            self
        }

        pub fn iso(mut self, iso: u32) -> Self {
            self.iso = Some(iso);
            self
        }

        pub fn orientation(mut self, orientation: u16) -> Self {
            self.orientation = orientation;
            self
        }

        /// Packs the samples the way an uncompressed DNG of this depth stores
        /// them: most significant bit first, each row padded to a byte.
        fn packed_rows(&self) -> Vec<u8> {
            let row_bytes = (self.width as usize * usize::from(self.bits)).div_ceil(8);
            let mut out = vec![0u8; row_bytes * self.height as usize];
            for y in 0..self.height as usize {
                let base = y * row_bytes;
                for x in 0..self.width as usize {
                    let value = self.samples[y * self.width as usize + x];
                    match self.bits {
                        8 => out[base + x] = value as u8,
                        16 => {
                            out[base + x * 2..base + x * 2 + 2]
                                .copy_from_slice(&value.to_le_bytes());
                        }
                        bits => {
                            let bit_start = x * usize::from(bits);
                            for offset in 0..usize::from(bits) {
                                let bit = (value >> (usize::from(bits) - 1 - offset)) & 1;
                                if bit == 1 {
                                    let position = bit_start + offset;
                                    out[base + position / 8] |= 1 << (7 - (position % 8));
                                }
                            }
                        }
                    }
                }
            }
            out
        }

        pub fn build(&self) -> Vec<u8> {
            let mut entries: Vec<(u16, u16, Vec<u8>)> = Vec::new();
            let push_short = |entries: &mut Vec<(u16, u16, Vec<u8>)>, tag: u16, value: u16| {
                entries.push((tag, 3, value.to_le_bytes().to_vec()));
            };
            let push_long = |entries: &mut Vec<(u16, u16, Vec<u8>)>, tag: u16, value: u32| {
                entries.push((tag, 4, value.to_le_bytes().to_vec()));
            };

            push_long(&mut entries, TAG_IMAGE_WIDTH, self.width);
            push_long(&mut entries, TAG_IMAGE_LENGTH, self.height);
            push_short(&mut entries, TAG_BITS_PER_SAMPLE, u16::from(self.bits));
            push_short(&mut entries, TAG_COMPRESSION, self.compression as u16);
            push_short(&mut entries, TAG_PHOTOMETRIC, self.photometric as u16);
            push_short(&mut entries, TAG_SAMPLES_PER_PIXEL, 1);
            push_short(&mut entries, TAG_ORIENTATION, self.orientation);
            push_long(&mut entries, TAG_ROWS_PER_STRIP, self.height);
            entries.push((TAG_MAKE, 2, format!("{}\0", self.make).into_bytes()));
            entries.push((TAG_MODEL, 2, format!("{}\0", self.model).into_bytes()));
            entries.push((TAG_DNG_VERSION, 1, vec![1, 4, 0, 0]));
            if self.photometric == PHOTOMETRIC_CFA {
                entries.push((TAG_CFA_REPEAT_DIM, 3, vec![2, 0, 2, 0]));
                entries.push((TAG_CFA_PATTERN, 1, self.cfa.to_vec()));
            }
            if self.write_black_level {
                entries.push((
                    TAG_BLACK_LEVEL_REPEAT_DIM,
                    3,
                    [
                        (self.black_repeat.0 as u16).to_le_bytes(),
                        (self.black_repeat.1 as u16).to_le_bytes(),
                    ]
                    .concat(),
                ));
                entries.push((
                    TAG_BLACK_LEVEL,
                    4,
                    self.black
                        .iter()
                        .flat_map(|value| value.to_le_bytes())
                        .collect(),
                ));
            }
            push_long(&mut entries, TAG_WHITE_LEVEL, self.white);
            if let Some(iso) = self.iso {
                push_short(&mut entries, TAG_ISO, iso as u16);
            }
            if let Some(neutral) = self.as_shot_neutral {
                entries.push((
                    TAG_AS_SHOT_NEUTRAL,
                    5,
                    neutral
                        .iter()
                        .flat_map(|value| {
                            let numerator = (value * 1_000_000.0).round() as u32;
                            [numerator.to_le_bytes(), 1_000_000u32.to_le_bytes()].concat()
                        })
                        .collect(),
                ));
            }
            if let Some(matrix) = self.color_matrix {
                entries.push((
                    TAG_COLOR_MATRIX_1,
                    10,
                    matrix
                        .iter()
                        .flat_map(|value| {
                            let numerator = (value * 1_000_000.0).round() as i32;
                            [numerator.to_le_bytes(), 1_000_000i32.to_le_bytes()].concat()
                        })
                        .collect(),
                ));
            }

            entries.extend(self.extra_entries.iter().cloned());
            // Placeholders: the offsets are known once the first directory's size is.
            if !self.exif_entries.is_empty() {
                push_long(&mut entries, EXIF_POINTER, 0);
            }
            if !self.gps_entries.is_empty() {
                push_long(&mut entries, GPS_POINTER, 0);
            }

            let mut tile_offsets = Vec::new();
            let pixels = if let Some((tw, th)) = self.tile_size {
                assert_eq!(
                    self.bits, 16,
                    "the synthetic tile writer supports 16-bit only"
                );
                assert!(tw > 0 && th > 0);
                let mut payload = Vec::new();
                let mut counts = Vec::new();
                for ty in 0..self.height.div_ceil(th) {
                    for tx in 0..self.width.div_ceil(tw) {
                        tile_offsets.push(payload.len() as u32);
                        for y in 0..th {
                            for x in 0..tw {
                                let (sx, sy) = (tx * tw + x, ty * th + y);
                                let value = if sx < self.width && sy < self.height {
                                    self.samples[(sy * self.width + sx) as usize]
                                } else {
                                    0
                                };
                                payload.extend_from_slice(&value.to_le_bytes());
                            }
                        }
                        counts.push(tw * th * 2);
                    }
                }
                entries.retain(|(tag, _, _)| *tag != TAG_ROWS_PER_STRIP);
                push_long(&mut entries, TAG_TILE_WIDTH, tw);
                push_long(&mut entries, TAG_TILE_LENGTH, th);
                entries.push((TAG_TILE_OFFSETS, 4, vec![0; tile_offsets.len() * 4]));
                entries.push((
                    TAG_TILE_BYTE_COUNTS,
                    4,
                    counts.iter().flat_map(|n| n.to_le_bytes()).collect(),
                ));
                payload
            } else {
                self.packed_rows()
            };
            // Two placeholder entries reserve the strip table; their values are
            // filled in once the directory size is known.
            if self.tile_size.is_none() {
                push_long(&mut entries, TAG_STRIP_OFFSETS, 0);
                push_long(&mut entries, TAG_STRIP_BYTE_COUNTS, pixels.len() as u32);
            }
            entries.sort_by_key(|(tag, _, _)| *tag);

            let count = entries.len();
            let directory_bytes = 2 + count * 12 + 4;
            let overflow_base = 8 + directory_bytes;
            let mut overflow: Vec<u8> = Vec::new();
            let mut directory = Vec::new();
            directory.extend((count as u16).to_le_bytes());

            // The pixel payload follows every inline and overflow value.
            let mut overflow_size = 0usize;
            for (_, field_type, payload) in &entries {
                if payload.len() > 4 {
                    let _ = field_type;
                    overflow_size += payload.len();
                }
            }
            let pixel_offset = overflow_base + overflow_size;
            // The Exif and GPS directories follow the pixels.
            let exif_at = pixel_offset + pixels.len();
            let exif_bytes = if self.exif_entries.is_empty() {
                Vec::new()
            } else {
                trailing_directory(&self.exif_entries, exif_at)
            };
            let gps_at = exif_at + exif_bytes.len();
            let gps_bytes = if self.gps_entries.is_empty() {
                Vec::new()
            } else {
                trailing_directory(&self.gps_entries, gps_at)
            };

            for (tag, field_type, payload) in &entries {
                directory.extend(tag.to_le_bytes());
                directory.extend(field_type.to_le_bytes());
                let width = match field_type {
                    1 | 2 => 1,
                    3 => 2,
                    4 => 4,
                    5 | 10 => 8,
                    _ => 1,
                };
                directory.extend(((payload.len() / width) as u32).to_le_bytes());
                let payload = if *tag == TAG_STRIP_OFFSETS {
                    (pixel_offset as u32).to_le_bytes().to_vec()
                } else if *tag == EXIF_POINTER && !self.exif_entries.is_empty() {
                    (exif_at as u32).to_le_bytes().to_vec()
                } else if *tag == GPS_POINTER && !self.gps_entries.is_empty() {
                    (gps_at as u32).to_le_bytes().to_vec()
                } else if *tag == TAG_TILE_OFFSETS {
                    tile_offsets
                        .iter()
                        .flat_map(|offset| (pixel_offset as u32 + offset).to_le_bytes())
                        .collect()
                } else {
                    payload.clone()
                };
                if payload.len() <= 4 {
                    let mut inline = payload.clone();
                    inline.resize(4, 0);
                    directory.extend(inline);
                } else {
                    directory.extend(((overflow_base + overflow.len()) as u32).to_le_bytes());
                    overflow.extend(payload.iter().copied());
                }
            }
            directory.extend(0u32.to_le_bytes());

            let mut out = vec![b'I', b'I', 42, 0, 8, 0, 0, 0];
            out.extend(directory);
            out.extend(overflow);
            out.extend(pixels);
            out.extend(exif_bytes);
            out.extend(gps_bytes);
            out
        }
    }
}

#[cfg(test)]
mod tests {
    use super::fixtures::DngBuilder;
    use super::*;

    /// A gradient with a different value at every photosite, so a transposed or
    /// offset read shows up immediately.
    fn ramp(width: u32, height: u32) -> Vec<u16> {
        (0..width * height)
            .map(|index| (index % 4096) as u16 + 100)
            .collect()
    }

    #[test]
    fn decodes_an_uncompressed_sixteen_bit_dng_sample_for_sample() {
        let samples = ramp(8, 6);
        let data = DngBuilder::new(8, 6, samples.clone()).build();
        let sensor = decode(&data).unwrap();
        assert_eq!(sensor.width, 8);
        assert_eq!(sensor.height, 6);
        assert_eq!(sensor.bits_per_sample, 16);
        assert_eq!(sensor.data, samples, "the sensor samples were altered");
    }

    #[test]
    fn is_dng_recognises_only_a_dng() {
        let data = DngBuilder::new(4, 4, ramp(4, 4)).build();
        assert!(is_dng(&data));
        assert!(!is_dng(b"not a tiff"));
        assert!(!is_dng(&[]));
    }

    #[test]
    fn decodes_every_supported_bit_depth_without_losing_a_sample() {
        for bits in [8u8, 10, 12, 14, 16] {
            let maximum = (1u32 << bits) - 1;
            let samples: Vec<u16> = (0..24)
                .map(|index| ((index * 37) % maximum) as u16)
                .collect();
            let data = DngBuilder::new(6, 4, samples.clone())
                .bits(bits)
                .levels(vec![0], (1, 1), maximum)
                .build();
            let sensor = decode(&data).unwrap();
            assert_eq!(sensor.bits_per_sample, bits);
            assert_eq!(sensor.data, samples, "{bits}-bit samples were altered");
            assert_eq!(sensor.white_level, maximum as f32);
        }
    }

    #[test]
    fn reads_every_bayer_layout() {
        let cases = [
            ([0u8, 1, 1, 2], CfaPattern::RGGB, "RGGB"),
            ([2, 1, 1, 0], CfaPattern::BGGR, "BGGR"),
            ([1, 0, 2, 1], CfaPattern::GRBG, "GRBG"),
            ([1, 2, 0, 1], CfaPattern::GBRG, "GBRG"),
        ];
        for (codes, expected, name) in cases {
            let data = DngBuilder::new(4, 4, ramp(4, 4)).cfa(codes).build();
            let sensor = decode(&data).unwrap();
            assert_eq!(sensor.cfa, expected, "{name} was misread");
            assert_eq!(sensor.cfa.name(), name);
        }
    }

    #[test]
    fn the_cfa_reports_the_colour_at_each_position() {
        assert_eq!(CfaPattern::RGGB.color_at(0, 0), CfaColor::Red);
        assert_eq!(CfaPattern::RGGB.color_at(1, 0), CfaColor::Green);
        assert_eq!(CfaPattern::RGGB.color_at(0, 1), CfaColor::Green);
        assert_eq!(CfaPattern::RGGB.color_at(1, 1), CfaColor::Blue);
        // The pattern repeats every two photosites in both directions.
        assert_eq!(CfaPattern::RGGB.color_at(2, 2), CfaColor::Red);
        assert_eq!(CfaPattern::BGGR.color_at(0, 0), CfaColor::Blue);
    }

    #[test]
    fn a_single_black_level_applies_to_every_position() {
        let data = DngBuilder::new(4, 4, ramp(4, 4))
            .levels(vec![512], (1, 1), 16383)
            .build();
        let sensor = decode(&data).unwrap();
        assert_eq!(sensor.black_level, [512.0; 4]);
        assert_eq!(sensor.white_level, 16383.0);
    }

    #[test]
    fn a_per_position_black_level_is_kept_per_position() {
        // A 2x2 repeat gives each CFA position its own level, which is what
        // sensors with different amplifier offsets actually record.
        let data = DngBuilder::new(4, 4, ramp(4, 4))
            .levels(vec![100, 200, 300, 400], (2, 2), 16383)
            .build();
        let sensor = decode(&data).unwrap();
        assert_eq!(sensor.black_level, [100.0, 200.0, 300.0, 400.0]);
        assert_eq!(sensor.black_at(0, 0), 100.0);
        assert_eq!(sensor.black_at(1, 0), 200.0);
        assert_eq!(sensor.black_at(0, 1), 300.0);
        assert_eq!(sensor.black_at(1, 1), 400.0);
    }

    #[test]
    fn a_black_level_at_or_above_white_is_refused() {
        let data = DngBuilder::new(4, 4, ramp(4, 4))
            .levels(vec![5000], (1, 1), 5000)
            .build();
        assert!(matches!(decode(&data), Err(RawError::InvalidMetadata(_))));
    }

    #[test]
    fn reads_camera_metadata_that_is_present_and_invents_none_that_is_not() {
        let data = DngBuilder::new(4, 4, ramp(4, 4)).iso(800).build();
        let sensor = decode(&data).unwrap();
        assert_eq!(sensor.metadata.manufacturer.as_deref(), Some("PhotoForge"));
        assert_eq!(sensor.metadata.model.as_deref(), Some("Synthetic Sensor"));
        assert_eq!(sensor.metadata.iso, Some(800));
        // Nothing was written for these, so nothing may be reported.
        assert_eq!(sensor.metadata.lens, None);
        assert_eq!(sensor.metadata.aperture, None);
        assert_eq!(sensor.metadata.capture_time, None);
        sensor.metadata.validate().unwrap();
    }

    #[test]
    fn reads_the_as_shot_neutral_and_reports_its_reciprocal_as_multipliers() {
        let data = DngBuilder::new(4, 4, ramp(4, 4))
            .neutral([0.5, 1.0, 0.8])
            .build();
        let sensor = decode(&data).unwrap();
        let neutral = sensor.as_shot_neutral.unwrap();
        assert!((neutral[0] - 0.5).abs() < 1e-4);
        assert!((neutral[2] - 0.8).abs() < 1e-4);
        let multipliers = sensor.metadata.white_balance_multipliers.unwrap();
        assert!((multipliers[0] - 2.0).abs() < 1e-3);
        assert!((multipliers[1] - 1.0).abs() < 1e-3);
    }

    #[test]
    fn orientation_is_read_and_an_invalid_one_falls_back_to_upright() {
        let data = DngBuilder::new(4, 4, ramp(4, 4)).orientation(6).build();
        assert_eq!(decode(&data).unwrap().orientation, 6);
        let bad = DngBuilder::new(4, 4, ramp(4, 4)).orientation(99).build();
        assert_eq!(decode(&bad).unwrap().orientation, 1);
    }

    /// The identity XYZ-to-camera matrix means the camera *is* XYZ, so the
    /// derived transform must be the standard XYZ-to-sRGB matrix with rows
    /// normalised for white.
    #[test]
    fn derives_a_camera_to_srgb_matrix_that_maps_camera_neutral_to_neutral() {
        let matrix = [0.7, 0.2, 0.1, 0.2, 0.7, 0.1, 0.1, 0.2, 0.7];
        let data = DngBuilder::new(4, 4, ramp(4, 4)).matrix(matrix).build();
        let sensor = decode(&data).unwrap();
        let transform = sensor.camera_to_srgb.expect("a matrix was derived");
        // A neutral camera signal must come out neutral in sRGB, which is what
        // the row normalisation in the DNG specification guarantees.
        for row in transform {
            let sum: f32 = row.iter().sum();
            assert!(
                (sum - 1.0).abs() < 1e-4,
                "a neutral input would be tinted: row sums to {sum}"
            );
        }
    }

    #[test]
    fn a_singular_colour_matrix_is_rejected_rather_than_producing_infinities() {
        // Three identical rows cannot be inverted.
        let singular = [[1.0f32, 1.0, 1.0], [1.0, 1.0, 1.0], [1.0, 1.0, 1.0]];
        assert!(camera_to_srgb(&singular).is_none());
    }

    #[test]
    fn a_file_that_is_tiff_but_not_dng_is_refused_by_name() {
        // A TIFF with no DNG version tag must not be treated as a sensor image.
        let mut data = DngBuilder::new(4, 4, ramp(4, 4)).build();
        // Blank the DNG version tag value in place.
        let position = data
            .windows(2)
            .position(|pair| pair == TAG_DNG_VERSION.to_le_bytes())
            .expect("version tag present");
        data[position..position + 2].copy_from_slice(&60000u16.to_le_bytes());
        assert!(matches!(decode(&data), Err(RawError::Malformed(_))));
    }

    #[test]
    fn an_unsupported_compression_is_named_rather_than_guessed_at() {
        let mut builder = DngBuilder::new(4, 4, ramp(4, 4));
        builder.compression = 34892; // lossy JPEG
        let error = decode(&builder.build()).unwrap_err();
        assert!(
            matches!(&error, RawError::Unsupported(message) if message.contains("34892")),
            "{error:?}"
        );
    }

    #[test]
    fn a_non_bayer_colour_filter_array_is_refused_honestly() {
        // Filter code 3 is cyan, which no Bayer demosaic here can handle.
        let data = DngBuilder::new(4, 4, ramp(4, 4)).cfa([0, 1, 3, 2]).build();
        let error = decode(&data).unwrap_err();
        assert!(
            matches!(&error, RawError::Unsupported(message) if message.contains("red, green, and blue")),
            "{error:?}"
        );
    }

    /// Overwrites the inline value of a LONG tag in a built file, so a test can
    /// declare a geometry no honest writer would produce.
    fn patch_long(data: &mut [u8], tag: u16, value: u32) {
        let count = u16::from_le_bytes([data[8], data[9]]) as usize;
        for index in 0..count {
            let base = 10 + index * 12;
            if u16::from_le_bytes([data[base], data[base + 1]]) == tag
                && u16::from_le_bytes([data[base + 2], data[base + 3]]) == 4
            {
                data[base + 8..base + 12].copy_from_slice(&value.to_le_bytes());
                return;
            }
        }
        panic!("tag {tag} is not an inline LONG in this fixture");
    }

    #[test]
    fn an_absurd_declared_size_is_refused_before_allocating() {
        // The file is small; only its declared dimensions are enormous, which
        // is exactly the shape of a decompression-bomb attempt.
        let mut data = DngBuilder::new(4, 4, ramp(4, 4)).build();
        patch_long(&mut data, TAG_IMAGE_WIDTH, 60_000);
        patch_long(&mut data, TAG_IMAGE_LENGTH, 60_000);
        let error = decode(&data).unwrap_err();
        assert!(
            matches!(error, RawError::FileTooLarge | RawError::InvalidMetadata(_)),
            "{error:?}"
        );
    }

    #[test]
    fn a_strip_pointing_outside_the_file_is_refused() {
        let mut data = DngBuilder::new(4, 4, ramp(4, 4)).build();
        patch_long(&mut data, TAG_STRIP_OFFSETS, 0xFFFF_0000);
        assert!(matches!(decode(&data), Err(RawError::Malformed(_))));
    }

    #[test]
    fn a_strip_declaring_more_bytes_than_the_file_holds_is_refused() {
        let mut data = DngBuilder::new(4, 4, ramp(4, 4)).build();
        patch_long(&mut data, TAG_STRIP_BYTE_COUNTS, 0x00FF_FFFF);
        assert!(matches!(decode(&data), Err(RawError::Malformed(_))));
    }

    #[test]
    fn a_truncated_file_fails_without_panicking() {
        let data = DngBuilder::new(8, 8, ramp(8, 8)).build();
        for length in 0..data.len() {
            // Every prefix must yield an error, never a panic or a hang.
            let _ = decode(&data[..length]);
        }
    }

    #[test]
    fn random_bytes_and_a_wrong_extension_payload_never_panic() {
        let mut state = 0x9E37_79B9_7F4A_7C15u64;
        for _ in 0..150 {
            let mut data = vec![b'I', b'I', 42, 0, 8, 0, 0, 0];
            for _ in 0..512 {
                state ^= state << 13;
                state ^= state >> 7;
                state ^= state << 17;
                data.push((state >> 32) as u8);
            }
            let _ = decode(&data);
        }
        // A JPEG that happens to be named .dng must be refused, not decoded.
        let jpeg = [0xFF, 0xD8, 0xFF, 0xE0, 0x00, 0x10, b'J', b'F', b'I', b'F'];
        assert!(decode(&jpeg).is_err());
    }

    #[test]
    fn the_active_area_defaults_to_the_whole_sensor_when_absent() {
        let data = DngBuilder::new(6, 4, ramp(6, 4)).build();
        assert_eq!(decode(&data).unwrap().active_area, (0, 0, 6, 4));
    }

    fn longs(values: &[u32]) -> Vec<u8> {
        values
            .iter()
            .flat_map(|value| value.to_le_bytes())
            .collect()
    }

    /// A 24 × 16 sensor with the active area (top, left, bottom, right) and the default crop
    /// (origin x, origin y, width, height) a test gives, either of which may be absent.
    fn picture_of(area: Option<[u32; 4]>, crop: Option<[u32; 4]>) -> Option<Rect> {
        let mut builder = DngBuilder::new(24, 16, ramp(24, 16));
        if let Some(area) = area {
            builder = builder.entry(TAG_ACTIVE_AREA, 4, longs(&area));
        }
        if let Some([x, y, width, height]) = crop {
            builder = builder
                .entry(TAG_DEFAULT_CROP_ORIGIN, 4, longs(&[x, y]))
                .entry(TAG_DEFAULT_CROP_SIZE, 4, longs(&[width, height]));
        }
        decode(&builder.build()).unwrap().picture_rect()
    }

    fn rect(x: u32, y: u32, width: u32, height: u32) -> Option<Rect> {
        Some(Rect {
            x,
            y,
            width,
            height,
        })
    }

    #[test]
    fn the_picture_is_the_default_crop_inside_the_active_area() {
        // Nothing declared: the picture is the whole sensor.
        assert_eq!(picture_of(None, None), None);
        // The active area alone: the picture is that.
        assert_eq!(picture_of(Some([2, 4, 14, 22]), None), rect(4, 2, 18, 12));
        // A default crop is relative to the active area's corner, not the sensor's.
        assert_eq!(
            picture_of(Some([2, 4, 14, 22]), Some([3, 1, 10, 8])),
            rect(7, 3, 10, 8)
        );
        // A crop with no active area is relative to the sensor's own corner.
        assert_eq!(picture_of(None, Some([2, 2, 20, 12])), rect(2, 2, 20, 12));
    }

    #[test]
    fn a_picture_that_is_the_whole_sensor_is_no_cut() {
        assert_eq!(picture_of(Some([0, 0, 16, 24]), None), None);
        assert_eq!(picture_of(None, Some([0, 0, 24, 16])), None);
        assert_eq!(
            picture_of(Some([2, 4, 14, 22]), Some([0, 0, 18, 12])),
            rect(4, 2, 18, 12)
        );
    }

    #[test]
    fn a_crop_that_does_not_lie_in_the_active_area_is_not_believed() {
        // Past the active area's right edge, empty, and starting outside: the active area is used.
        let area = Some([2, 4, 14, 22]);
        assert_eq!(picture_of(area, Some([10, 0, 12, 6])), rect(4, 2, 18, 12));
        assert_eq!(picture_of(area, Some([0, 0, 0, 6])), rect(4, 2, 18, 12));
        assert_eq!(picture_of(area, Some([30, 30, 2, 2])), rect(4, 2, 18, 12));
        // An active area that is not on the sensor is not believed either.
        assert_eq!(
            picture_of(Some([0, 0, 99, 99]), Some([2, 2, 8, 8])),
            rect(2, 2, 8, 8)
        );
    }

    /// A 24 × 16 sensor with this orientation, active area and default crop, as bytes.
    fn framed(orientation: u16, area: Option<[u32; 4]>, crop: Option<[u32; 4]>) -> Vec<u8> {
        let mut builder = DngBuilder::new(24, 16, ramp(24, 16)).orientation(orientation);
        if let Some(area) = area {
            builder = builder.entry(TAG_ACTIVE_AREA, 4, longs(&area));
        }
        if let Some([x, y, width, height]) = crop {
            builder = builder
                .entry(TAG_DEFAULT_CROP_ORIGIN, 4, longs(&[x, y]))
                .entry(TAG_DEFAULT_CROP_SIZE, 4, longs(&[width, height]));
        }
        builder.build()
    }

    #[test]
    fn the_geometry_read_from_the_header_is_the_picture_a_development_is_cut_to() {
        let whole = Rect {
            x: 0,
            y: 0,
            width: 24,
            height: 16,
        };
        for (area, crop) in [
            (None, None),
            (Some([2, 4, 14, 22]), None),
            (Some([2, 4, 14, 22]), Some([3, 1, 10, 8])),
            (None, Some([2, 2, 20, 12])),
            // A crop that is not believed, and an active area that is not on the sensor.
            (Some([2, 4, 14, 22]), Some([10, 0, 12, 6])),
            (Some([0, 0, 99, 99]), Some([2, 2, 8, 8])),
        ] {
            for orientation in [1_u16, 3, 6, 8] {
                let data = framed(orientation, area, crop);
                let sensor = decode(&data).unwrap();
                let geometry = inspect_geometry(&data).unwrap();
                assert_eq!((geometry.sensor_width, geometry.sensor_height), (24, 16));
                assert_eq!(geometry.cut(), sensor.picture_rect(), "{area:?} {crop:?}");
                assert_eq!(geometry.rect, sensor.picture_rect().unwrap_or(whole));
                assert_eq!(u16::from(geometry.orientation), sensor.orientation);
                let (width, height) = geometry.presented_size();
                let swapped = matches!(orientation, 5..=8);
                assert_eq!(
                    (width, height),
                    if swapped {
                        (geometry.rect.height, geometry.rect.width)
                    } else {
                        (geometry.rect.width, geometry.rect.height)
                    }
                );
            }
        }
    }

    #[test]
    fn a_rectangle_chosen_on_the_upright_picture_is_the_photosites_it_covers() {
        // The picture is 10 x 8 at (7, 3); for every orientation and every rectangle of the
        // upright picture, the sensor rectangle is exactly the photosites turned into it.
        for orientation in 1..=8_u16 {
            let data = framed(orientation, Some([2, 4, 14, 22]), Some([3, 1, 10, 8]));
            let geometry = inspect_geometry(&data).unwrap();
            assert_eq!(geometry.rect, rect(7, 3, 10, 8).unwrap());
            let (presented_width, presented_height) = geometry.presented_size();
            for y in 0..presented_height {
                for x in 0..presented_width {
                    for height in 1..=presented_height - y {
                        for width in 1..=presented_width - x {
                            let chosen = Rect {
                                x,
                                y,
                                width,
                                height,
                            };
                            let sensor = geometry.to_sensor(&chosen).expect("on the picture");
                            assert_eq!(sensor.pixels(), chosen.pixels());
                            assert!(
                                sensor.x >= 7
                                    && sensor.y >= 3
                                    && sensor.right() <= 17
                                    && sensor.bottom() <= 11,
                                "{orientation} {chosen:?} -> {sensor:?}"
                            );
                            for cy in y..y + height {
                                for cx in x..x + width {
                                    let (ux, uy) = crate::orientation::unplace(
                                        orientation as u8,
                                        10,
                                        8,
                                        cx,
                                        cy,
                                    );
                                    let (sx, sy) = (7 + ux, 3 + uy);
                                    assert!(
                                        sx >= sensor.x
                                            && u64::from(sx) < sensor.right()
                                            && sy >= sensor.y
                                            && u64::from(sy) < sensor.bottom(),
                                        "{orientation} {chosen:?} pixel ({cx}, {cy})"
                                    );
                                }
                            }
                            assert_eq!(geometry.from_sensor(&sensor), Some(chosen));
                        }
                    }
                }
            }
        }
    }

    #[test]
    fn a_rectangle_off_the_picture_is_not_mapped() {
        let data = framed(6, Some([2, 4, 14, 22]), Some([3, 1, 10, 8]));
        let geometry = inspect_geometry(&data).unwrap();
        // Upright, a quarter turn makes the 10 x 8 picture 8 wide and 10 tall.
        assert_eq!(geometry.presented_size(), (8, 10));
        let on = |x, y, width, height| {
            geometry.to_sensor(&Rect {
                x,
                y,
                width,
                height,
            })
        };
        assert!(on(0, 0, 8, 10).is_some());
        assert!(on(0, 0, 9, 10).is_none());
        assert!(on(0, 0, 8, 11).is_none());
        assert!(on(7, 9, 1, 1).is_some());
        assert!(on(8, 0, 1, 1).is_none());
        assert!(on(0, 0, 0, 5).is_none());
        // And a sensor rectangle that is not wholly on the picture has no place on it.
        let from = |x, y, width, height| {
            geometry.from_sensor(&Rect {
                x,
                y,
                width,
                height,
            })
        };
        assert!(from(7, 3, 10, 8).is_some());
        assert!(from(6, 3, 10, 8).is_none());
        assert!(from(7, 3, 11, 8).is_none());
        assert!(from(7, 3, 0, 8).is_none());
    }

    #[test]
    fn tiled_uncompressed_edges_and_padding_decode_sample_for_sample() {
        for (w, h) in [(8, 6), (33, 19), (32, 32)] {
            let samples = ramp(w, h);
            let mut builder = DngBuilder::new(w, h, samples.clone());
            builder.tile_size = Some((16, 16));
            let sensor = decode(&builder.build()).unwrap();
            assert_eq!(sensor.data, samples);
        }
    }

    #[test]
    fn truncated_segment_counts_fail_instead_of_manufacturing_black_pixels() {
        let mut bytes = DngBuilder::new(8, 8, ramp(8, 8)).build();
        patch_long(&mut bytes, TAG_STRIP_BYTE_COUNTS, 2);
        assert!(matches!(decode(&bytes), Err(RawError::Malformed(_))));
        let mut builder = DngBuilder::new(8, 8, ramp(8, 8));
        builder.tile_size = Some((16, 16));
        let mut bytes = builder.build();
        patch_long(&mut bytes, TAG_TILE_WIDTH, u32::MAX);
        assert!(decode(&bytes).is_err());
    }

    #[test]
    fn structured_malformed_offsets_dimensions_and_depths_never_panic() {
        let base = DngBuilder::new(8, 8, ramp(8, 8)).build();
        for tag in [
            TAG_IMAGE_WIDTH,
            TAG_IMAGE_LENGTH,
            TAG_STRIP_OFFSETS,
            TAG_STRIP_BYTE_COUNTS,
            TAG_ROWS_PER_STRIP,
        ] {
            for value in [0, 1, 3, 19_999, 20_001, u32::MAX] {
                let mut bytes = base.clone();
                patch_long(&mut bytes, tag, value);
                assert!(std::panic::catch_unwind(|| decode(&bytes)).is_ok());
            }
        }
        for offset in 0..base.len() {
            for byte in [0, 255] {
                let mut bytes = base.clone();
                bytes[offset] = byte;
                assert!(
                    std::panic::catch_unwind(|| decode(&bytes)).is_ok(),
                    "offset {offset}"
                );
            }
        }
    }

    /// An 8 x 8 sensor whose first `rows` rows are masked: each CFA position there holds a
    /// distinct level (100, 110, 120, 130), and the picture below holds something else.
    fn masked_sensor(rows: u32) -> Vec<u16> {
        let mut samples = Vec::new();
        for y in 0..8u32 {
            for x in 0..8u32 {
                samples.push(if y < rows {
                    (100 + 10 * CfaPattern::cell_index(x, y)) as u16
                } else {
                    (1000 + x + y * 8) as u16
                });
            }
        }
        samples
    }

    /// The bytes of a `MaskedAreas` entry: `(top, left, bottom, right)` per area.
    fn masked_areas(areas: &[(u32, u32, u32, u32)]) -> Vec<u8> {
        let mut bytes = Vec::new();
        for (top, left, bottom, right) in areas {
            for value in [top, left, bottom, right] {
                bytes.extend_from_slice(&value.to_le_bytes());
            }
        }
        bytes
    }

    #[test]
    fn masked_photosites_give_the_black_level_of_a_file_that_declares_none() {
        let data = DngBuilder::new(8, 8, masked_sensor(2))
            .without_black_level()
            .entry(TAG_MASKED_AREAS, 4, masked_areas(&[(0, 0, 2, 8)]))
            .build();
        let sensor = decode(&data).unwrap();
        assert_eq!(
            sensor.masked_black_level,
            Some([100.0, 110.0, 120.0, 130.0])
        );
        assert_eq!(sensor.black_level, [100.0, 110.0, 120.0, 130.0]);
    }

    #[test]
    fn a_declared_black_level_stays_authoritative_over_the_masked_photosites() {
        let data = DngBuilder::new(8, 8, masked_sensor(2))
            .levels(vec![64], (1, 1), 65535)
            .entry(TAG_MASKED_AREAS, 4, masked_areas(&[(0, 0, 2, 8)]))
            .build();
        let sensor = decode(&data).unwrap();
        assert_eq!(sensor.black_level, [64.0; 4]);
        // Still recorded, so a disagreement can be shown.
        assert_eq!(
            sensor.masked_black_level,
            Some([100.0, 110.0, 120.0, 130.0])
        );
    }

    /// A region and the whole sensor must develop alike, so the window measures what the whole
    /// decode does, whether or not the masked rows are inside the window.
    #[test]
    fn a_window_measures_the_same_black_level_as_the_whole_sensor() {
        let data = DngBuilder::new(8, 8, masked_sensor(2))
            .without_black_level()
            .entry(TAG_MASKED_AREAS, 4, masked_areas(&[(0, 0, 2, 8)]))
            .build();
        let whole = decode(&data).unwrap();
        for window in [
            Rect {
                x: 0,
                y: 2,
                width: 6,
                height: 6,
            },
            Rect {
                x: 2,
                y: 0,
                width: 6,
                height: 4,
            },
            Rect {
                x: 4,
                y: 4,
                width: 4,
                height: 4,
            },
        ] {
            let part = decode_window(&data, window).unwrap();
            assert_eq!(part.black_level, whole.black_level, "{window:?}");
            assert_eq!(
                part.masked_black_level, whole.masked_black_level,
                "{window:?}"
            );
        }
    }

    #[test]
    fn masked_areas_that_do_not_lie_on_the_sensor_are_not_believed() {
        let data = DngBuilder::new(8, 8, masked_sensor(2))
            .without_black_level()
            .entry(
                TAG_MASKED_AREAS,
                4,
                masked_areas(&[(0, 0, 2, 9), (3, 0, 3, 8), (5, 4, 2, 8), (0, 0, 9, 8)]),
            )
            .build();
        let sensor = decode(&data).unwrap();
        assert_eq!(sensor.masked_black_level, None);
        assert_eq!(sensor.black_level, [0.0; 4]);
        // A trailing partial entry is ignored rather than read past.
        let mut ragged = masked_areas(&[(0, 0, 2, 8)]);
        ragged.extend_from_slice(&1u32.to_le_bytes());
        let data = DngBuilder::new(8, 8, masked_sensor(2))
            .without_black_level()
            .entry(TAG_MASKED_AREAS, 4, ragged)
            .build();
        assert_eq!(
            decode(&data).unwrap().masked_black_level,
            Some([100.0, 110.0, 120.0, 130.0])
        );
    }

    /// One row cannot say what black is at the positions of the other row.
    #[test]
    fn a_mask_that_misses_a_cfa_position_measures_nothing() {
        let data = DngBuilder::new(8, 8, masked_sensor(1))
            .without_black_level()
            .entry(TAG_MASKED_AREAS, 4, masked_areas(&[(0, 0, 1, 8)]))
            .build();
        let sensor = decode(&data).unwrap();
        assert_eq!(sensor.masked_black_level, None);
        assert_eq!(sensor.black_level, [0.0; 4]);
    }

    /// A measurement that is not below white would make the whole file undevelopable, so it is
    /// not believed instead; the file still opens.
    #[test]
    fn a_measured_level_at_or_above_white_is_not_believed() {
        let data = DngBuilder::new(8, 8, vec![4000; 64])
            .levels(vec![0], (1, 1), 4000)
            .without_black_level()
            .entry(TAG_MASKED_AREAS, 4, masked_areas(&[(0, 0, 2, 8)]))
            .build();
        let sensor = decode(&data).unwrap();
        assert_eq!(sensor.masked_black_level, None);
        assert_eq!(sensor.black_level, [0.0; 4]);
    }

    #[test]
    fn a_file_without_masked_areas_is_unchanged() {
        let data = DngBuilder::new(8, 8, masked_sensor(0))
            .levels(vec![512], (1, 1), 16383)
            .build();
        let sensor = decode(&data).unwrap();
        assert_eq!(sensor.masked_black_level, None);
        assert_eq!(sensor.black_level, [512.0; 4]);
    }
}
