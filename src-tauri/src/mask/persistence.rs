use super::bitmap::{checked_length, MaskBitmap, MAX_MASK_PIXELS};
use crate::error::AppError;
use base64::engine::general_purpose::STANDARD_NO_PAD;
use base64::Engine;
use image::{DynamicImage, ImageFormat, ImageReader};
use serde::{Deserialize, Serialize};
use std::fs;
use std::path::{Component, Path, PathBuf};

pub const MASK_FORMAT_VERSION: u32 = 1;
const RAW_MASK_ENCODING: &str = "base64_u8";
const RLE_MASK_ENCODING: &str = "base64_rle_u8";
const MAX_MASK_FILE_BYTES: u64 = MAX_MASK_PIXELS * 4 / 3 + 1_048_576;

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
        let rle = encode_rle(bitmap.coverage());
        let (encoding, bytes) = if rle.len() < bitmap.coverage().len() {
            (RLE_MASK_ENCODING, rle.as_slice())
        } else {
            (RAW_MASK_ENCODING, bitmap.coverage())
        };
        Self {
            version: MASK_FORMAT_VERSION,
            width: bitmap.width(),
            height: bitmap.height(),
            encoding: encoding.into(),
            data: STANDARD_NO_PAD.encode(bytes),
            checksum: checksum(bitmap.coverage()),
        }
    }

    pub fn decode(&self) -> Result<MaskBitmap, AppError> {
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
        let encoded = STANDARD_NO_PAD
            .decode(self.data.as_bytes())
            .map_err(|_| AppError::InvalidMask("mask data is not valid base64".into()))?;
        let decoded = if self.encoding == RLE_MASK_ENCODING {
            decode_rle(&encoded, expected)?
        } else {
            encoded
        };
        if decoded.len() != expected {
            return Err(AppError::InvalidMask(
                "decoded mask data does not match its dimensions".into(),
            ));
        }
        if checksum(&decoded) != self.checksum {
            return Err(AppError::InvalidMask(
                "mask integrity checksum does not match".into(),
            ));
        }
        MaskBitmap::from_coverage(self.width, self.height, decoded)
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
        self.mask.decode()
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
    let temporary = temporary_path(path)?;
    image::save_buffer_with_format(
        &temporary,
        mask.coverage(),
        mask.width(),
        mask.height(),
        image::ColorType::L8,
        ImageFormat::Png,
    )
    .map_err(|error| AppError::MaskIo(error.to_string()))?;
    replace_file(&temporary, path)?;
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

fn grayscale_to_mask(image: DynamicImage) -> Result<MaskBitmap, AppError> {
    let grayscale = image.to_luma8();
    MaskBitmap::from_coverage(grayscale.width(), grayscale.height(), grayscale.into_raw())
}

fn checksum(data: &[u8]) -> String {
    let mut hash = 0xcbf29ce484222325_u64;
    for byte in data {
        hash ^= u64::from(*byte);
        hash = hash.wrapping_mul(0x100000001b3);
    }
    format!("fnv1a64:{hash:016x}")
}

fn encode_rle(data: &[u8]) -> Vec<u8> {
    if data.is_empty() {
        return Vec::new();
    }
    let mut encoded = Vec::new();
    let mut value = data[0];
    let mut count = 1_u32;
    for &next in &data[1..] {
        if next == value && count < u32::MAX {
            count += 1;
        } else {
            encoded.push(value);
            encoded.extend_from_slice(&count.to_le_bytes());
            value = next;
            count = 1;
        }
    }
    encoded.push(value);
    encoded.extend_from_slice(&count.to_le_bytes());
    encoded
}

fn decode_rle(encoded: &[u8], expected: usize) -> Result<Vec<u8>, AppError> {
    if encoded.is_empty() || encoded.len() % 5 != 0 {
        return Err(AppError::InvalidMask(
            "run-length mask data is malformed".into(),
        ));
    }
    let mut decoded = Vec::with_capacity(expected);
    for chunk in encoded.chunks_exact(5) {
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
    }
    if decoded.len() != expected {
        return Err(AppError::InvalidMask(
            "run-length mask data does not match its dimensions".into(),
        ));
    }
    Ok(decoded)
}

fn atomic_write(path: &Path, bytes: &[u8]) -> Result<(), AppError> {
    let temporary = temporary_path(path)?;
    fs::write(&temporary, bytes).map_err(map_io)?;
    replace_file(&temporary, path)
}

fn temporary_path(path: &Path) -> Result<PathBuf, AppError> {
    let parent = path
        .parent()
        .ok_or_else(|| AppError::MaskIo("mask path has no parent folder".into()))?;
    let name = path
        .file_name()
        .and_then(|value| value.to_str())
        .ok_or_else(|| AppError::MaskIo("mask filename is invalid".into()))?;
    Ok(parent.join(format!(".{name}.{}.tmp", std::process::id())))
}

fn replace_file(temporary: &Path, destination: &Path) -> Result<(), AppError> {
    if destination.exists() {
        fs::remove_file(destination).map_err(map_io)?;
    }
    if let Err(error) = fs::rename(temporary, destination) {
        let _ = fs::remove_file(temporary);
        return Err(map_io(error));
    }
    Ok(())
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
