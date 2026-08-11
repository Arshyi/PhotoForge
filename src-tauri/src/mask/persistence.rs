use super::bitmap::{checked_length, MaskBitmap, MAX_MASK_PIXELS};
use super::diagnostics::{
    inspect_with_progress as mask_diagnostics_with_progress, MaskDiagnostics,
};
use super::progress::{MaskProgressHandle, MaskWorkContext, PlannedMaskProgress};
use crate::error::AppError;
use base64::engine::general_purpose::STANDARD_NO_PAD;
use base64::Engine;
use image::{DynamicImage, ImageFormat, ImageReader};
use serde::{Deserialize, Serialize};
use std::fs::{self, File};
use std::io::{BufWriter, Cursor, Read, Write};
use std::path::{Component, Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use tempfile::{Builder as TempFileBuilder, NamedTempFile};

pub const MASK_FORMAT_VERSION: u32 = 1;
const RAW_MASK_ENCODING: &str = "base64_u8";
const RLE_MASK_ENCODING: &str = "base64_rle_u8";
const MAX_MASK_FILE_BYTES: u64 = MAX_MASK_PIXELS * 4 / 3 + 1_048_576;
const MAX_MASK_PNG_FILE_BYTES: u64 = MAX_MASK_PIXELS * 4 + 1_048_576;
const IO_CHUNK_BYTES: usize = 64 * 1024;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MaskSnapshot {
    pub version: u32,
    pub width: u32,
    pub height: u32,
    pub encoding: String,
    pub data: String,
    pub checksum: String,
}

impl MaskSnapshot {
    pub fn encode(bitmap: &MaskBitmap) -> Self {
        Self::encode_with_progress(bitmap, MaskWorkContext::new(None, None))
            .expect("mask encoding without cancellation is infallible")
    }

    pub fn encode_with_progress(
        bitmap: &MaskBitmap,
        context: MaskWorkContext<'_>,
    ) -> Result<Self, AppError> {
        let rle = encode_rle_with_progress(bitmap.coverage(), context)?;
        let (encoding, bytes) = if rle.len() < bitmap.coverage().len() {
            (RLE_MASK_ENCODING, rle.as_slice())
        } else {
            (RAW_MASK_ENCODING, bitmap.coverage())
        };
        context.check_cancelled()?;
        let data = STANDARD_NO_PAD.encode(bytes);
        context.check_cancelled()?;
        let checksum = checksum_with_progress(bitmap.coverage(), context)?;
        Ok(Self {
            version: MASK_FORMAT_VERSION,
            width: bitmap.width(),
            height: bitmap.height(),
            encoding: encoding.into(),
            data,
            checksum,
        })
    }

    pub fn decode(&self) -> Result<MaskBitmap, AppError> {
        self.decode_with_progress(MaskWorkContext::new(None, None))
    }

    pub fn decode_with_progress(
        &self,
        context: MaskWorkContext<'_>,
    ) -> Result<MaskBitmap, AppError> {
        let expected = self.validate_structure()?;
        let encoded = decode_base64_with_progress(
            self.data.as_bytes(),
            if self.encoding == RAW_MASK_ENCODING {
                Some((context, expected as u64))
            } else {
                None
            },
            context,
        )?;
        let decoded = if self.encoding == RLE_MASK_ENCODING {
            decode_rle_with_progress(&encoded, expected, context)?
        } else {
            encoded
        };
        if decoded.len() != expected {
            return Err(AppError::InvalidMask(
                "decoded mask data does not match its dimensions".into(),
            ));
        }
        if checksum_with_progress(&decoded, context)? != self.checksum {
            return Err(AppError::InvalidMask(
                "mask integrity checksum does not match".into(),
            ));
        }
        MaskBitmap::from_coverage(self.width, self.height, decoded)
    }

    fn validate_structure(&self) -> Result<usize, AppError> {
        if self.version != MASK_FORMAT_VERSION {
            return Err(AppError::UnsupportedMaskVersion(self.version));
        }
        if !matches!(
            self.encoding.as_str(),
            RAW_MASK_ENCODING | RLE_MASK_ENCODING
        ) {
            return Err(AppError::InvalidMask(format!(
                "unsupported mask encoding {}",
                self.encoding
            )));
        }
        let expected = checked_length(self.width, self.height)?;
        let maximum_encoded = expected
            .checked_mul(4)
            .and_then(|value| value.checked_div(3))
            .and_then(|value| value.checked_add(4))
            .ok_or(AppError::OutOfMemoryRisk)?;
        if self.data.len() > maximum_encoded {
            return Err(AppError::InvalidMask(
                "encoded mask data exceeds the dimensions".into(),
            ));
        }
        if !is_canonical_standard_no_pad(&self.data) {
            return Err(AppError::InvalidMask(
                "mask data is not canonical unpadded base64".into(),
            ));
        }
        let checksum = self
            .checksum
            .strip_prefix("fnv1a64:")
            .ok_or_else(|| AppError::InvalidMask("mask integrity checksum is malformed".into()))?;
        if checksum.len() != 16 || !checksum.bytes().all(|value| value.is_ascii_hexdigit()) {
            return Err(AppError::InvalidMask(
                "mask integrity checksum is malformed".into(),
            ));
        }
        Ok(expected)
    }

    pub fn decode_work_units(&self) -> Result<u64, AppError> {
        Ok((checked_length(self.width, self.height)? as u64).saturating_mul(2))
    }
}

fn is_canonical_standard_no_pad(value: &str) -> bool {
    fn sextet(byte: u8) -> Option<u8> {
        match byte {
            b'A'..=b'Z' => Some(byte - b'A'),
            b'a'..=b'z' => Some(byte - b'a' + 26),
            b'0'..=b'9' => Some(byte - b'0' + 52),
            b'+' => Some(62),
            b'/' => Some(63),
            _ => None,
        }
    }

    let bytes = value.as_bytes();
    if bytes.len() % 4 == 1 || bytes.iter().any(|byte| sextet(*byte).is_none()) {
        return false;
    }
    match bytes.len() % 4 {
        2 => sextet(*bytes.last().expect("remainder implies a final byte"))
            .is_some_and(|value| value & 0x0f == 0),
        3 => sextet(*bytes.last().expect("remainder implies a final byte"))
            .is_some_and(|value| value & 0x03 == 0),
        _ => true,
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct MaskMetadata {
    #[serde(default)]
    pub created_at: String,
    #[serde(default)]
    pub modified_at: String,
    #[serde(default)]
    pub source_tool: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MaskFile {
    pub format: String,
    pub version: u32,
    pub id: String,
    pub name: String,
    pub mask: MaskSnapshot,
    #[serde(default)]
    pub metadata: MaskMetadata,
}

impl MaskFile {
    pub fn new(id: String, name: String, mask: MaskSnapshot, metadata: MaskMetadata) -> Self {
        Self {
            format: "photoforge-mask".into(),
            version: MASK_FORMAT_VERSION,
            id,
            name,
            mask,
            metadata,
        }
    }

    pub fn validate(&self) -> Result<MaskBitmap, AppError> {
        self.validate_with_progress(MaskWorkContext::new(None, None))
    }

    pub fn validate_with_progress(
        &self,
        context: MaskWorkContext<'_>,
    ) -> Result<MaskBitmap, AppError> {
        self.validate_structure()?;
        self.mask.decode_with_progress(context)
    }

    fn validate_structure(&self) -> Result<(), AppError> {
        if self.format != "photoforge-mask" {
            return Err(AppError::InvalidMask(
                "file is not a PhotoForge mask".into(),
            ));
        }
        if self.version != MASK_FORMAT_VERSION {
            return Err(AppError::UnsupportedMaskVersion(self.version));
        }
        if self.id.trim().is_empty()
            || self.id.len() > 128
            || self.name.trim().is_empty()
            || self.name.len() > 120
            || self.metadata.created_at.len() > 128
            || self.metadata.modified_at.len() > 128
            || self
                .metadata
                .source_tool
                .as_ref()
                .is_some_and(|value| value.len() > 120)
        {
            return Err(AppError::InvalidMask(
                "mask identity, name, or metadata is invalid".into(),
            ));
        }
        self.mask.validate_structure()?;
        Ok(())
    }
}

pub fn save_mask(path: &Path, document: &MaskFile) -> Result<PathBuf, AppError> {
    validate_local_path(path)?;
    document.validate()?;
    let bytes =
        serde_json::to_vec_pretty(document).map_err(|error| AppError::MaskIo(error.to_string()))?;
    if bytes.len() as u64 > MAX_MASK_FILE_BYTES {
        return Err(AppError::MaskTooLarge {
            pixels: bytes.len() as u64,
            limit: MAX_MASK_FILE_BYTES,
        });
    }
    atomic_write(path, &bytes)?;
    Ok(path.to_path_buf())
}

pub fn load_mask(path: &Path) -> Result<MaskFile, AppError> {
    validate_local_path(path)?;
    let metadata = fs::metadata(path).map_err(map_io)?;
    if metadata.len() > MAX_MASK_FILE_BYTES {
        return Err(AppError::MaskTooLarge {
            pixels: metadata.len(),
            limit: MAX_MASK_FILE_BYTES,
        });
    }
    let bytes = fs::read(path).map_err(map_io)?;
    let document: MaskFile = serde_json::from_slice(&bytes)
        .map_err(|error| AppError::InvalidMask(format!("malformed mask JSON: {error}")))?;
    document.validate()?;
    Ok(document)
}

pub fn export_png(path: &Path, mask: &MaskBitmap) -> Result<PathBuf, AppError> {
    validate_local_path(path)?;
    export_bitmap_png_with_progress(path, mask, MaskWorkContext::new(None, None))?;
    Ok(path.to_path_buf())
}

pub fn import_png(path: &Path) -> Result<MaskBitmap, AppError> {
    validate_local_path(path)?;
    let reader = ImageReader::open(path)
        .map_err(map_io)?
        .with_guessed_format()
        .map_err(map_io)?;
    if reader.format() != Some(ImageFormat::Png) {
        return Err(AppError::InvalidMask(
            "grayscale mask import requires a PNG file".into(),
        ));
    }
    let (width, height) = reader
        .into_dimensions()
        .map_err(|error| AppError::InvalidMask(error.to_string()))?;
    checked_length(width, height)?;
    let image = ImageReader::open(path)
        .map_err(map_io)?
        .with_guessed_format()
        .map_err(map_io)?
        .decode()
        .map_err(|error| AppError::InvalidMask(error.to_string()))?;
    grayscale_to_mask(image)
}

pub(crate) fn save_mask_with_progress(
    path: &Path,
    document: &MaskFile,
    progress: &MaskProgressHandle,
    cancelled: &AtomicBool,
) -> Result<PathBuf, AppError> {
    validate_local_path(path)?;
    document.validate_structure()?;
    progress.mark_running("serialize_mask_file")?;
    check_cancelled(cancelled)?;
    let bytes =
        serde_json::to_vec_pretty(document).map_err(|error| AppError::MaskIo(error.to_string()))?;
    check_cancelled(cancelled)?;
    if bytes.len() as u64 > MAX_MASK_FILE_BYTES {
        return Err(AppError::MaskTooLarge {
            pixels: bytes.len() as u64,
            limit: MAX_MASK_FILE_BYTES,
        });
    }

    let decode_units = document.mask.decode_work_units()?;
    let total_units = decode_units.saturating_add(bytes.len() as u64);
    let planned = progress.planned(total_units);
    let report =
        |phase: &str, completed: u64, total: u64| planned.report_local(phase, completed, total);
    let context = MaskWorkContext::new(Some(cancelled), Some(&report));
    document.validate_with_progress(context)?;
    atomic_write_with_progress(path, &bytes, context, "write_mask_file_bytes")?;
    Ok(path.to_path_buf())
}

pub(crate) fn load_mask_with_progress(
    path: &Path,
    progress: &MaskProgressHandle,
    cancelled: &AtomicBool,
) -> Result<(MaskFile, MaskDiagnostics), AppError> {
    validate_local_path(path)?;
    let file_bytes = bounded_file_length(path, MAX_MASK_FILE_BYTES)?;
    progress.mark_running("inspect_mask_file_bytes")?;
    let preflight_bytes = read_bounded_file(path, MAX_MASK_FILE_BYTES, cancelled)?;
    if preflight_bytes.len() as u64 != file_bytes {
        return Err(AppError::InvalidMask(
            "mask file changed while it was being read".into(),
        ));
    }
    progress.mark_running("parse_mask_file")?;
    check_cancelled(cancelled)?;
    let preflight_document: MaskFile = serde_json::from_slice(&preflight_bytes)
        .map_err(|error| AppError::InvalidMask(format!("malformed mask JSON: {error}")))?;
    preflight_document.validate_structure()?;
    check_cancelled(cancelled)?;

    let expected_dimensions = (
        preflight_document.mask.width,
        preflight_document.mask.height,
    );
    let pixels = checked_length(expected_dimensions.0, expected_dimensions.1)? as u64;
    let decode_units = preflight_document.mask.decode_work_units()?;
    drop(preflight_document);
    drop(preflight_bytes);

    let total_units = file_bytes
        .saturating_mul(2)
        .saturating_add(decode_units)
        .saturating_add(pixels);
    let planned = progress.planned(total_units);
    planned.report_local("inspect_mask_file_bytes", file_bytes, file_bytes)?;
    let bytes = read_bounded_file_with_progress(
        path,
        MAX_MASK_FILE_BYTES,
        file_bytes,
        cancelled,
        &planned,
        "read_mask_file_bytes",
    )?;
    planned.report_local("parse_mask_file", 0, 0)?;
    check_cancelled(cancelled)?;
    let document: MaskFile = serde_json::from_slice(&bytes)
        .map_err(|error| AppError::InvalidMask(format!("malformed mask JSON: {error}")))?;
    document.validate_structure()?;
    if (document.mask.width, document.mask.height) != expected_dimensions {
        return Err(AppError::InvalidMask(
            "mask file changed while it was being read".into(),
        ));
    }
    let report =
        |phase: &str, completed: u64, total: u64| planned.report_local(phase, completed, total);
    let context = MaskWorkContext::new(Some(cancelled), Some(&report));
    let bitmap = document.validate_with_progress(context)?;
    let diagnostics = mask_diagnostics_with_progress(&bitmap, context)?;
    Ok((document, diagnostics))
}

pub(crate) fn export_png_snapshot_with_progress(
    path: &Path,
    snapshot: &MaskSnapshot,
    progress: &MaskProgressHandle,
    cancelled: &AtomicBool,
) -> Result<PathBuf, AppError> {
    validate_local_path(path)?;
    let pixels = checked_length(snapshot.width, snapshot.height)? as u64;
    let total_units = snapshot.decode_work_units()?.saturating_add(pixels);
    let planned = progress.planned(total_units);
    let report =
        |phase: &str, completed: u64, total: u64| planned.report_local(phase, completed, total);
    let context = MaskWorkContext::new(Some(cancelled), Some(&report));
    let bitmap = snapshot.decode_with_progress(context)?;
    export_bitmap_png_with_progress(path, &bitmap, context)?;
    Ok(path.to_path_buf())
}

pub(crate) fn import_png_snapshot_with_progress(
    path: &Path,
    progress: &MaskProgressHandle,
    cancelled: &AtomicBool,
) -> Result<(MaskSnapshot, MaskDiagnostics), AppError> {
    validate_local_path(path)?;
    progress.mark_running("inspect_png")?;
    check_cancelled(cancelled)?;
    let reader = ImageReader::open(path)
        .map_err(map_io)?
        .with_guessed_format()
        .map_err(map_io)?;
    if reader.format() != Some(ImageFormat::Png) {
        return Err(AppError::InvalidMask(
            "grayscale mask import requires a PNG file".into(),
        ));
    }
    let (width, height) = reader
        .into_dimensions()
        .map_err(|error| AppError::InvalidMask(error.to_string()))?;
    let pixels = checked_length(width, height)? as u64;
    check_cancelled(cancelled)?;

    let file_bytes = bounded_file_length(path, MAX_MASK_PNG_FILE_BYTES)?;
    let total_units = file_bytes.saturating_add(pixels.saturating_mul(4));
    let planned = progress.planned(total_units);
    let bytes = read_bounded_file_with_progress(
        path,
        MAX_MASK_PNG_FILE_BYTES,
        file_bytes,
        cancelled,
        &planned,
        "read_png_file_bytes",
    )?;
    planned.report_local("decode_png", 0, 0)?;
    check_cancelled(cancelled)?;
    let reader = ImageReader::new(Cursor::new(bytes))
        .with_guessed_format()
        .map_err(map_io)?;
    if reader.format() != Some(ImageFormat::Png) {
        return Err(AppError::InvalidMask(
            "grayscale mask import requires a PNG file".into(),
        ));
    }
    let image = reader
        .decode()
        .map_err(|error| AppError::InvalidMask(error.to_string()))?;
    check_cancelled(cancelled)?;
    if image.width() != width || image.height() != height {
        return Err(AppError::InvalidMask(
            "decoded PNG dimensions changed while reading".into(),
        ));
    }

    let grayscale = image.into_luma8();
    check_cancelled(cancelled)?;
    let raw = grayscale.into_raw();
    let mut coverage = Vec::new();
    coverage
        .try_reserve_exact(raw.len())
        .map_err(|_| AppError::OutOfMemoryRisk)?;
    for chunk in raw.chunks(IO_CHUNK_BYTES) {
        check_cancelled(cancelled)?;
        coverage.extend_from_slice(chunk);
        planned.report_local("convert_png_pixels", coverage.len() as u64, pixels)?;
    }
    let bitmap = MaskBitmap::from_coverage(width, height, coverage)?;
    let report =
        |phase: &str, completed: u64, total: u64| planned.report_local(phase, completed, total);
    let context = MaskWorkContext::new(Some(cancelled), Some(&report));
    let diagnostics = mask_diagnostics_with_progress(&bitmap, context)?;
    let snapshot = MaskSnapshot::encode_with_progress(&bitmap, context)?;
    Ok((snapshot, diagnostics))
}

fn grayscale_to_mask(image: DynamicImage) -> Result<MaskBitmap, AppError> {
    let grayscale = image.to_luma8();
    MaskBitmap::from_coverage(grayscale.width(), grayscale.height(), grayscale.into_raw())
}

fn checksum_with_progress(data: &[u8], context: MaskWorkContext<'_>) -> Result<String, AppError> {
    let mut hash = 0xcbf29ce484222325_u64;
    let total = data.len() as u64;
    context.report("validate_mask_pixels", 0, total)?;
    for (index, chunk) in data.chunks(IO_CHUNK_BYTES).enumerate() {
        for byte in chunk {
            hash ^= u64::from(*byte);
            hash = hash.wrapping_mul(0x100000001b3);
        }
        let completed = ((index + 1) * IO_CHUNK_BYTES).min(data.len()) as u64;
        context.report("validate_mask_pixels", completed, total)?;
    }
    Ok(format!("fnv1a64:{hash:016x}"))
}

fn encode_rle_with_progress(
    data: &[u8],
    context: MaskWorkContext<'_>,
) -> Result<Vec<u8>, AppError> {
    if data.is_empty() {
        context.report("encode_mask_pixels", 0, 0)?;
        return Ok(Vec::new());
    }
    let total = data.len() as u64;
    context.report("encode_mask_pixels", 0, total)?;
    let mut encoded = Vec::new();
    let mut value = data[0];
    let mut count = 1_u32;
    for (index, &next) in data[1..].iter().enumerate() {
        if next == value && count < u32::MAX {
            count += 1;
        } else {
            encoded.push(value);
            encoded.extend_from_slice(&count.to_le_bytes());
            value = next;
            count = 1;
        }
        let completed = index + 2;
        if completed % IO_CHUNK_BYTES == 0 || completed == data.len() {
            context.report("encode_mask_pixels", completed as u64, total)?;
        }
    }
    encoded.push(value);
    encoded.extend_from_slice(&count.to_le_bytes());
    if data.len() == 1 {
        context.report("encode_mask_pixels", 1, 1)?;
    }
    Ok(encoded)
}

fn decode_rle_with_progress(
    encoded: &[u8],
    expected: usize,
    context: MaskWorkContext<'_>,
) -> Result<Vec<u8>, AppError> {
    if encoded.is_empty() || encoded.len() % 5 != 0 {
        return Err(AppError::InvalidMask(
            "run-length mask data is malformed".into(),
        ));
    }
    let mut decoded = Vec::with_capacity(expected);
    context.report("decode_mask_pixels", 0, expected as u64)?;
    for chunk in encoded.chunks_exact(5) {
        context.check_cancelled()?;
        let count = u32::from_le_bytes([chunk[1], chunk[2], chunk[3], chunk[4]]) as usize;
        let next_length = decoded
            .len()
            .checked_add(count)
            .ok_or(AppError::OutOfMemoryRisk)?;
        if count == 0 || next_length > expected {
            return Err(AppError::InvalidMask(
                "run-length mask data exceeds its dimensions".into(),
            ));
        }
        decoded.resize(decoded.len() + count, chunk[0]);
        context.report("decode_mask_pixels", decoded.len() as u64, expected as u64)?;
    }
    if decoded.len() != expected {
        return Err(AppError::InvalidMask(
            "run-length mask data does not match its dimensions".into(),
        ));
    }
    Ok(decoded)
}

fn decode_base64_with_progress(
    data: &[u8],
    raw_progress: Option<(MaskWorkContext<'_>, u64)>,
    context: MaskWorkContext<'_>,
) -> Result<Vec<u8>, AppError> {
    const BASE64_CHUNK_BYTES: usize = IO_CHUNK_BYTES - (IO_CHUNK_BYTES % 4);
    let mut decoded = Vec::new();
    if let Some((progress, total)) = raw_progress {
        progress.report("decode_mask_pixels", 0, total)?;
    } else {
        context.report("decode_mask_payload", 0, 0)?;
    }
    for chunk in data.chunks(BASE64_CHUNK_BYTES) {
        context.check_cancelled()?;
        let bytes = STANDARD_NO_PAD
            .decode(chunk)
            .map_err(|_| AppError::InvalidMask("mask data is not valid base64".into()))?;
        decoded
            .try_reserve(bytes.len())
            .map_err(|_| AppError::OutOfMemoryRisk)?;
        decoded.extend_from_slice(&bytes);
        if let Some((progress, total)) = raw_progress {
            progress.report("decode_mask_pixels", decoded.len() as u64, total)?;
        }
    }
    Ok(decoded)
}

fn read_bounded_file(path: &Path, limit: u64, cancelled: &AtomicBool) -> Result<Vec<u8>, AppError> {
    let expected = bounded_file_length(path, limit)?;
    read_bounded_file_chunks(path, limit, expected, cancelled, None)
}

fn read_bounded_file_with_progress(
    path: &Path,
    limit: u64,
    expected: u64,
    cancelled: &AtomicBool,
    progress: &PlannedMaskProgress,
    phase: &str,
) -> Result<Vec<u8>, AppError> {
    let report = |completed: u64, total: u64| progress.report_local(phase, completed, total);
    read_bounded_file_chunks(path, limit, expected, cancelled, Some(&report))
}

fn read_bounded_file_chunks(
    path: &Path,
    limit: u64,
    expected: u64,
    cancelled: &AtomicBool,
    progress: Option<&dyn Fn(u64, u64) -> Result<(), AppError>>,
) -> Result<Vec<u8>, AppError> {
    let capacity = usize::try_from(expected).map_err(|_| AppError::OutOfMemoryRisk)?;
    let mut bytes = Vec::new();
    bytes
        .try_reserve_exact(capacity)
        .map_err(|_| AppError::OutOfMemoryRisk)?;
    let mut file = File::open(path).map_err(map_io)?;
    let mut chunk = [0_u8; IO_CHUNK_BYTES];
    if let Some(report) = progress {
        report(0, expected)?;
    }
    loop {
        check_cancelled(cancelled)?;
        let read = file.read(&mut chunk).map_err(map_io)?;
        if read == 0 {
            break;
        }
        let next_length = (bytes.len() as u64)
            .checked_add(read as u64)
            .ok_or(AppError::OutOfMemoryRisk)?;
        if next_length > limit {
            return Err(AppError::MaskTooLarge {
                pixels: next_length,
                limit,
            });
        }
        if next_length > expected {
            return Err(AppError::InvalidMask(
                "mask file changed while it was being read".into(),
            ));
        }
        bytes
            .try_reserve(read)
            .map_err(|_| AppError::OutOfMemoryRisk)?;
        bytes.extend_from_slice(&chunk[..read]);
        if let Some(report) = progress {
            report(bytes.len() as u64, expected)?;
        }
    }
    check_cancelled(cancelled)?;
    if bytes.len() as u64 != expected {
        return Err(AppError::InvalidMask(
            "mask file changed while it was being read".into(),
        ));
    }
    Ok(bytes)
}

fn bounded_file_length(path: &Path, limit: u64) -> Result<u64, AppError> {
    let length = fs::metadata(path).map_err(map_io)?.len();
    if length > limit {
        Err(AppError::MaskTooLarge {
            pixels: length,
            limit,
        })
    } else {
        Ok(length)
    }
}

fn atomic_write_with_progress(
    path: &Path,
    bytes: &[u8],
    context: MaskWorkContext<'_>,
    phase: &str,
) -> Result<(), AppError> {
    let mut temporary = secure_temporary_file(path)?;
    {
        let mut writer = BufWriter::new(temporary.as_file_mut());
        let total = bytes.len() as u64;
        context.report(phase, 0, total)?;
        let mut completed = 0_u64;
        for chunk in bytes.chunks(IO_CHUNK_BYTES) {
            context.check_cancelled()?;
            writer.write_all(chunk).map_err(map_io)?;
            completed = completed.saturating_add(chunk.len() as u64);
            context.report(phase, completed, total)?;
        }
        writer.flush().map_err(map_io)?;
    }
    temporary.as_file().sync_all().map_err(map_io)?;
    context.check_cancelled()?;
    persist_temporary_file(temporary, path)
}

fn export_bitmap_png_with_progress(
    path: &Path,
    mask: &MaskBitmap,
    context: MaskWorkContext<'_>,
) -> Result<(), AppError> {
    let mut temporary = secure_temporary_file(path)?;
    {
        let mut encoder = png::Encoder::new(temporary.as_file_mut(), mask.width(), mask.height());
        encoder.set_color(png::ColorType::Grayscale);
        encoder.set_depth(png::BitDepth::Eight);
        let mut writer = encoder
            .write_header()
            .map_err(|error| AppError::MaskIo(error.to_string()))?;
        let mut stream = writer
            .stream_writer()
            .map_err(|error| AppError::MaskIo(error.to_string()))?;
        let width = mask.width() as usize;
        let total = mask.coverage().len() as u64;
        context.report("encode_png_pixels", 0, total)?;
        let mut completed = 0_u64;
        for row in mask.coverage().chunks_exact(width) {
            context.check_cancelled()?;
            stream
                .write_all(row)
                .map_err(|error| AppError::MaskIo(error.to_string()))?;
            completed = completed.saturating_add(row.len() as u64);
            context.report("encode_png_pixels", completed, total)?;
        }
        stream
            .finish()
            .map_err(|error| AppError::MaskIo(error.to_string()))?;
    }
    temporary.as_file().sync_all().map_err(map_io)?;
    context.check_cancelled()?;
    persist_temporary_file(temporary, path)
}

fn check_cancelled(cancelled: &AtomicBool) -> Result<(), AppError> {
    if cancelled.load(Ordering::Acquire) {
        Err(AppError::MaskCancelled)
    } else {
        Ok(())
    }
}

fn atomic_write(path: &Path, bytes: &[u8]) -> Result<(), AppError> {
    let mut temporary = secure_temporary_file(path)?;
    temporary.write_all(bytes).map_err(map_io)?;
    temporary.flush().map_err(map_io)?;
    temporary.as_file().sync_all().map_err(map_io)?;
    persist_temporary_file(temporary, path)
}

fn secure_temporary_file(path: &Path) -> Result<NamedTempFile, AppError> {
    let parent = path
        .parent()
        .ok_or_else(|| AppError::MaskIo("mask path has no parent folder".into()))?;
    TempFileBuilder::new()
        .prefix(".photoforge-mask-")
        .suffix(".tmp")
        .tempfile_in(parent)
        .map_err(map_io)
}

fn persist_temporary_file(temporary: NamedTempFile, destination: &Path) -> Result<(), AppError> {
    temporary
        .persist(destination)
        .map(|_| ())
        .map_err(|error| map_io(error.error))
}

fn validate_local_path(path: &Path) -> Result<(), AppError> {
    let text = path.to_string_lossy();
    if !path.is_absolute()
        || text.starts_with("\\\\")
        || text.starts_with("//")
        || text.contains("://")
        || path.components().any(|part| part == Component::ParentDir)
    {
        return Err(AppError::MaskIo(
            "mask paths must be absolute local paths without parent traversal".into(),
        ));
    }
    Ok(())
}

fn map_io(error: std::io::Error) -> AppError {
    if error.kind() == std::io::ErrorKind::PermissionDenied {
        AppError::Permission
    } else {
        AppError::MaskIo(error.to_string())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::mask::{MaskProgressState, SharedMaskProgress};
    use std::sync::{Arc, Mutex};

    fn progress(document_id: u64, request_id: u64, operation: &str) -> MaskProgressHandle {
        let shared: SharedMaskProgress = Arc::new(Mutex::new(None));
        MaskProgressHandle::begin(shared, document_id, request_id, operation).unwrap()
    }

    #[test]
    fn snapshot_round_trip_and_corruption_detection() {
        let mask = MaskBitmap::from_coverage(2, 2, vec![0, 64, 128, 255]).unwrap();
        let snapshot = MaskSnapshot::encode(&mask);
        assert_eq!(snapshot.decode().unwrap(), mask);
        let mut corrupt = snapshot;
        corrupt.checksum = "fnv1a64:0000000000000000".into();
        assert!(corrupt.decode().is_err());
    }

    #[test]
    fn coherent_masks_use_bounded_run_length_encoding() {
        let mask = MaskBitmap::full(1_000, 1_000).unwrap();
        let snapshot = MaskSnapshot::encode(&mask);
        assert_eq!(snapshot.encoding, RLE_MASK_ENCODING);
        assert!(snapshot.data.len() < 100);
        assert_eq!(snapshot.decode().unwrap(), mask);
    }

    #[test]
    fn malformed_run_length_data_is_rejected_without_large_allocation() {
        let mut snapshot = MaskSnapshot::encode(&MaskBitmap::full(2, 2).unwrap());
        snapshot.encoding = RLE_MASK_ENCODING.into();
        snapshot.data = STANDARD_NO_PAD.encode([255, 255, 255, 255, 127]);
        assert!(snapshot.decode().is_err());
    }

    #[test]
    fn json_and_png_round_trip() {
        let directory = tempfile::tempdir().unwrap();
        let mask = MaskBitmap::from_coverage(2, 2, vec![0, 64, 128, 255]).unwrap();
        let document = MaskFile::new(
            "mask-1".into(),
            "Subject".into(),
            MaskSnapshot::encode(&mask),
            MaskMetadata::default(),
        );
        let json_path = directory.path().join("subject.photoforge-mask.json");
        save_mask(&json_path, &document).unwrap();
        assert_eq!(load_mask(&json_path).unwrap(), document);
        let png_path = directory.path().join("subject.png");
        export_png(&png_path, &mask).unwrap();
        assert_eq!(import_png(&png_path).unwrap(), mask);
    }

    #[test]
    fn progressed_json_and_png_round_trips_reach_exact_real_unit_totals() {
        let directory = tempfile::tempdir().unwrap();
        let coverage: Vec<u8> = (0..(257 * 193)).map(|index| (index % 251) as u8).collect();
        let mask = MaskBitmap::from_coverage(257, 193, coverage).unwrap();
        let document = MaskFile::new(
            "mask-progress".into(),
            "Progress fixture".into(),
            MaskSnapshot::encode(&mask),
            MaskMetadata::default(),
        );
        let cancelled = AtomicBool::new(false);

        let json_path = directory.path().join("progress.photoforge-mask.json");
        let save_progress = progress(1, 1, "export_mask_file");
        save_mask_with_progress(&json_path, &document, &save_progress, &cancelled).unwrap();
        let saved = save_progress.snapshot().unwrap().unwrap();
        assert_eq!(saved.state, MaskProgressState::Running);
        assert_eq!(saved.completed_units, saved.total_units);

        let load_progress = progress(1, 2, "import_mask_file");
        let (loaded, loaded_diagnostics) =
            load_mask_with_progress(&json_path, &load_progress, &cancelled).unwrap();
        assert_eq!(loaded, document);
        assert_eq!(loaded_diagnostics.width, mask.width());
        assert_eq!(loaded_diagnostics.height, mask.height());
        let loaded_progress = load_progress.snapshot().unwrap().unwrap();
        assert_eq!(loaded_progress.completed_units, loaded_progress.total_units);

        let png_path = directory.path().join("progress.png");
        let export_progress = progress(1, 3, "export_mask_png");
        export_png_snapshot_with_progress(&png_path, &document.mask, &export_progress, &cancelled)
            .unwrap();
        let exported = export_progress.snapshot().unwrap().unwrap();
        assert_eq!(exported.completed_units, exported.total_units);

        let import_progress = progress(1, 4, "import_mask_png");
        let (imported, imported_diagnostics) =
            import_png_snapshot_with_progress(&png_path, &import_progress, &cancelled).unwrap();
        assert_eq!(imported.decode().unwrap(), mask);
        assert_eq!(imported_diagnostics.width, mask.width());
        assert_eq!(imported_diagnostics.height, mask.height());
        let imported_progress = import_progress.snapshot().unwrap().unwrap();
        assert_eq!(
            imported_progress.completed_units,
            imported_progress.total_units
        );
    }

    #[test]
    fn snapshot_decode_cancellation_is_acknowledged_between_real_chunks() {
        let mask = MaskBitmap::from_coverage(
            512,
            512,
            (0..(512 * 512)).map(|index| (index % 251) as u8).collect(),
        )
        .unwrap();
        let snapshot = MaskSnapshot::encode(&mask);
        let cancelled = AtomicBool::new(false);
        let report = |phase: &str, completed: u64, _total: u64| {
            if phase == "decode_mask_pixels" && completed > 0 {
                cancelled.store(true, Ordering::Release);
            }
            Ok(())
        };
        assert!(matches!(
            snapshot.decode_with_progress(MaskWorkContext::new(Some(&cancelled), Some(&report))),
            Err(AppError::MaskCancelled)
        ));
    }

    #[test]
    fn cancelled_atomic_json_write_preserves_destination_and_removes_temporary_file() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("existing.photoforge-mask.json");
        fs::write(&path, b"existing").unwrap();
        let bytes = vec![b'x'; IO_CHUNK_BYTES * 3];
        let cancelled = AtomicBool::new(false);
        let report = |_phase: &str, completed: u64, _total: u64| {
            if completed >= IO_CHUNK_BYTES as u64 {
                cancelled.store(true, Ordering::Release);
            }
            Ok(())
        };
        assert!(matches!(
            atomic_write_with_progress(
                &path,
                &bytes,
                MaskWorkContext::new(Some(&cancelled), Some(&report)),
                "write_mask_file_bytes"
            ),
            Err(AppError::MaskCancelled)
        ));
        assert_eq!(fs::read(&path).unwrap(), b"existing");
        let remaining: Vec<_> = fs::read_dir(directory.path())
            .unwrap()
            .map(|entry| entry.unwrap().file_name())
            .collect();
        assert_eq!(remaining, vec![path.file_name().unwrap().to_os_string()]);
    }

    #[test]
    fn atomic_write_securely_replaces_an_existing_destination() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("existing.photoforge-mask.json");
        fs::write(&path, b"old").unwrap();
        atomic_write(&path, b"new complete content").unwrap();
        assert_eq!(fs::read(&path).unwrap(), b"new complete content");
        assert_eq!(fs::read_dir(directory.path()).unwrap().count(), 1);
    }

    #[test]
    fn malformed_progressed_import_fails_without_inventing_units() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("malformed.photoforge-mask.json");
        fs::write(&path, b"not json").unwrap();
        let progress = progress(2, 9, "import_mask_file");
        let cancelled = AtomicBool::new(false);
        assert!(matches!(
            load_mask_with_progress(&path, &progress, &cancelled),
            Err(AppError::InvalidMask(_))
        ));
        let snapshot = progress.snapshot().unwrap().unwrap();
        assert_eq!(snapshot.total_units, 0);
        assert_eq!(snapshot.completed_units, 0);
        assert_eq!(snapshot.phase, "parse_mask_file");
    }

    #[test]
    fn progressed_save_rejects_oversized_snapshot_data_before_serialization() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("oversized.photoforge-mask.json");
        let mut snapshot = MaskSnapshot::encode(&MaskBitmap::full(1, 1).unwrap());
        snapshot.data = "A".repeat(1024 * 1024);
        let document = MaskFile::new(
            "oversized".into(),
            "Oversized".into(),
            snapshot,
            MaskMetadata::default(),
        );
        let export_progress = progress(3, 10, "export_mask_file");
        let cancelled = AtomicBool::new(false);

        assert!(matches!(
            save_mask_with_progress(&path, &document, &export_progress, &cancelled),
            Err(AppError::InvalidMask(_))
        ));
        assert!(!path.exists());
        let queued = export_progress.snapshot().unwrap().unwrap();
        assert_eq!(queued.phase, "queued");
        assert_eq!(queued.completed_units, 0);
        assert_eq!(queued.total_units, 0);

        let metadata_path = directory
            .path()
            .join("oversized-metadata.photoforge-mask.json");
        let metadata_document = MaskFile::new(
            "oversized-metadata".into(),
            "Oversized metadata".into(),
            MaskSnapshot::encode(&MaskBitmap::full(1, 1).unwrap()),
            MaskMetadata {
                created_at: "2".repeat(1024 * 1024),
                modified_at: String::new(),
                source_tool: None,
            },
        );
        let metadata_progress = progress(3, 11, "export_mask_file");
        assert!(matches!(
            save_mask_with_progress(
                &metadata_path,
                &metadata_document,
                &metadata_progress,
                &cancelled
            ),
            Err(AppError::InvalidMask(_))
        ));
        assert!(!metadata_path.exists());
        assert_eq!(
            metadata_progress.snapshot().unwrap().unwrap().phase,
            "queued"
        );
    }

    #[test]
    fn progressed_save_rejects_noncanonical_base64_before_serialization() {
        let directory = tempfile::tempdir().unwrap();
        let cancelled = AtomicBool::new(false);
        for (index, (label, payload)) in [
            ("control-character", "A\0"),
            ("invalid-alphabet", "A?"),
            ("padding", "AA=="),
            ("nonzero-trailing-bits", "AB"),
        ]
        .into_iter()
        .enumerate()
        {
            let path = directory
                .path()
                .join(format!("{label}.photoforge-mask.json"));
            let mut snapshot = MaskSnapshot::encode(&MaskBitmap::full(1, 1).unwrap());
            snapshot.data = payload.into();
            let document = MaskFile::new(
                label.into(),
                label.into(),
                snapshot,
                MaskMetadata::default(),
            );
            let export_progress = progress(4, 20 + index as u64, "export_mask_file");

            assert!(matches!(
                save_mask_with_progress(&path, &document, &export_progress, &cancelled),
                Err(AppError::InvalidMask(message))
                    if message == "mask data is not canonical unpadded base64"
            ));
            assert!(!path.exists());
            let queued = export_progress.snapshot().unwrap().unwrap();
            assert_eq!(queued.phase, "queued");
            assert_eq!(queued.completed_units, 0);
            assert_eq!(queued.total_units, 0);
        }
    }

    #[test]
    fn bounded_file_reader_reports_intermediate_byte_progress_and_cancels() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("chunked.photoforge-mask.json");
        let contents = vec![b'x'; IO_CHUNK_BYTES * 3];
        fs::write(&path, &contents).unwrap();
        let cancelled = AtomicBool::new(false);
        let observed_intermediate = AtomicBool::new(false);
        let report = |completed: u64, total: u64| {
            if completed > 0 && completed < total {
                observed_intermediate.store(true, Ordering::Release);
                cancelled.store(true, Ordering::Release);
            }
            Ok(())
        };

        assert!(matches!(
            read_bounded_file_chunks(
                &path,
                contents.len() as u64,
                contents.len() as u64,
                &cancelled,
                Some(&report)
            ),
            Err(AppError::MaskCancelled)
        ));
        assert!(observed_intermediate.load(Ordering::Acquire));
    }

    #[test]
    fn unsupported_versions_and_relative_paths_are_rejected() {
        let mut snapshot = MaskSnapshot::encode(&MaskBitmap::empty(1, 1).unwrap());
        snapshot.version = 99;
        assert!(matches!(
            snapshot.decode(),
            Err(AppError::UnsupportedMaskVersion(99))
        ));
        assert!(load_mask(Path::new("relative.json")).is_err());
    }
}
