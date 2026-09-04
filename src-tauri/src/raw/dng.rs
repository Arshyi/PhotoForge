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
use super::{RawCaptureMetadata, RawError, RAW_MAX_PIXELS};

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
    /// Black level for each CFA position, in sensor units.
    pub black_level: [f32; 4],
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

impl SensorImage {
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

/// Decodes a DNG buffer into its sensor image.
pub fn decode(data: &[u8]) -> Result<SensorImage, RawError> {
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
    super::validate_dimensions(width, height)?;
    crate::resources::ResourceEstimate::decode(width, height, data.len() as u64)
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
    let samples = decode_samples(data, raw, width, height, bits_per_sample, compression)?;

    let cfa = if linear {
        CfaPattern::RGGB
    } else {
        read_cfa(raw)?
    };
    let white_level = raw
        .f64(TAG_WHITE_LEVEL)
        .map(|value| value as f32)
        .unwrap_or_else(|| ((1u32 << bits_per_sample) - 1) as f32);
    let black_level = read_black_level(raw);
    let active_area = read_active_area(raw, width, height);
    let default_crop = read_default_crop(raw);

    let color_matrix = read_color_matrix(&file);
    let as_shot_neutral = read_as_shot_neutral(&file);
    let camera_to_srgb = color_matrix.and_then(|matrix| camera_to_srgb(&matrix));

    validate_levels(&black_level, white_level)?;

    Ok(SensorImage {
        data: samples,
        width,
        height,
        cfa,
        black_level,
        white_level,
        bits_per_sample,
        active_area,
        default_crop,
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

/// Reads the pixel payload, whether it is stored in strips or tiles.
fn decode_samples(
    data: &[u8],
    ifd: &Ifd,
    width: u32,
    height: u32,
    bits: u8,
    compression: u32,
) -> Result<Vec<u16>, RawError> {
    let pixels = u64::from(width) * u64::from(height);
    if pixels > RAW_MAX_PIXELS {
        return Err(RawError::FileTooLarge);
    }

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
    let expected_segments = u64::from(across) * u64::from(down);
    if expected_segments != offsets.len() as u64
        || u64::from(segment_width) * u64::from(segment_height) > RAW_MAX_PIXELS
    {
        return Err(RawError::Malformed(
            "strip/tile dimensions and segment count disagree".into(),
        ));
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
    let mut out = Vec::new();
    out.try_reserve_exact(pixels as usize)
        .map_err(|_| RawError::FileTooLarge)?;
    out.resize(pixels as usize, 0u16);

    for (index, (offset, count)) in offsets.iter().zip(counts.iter()).enumerate() {
        let start = *offset as usize;
        let length = *count as usize;
        let end = start
            .checked_add(length)
            .ok_or_else(|| RawError::Malformed("a strip offset overflows".into()))?;
        let payload = data
            .get(start..end)
            .ok_or_else(|| RawError::Malformed("a strip lies outside the file".into()))?;

        let (origin_x, origin_y) = if tiled {
            let column = (index as u32) % across;
            let row = (index as u32) / across;
            (column * segment_width, row * segment_height)
        } else {
            (0, index as u32 * segment_height)
        };
        if origin_y >= height {
            continue;
        }

        match compression {
            COMPRESSION_NONE => {
                let rows = if tiled { segment_height } else { segment_height.min(height-origin_y) };
                let needed = (u64::from(segment_width)*u64::from(bits)).div_ceil(8).checked_mul(u64::from(rows)).ok_or(RawError::FileTooLarge)?;
                if (payload.len() as u64) < needed {
                    return Err(RawError::Malformed("uncompressed strip/tile payload is truncated".into()));
                }
                copy_uncompressed(
                payload,
                &mut out,
                width,
                height,
                origin_x,
                origin_y,
                segment_width,
                segment_height,
                bits,
                data.starts_with(b"II"),
            )},
            COMPRESSION_LOSSLESS_JPEG => {
                let frame = ljpeg::decode(payload)
                    .map_err(|error| RawError::Malformed(error.to_string()))?;
                if frame.width.checked_mul(frame.components) != Some(segment_width as usize)
                    || frame.height < segment_height.min(height-origin_y) as usize
                    || frame.height > segment_height as usize {
                    return Err(RawError::Malformed("lossless JPEG frame dimensions disagree with its strip/tile".into()));
                }
                copy_frame(&frame, &mut out, width, height, origin_x, origin_y);
            }
            other => {
                return Err(RawError::Unsupported(format!(
                    "DNG compression {other} is not supported; this build reads uncompressed and lossless JPEG"
                )))
            }
        }
    }
    Ok(out)
}

#[allow(clippy::too_many_arguments)]
fn copy_uncompressed(
    payload: &[u8],
    out: &mut [u16],
    width: u32,
    height: u32,
    origin_x: u32,
    origin_y: u32,
    segment_width: u32,
    segment_height: u32,
    bits: u8,
    little_endian: bool,
) {
    // TIFF pads each row to a byte boundary, so rows are addressed by their own
    // stride rather than by a running bit position.
    let row_bits = u64::from(segment_width) * u64::from(bits);
    let row_bytes = row_bits.div_ceil(8) as usize;
    for row in 0..segment_height {
        let y = origin_y + row;
        if y >= height {
            break;
        }
        let row_start = row as usize * row_bytes;
        let Some(row_data) = payload.get(row_start..row_start + row_bytes) else {
            break;
        };
        for column in 0..segment_width {
            let x = origin_x + column;
            if x >= width {
                break;
            }
            let value = if bits == 16 && !little_endian {
                u16::from_be_bytes([
                    row_data[column as usize * 2],
                    row_data[column as usize * 2 + 1],
                ])
            } else {
                read_packed(row_data, column as usize, bits)
            };
            out[y as usize * width as usize + x as usize] = value;
        }
    }
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

fn copy_frame(
    frame: &ljpeg::Frame,
    out: &mut [u16],
    width: u32,
    height: u32,
    origin_x: u32,
    origin_y: u32,
) {
    let row_samples = frame.width * frame.components;
    for row in 0..frame.height {
        let y = origin_y + row as u32;
        if y >= height {
            break;
        }
        for column in 0..row_samples {
            let x = origin_x + column as u32;
            if x >= width {
                break;
            }
            let Some(value) = frame.samples.get(row * row_samples + column) else {
                return;
            };
            out[y as usize * width as usize + x as usize] = *value;
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

    pub struct DngBuilder {
        pub width: u32,
        pub height: u32,
        pub bits: u8,
        pub cfa: [u8; 4],
        pub black: Vec<u32>,
        pub black_repeat: (u32, u32),
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
            }
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
}
