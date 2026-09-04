//! Safe RAW-source discovery and non-destructive development contracts.
//!
//! This module intentionally stops at a trustable source boundary.  The
//! existing application does not yet bundle a RAW decoder, so recognised
//! camera extensions are reported as `RecognizedDecoderUnavailable` rather
//! than being routed through the 8-bit image loader or a shell command.  A
//! future decoder can implement the same bounded metadata contract without
//! changing the project/layer model.

pub mod demosaic;
pub mod develop;
pub mod dng;
pub mod ljpeg;
pub mod tiff;

use crate::color::DevelopmentParameters;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::fs::{self, File};
use std::io::{self, Read};
use std::path::{Path, PathBuf};

pub const RAW_MAX_FILE_BYTES: u64 = 750 * 1024 * 1024;
pub const RAW_MAX_HEADER_BYTES: usize = 64 * 1024;
pub const RAW_MAX_PIXELS: u64 = 40_000_000;

/// Camera RAW families that the planned backend can identify by extension.
/// Recognition is not a claim that this build can decode the format.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "UPPERCASE")]
pub enum RawFormat {
    Dng,
    Cr2,
    Cr3,
    Nef,
    Arw,
    Raf,
    Orf,
    Rw2,
    Pef,
    Srw,
    Mrw,
    Erf,
    Mef,
    Iiq,
    #[serde(rename = "3FR")]
    ThreeFr,
    Mos,
    Ari,
}

impl RawFormat {
    pub const PRIORITY: [Self; 8] = [
        Self::Dng,
        Self::Cr2,
        Self::Cr3,
        Self::Nef,
        Self::Arw,
        Self::Raf,
        Self::Orf,
        Self::Rw2,
    ];

    pub fn from_extension(extension: &str) -> Option<Self> {
        match extension
            .trim_start_matches('.')
            .to_ascii_lowercase()
            .as_str()
        {
            "dng" => Some(Self::Dng),
            "cr2" => Some(Self::Cr2),
            "cr3" => Some(Self::Cr3),
            "nef" => Some(Self::Nef),
            "arw" | "srf" | "sr2" => Some(Self::Arw),
            "raf" => Some(Self::Raf),
            "orf" => Some(Self::Orf),
            "rw2" => Some(Self::Rw2),
            "pef" => Some(Self::Pef),
            "srw" => Some(Self::Srw),
            "mrw" => Some(Self::Mrw),
            "erf" => Some(Self::Erf),
            "mef" => Some(Self::Mef),
            "iiq" => Some(Self::Iiq),
            "3fr" => Some(Self::ThreeFr),
            "mos" => Some(Self::Mos),
            "ari" => Some(Self::Ari),
            _ => None,
        }
    }

    pub fn extension(self) -> &'static str {
        match self {
            Self::Dng => "dng",
            Self::Cr2 => "cr2",
            Self::Cr3 => "cr3",
            Self::Nef => "nef",
            Self::Arw => "arw",
            Self::Raf => "raf",
            Self::Orf => "orf",
            Self::Rw2 => "rw2",
            Self::Pef => "pef",
            Self::Srw => "srw",
            Self::Mrw => "mrw",
            Self::Erf => "erf",
            Self::Mef => "mef",
            Self::Iiq => "iiq",
            Self::ThreeFr => "3fr",
            Self::Mos => "mos",
            Self::Ari => "ari",
        }
    }

    pub fn is_priority(self) -> bool {
        Self::PRIORITY.contains(&self)
    }
}

/// Why a file cannot be passed to a decoder in this build.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum RawSupport {
    RecognizedDecoderUnavailable,
    Unsupported,
}

/// The decoder-independent result returned by a safe RAW inspection.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RawInspection {
    pub filename: String,
    pub format: Option<RawFormat>,
    pub support: RawSupport,
    pub file_size: u64,
    pub sha256: String,
    pub tiff_header: bool,
    pub dimensions: Option<(u32, u32)>,
    pub decoder: Option<String>,
}

/// Camera metadata that can be shown locally when a decoder supplies it. Every
/// field is optional because metadata is not required to decode a photograph.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct RawCaptureMetadata {
    pub manufacturer: Option<String>,
    pub model: Option<String>,
    pub lens: Option<String>,
    pub focal_length_mm: Option<f32>,
    pub aperture: Option<f32>,
    pub shutter_speed_seconds: Option<f32>,
    pub iso: Option<u32>,
    pub capture_time: Option<String>,
    pub orientation: Option<u16>,
    pub exposure_compensation: Option<f32>,
    pub white_balance_multipliers: Option<[f32; 3]>,
}

impl RawCaptureMetadata {
    pub fn validate(&self) -> Result<(), RawError> {
        for value in [
            self.manufacturer.as_deref(),
            self.model.as_deref(),
            self.lens.as_deref(),
            self.capture_time.as_deref(),
        ]
        .into_iter()
        .flatten()
        {
            if value.trim().is_empty()
                || value.chars().count() > 256
                || value.chars().any(char::is_control)
            {
                return Err(RawError::InvalidMetadata(
                    "camera metadata text is empty, too long, or contains control characters"
                        .into(),
                ));
            }
        }
        for value in [
            self.focal_length_mm,
            self.aperture,
            self.shutter_speed_seconds,
            self.exposure_compensation,
        ]
        .into_iter()
        .flatten()
        {
            if !value.is_finite() || value < 0.0 {
                return Err(RawError::InvalidMetadata(
                    "numeric camera metadata must be finite and nonnegative".into(),
                ));
            }
        }
        if self
            .orientation
            .is_some_and(|value| !(1..=8).contains(&value))
        {
            return Err(RawError::InvalidMetadata(
                "EXIF orientation must be between 1 and 8".into(),
            ));
        }
        if self.white_balance_multipliers.is_some_and(|values| {
            values
                .iter()
                .any(|value| !value.is_finite() || *value <= 0.0)
        }) {
            return Err(RawError::InvalidMetadata(
                "white-balance metadata must contain positive finite values".into(),
            ));
        }
        Ok(())
    }
}

/// A source reference suitable for a future project manifest.  The absolute
/// path is deliberately not serialised or returned by the inspection command;
/// callers can store a user-approved local path alongside the hash.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct RawSourceReference {
    pub filename: String,
    pub format: RawFormat,
    pub file_size: u64,
    pub sha256: String,
    pub width: u32,
    pub height: u32,
}

impl RawSourceReference {
    pub fn validate(&self) -> Result<(), RawError> {
        if self.filename.trim().is_empty() || self.filename.chars().count() > 255 {
            return Err(RawError::InvalidMetadata(
                "filename is empty or too long".into(),
            ));
        }
        validate_dimensions(self.width, self.height)?;
        if self.file_size == 0 || self.file_size > RAW_MAX_FILE_BYTES || self.sha256.len() != 64 {
            return Err(RawError::InvalidMetadata(
                "source size or hash is invalid".into(),
            ));
        }
        if !self.sha256.bytes().all(|byte| byte.is_ascii_hexdigit()) {
            return Err(RawError::InvalidMetadata(
                "source hash is not hexadecimal".into(),
            ));
        }
        Ok(())
    }
}

/// Future RAW decoder capabilities are explicit rather than inferred from a
/// manifest.  `backend_available` is false in the current build.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RawDecoderCapabilities {
    pub backend: String,
    pub backend_available: bool,
    pub native_dependencies: bool,
    pub license: String,
    pub formats: Vec<RawFormat>,
    pub demosaic_algorithms: Vec<String>,
}

pub fn decoder_capabilities() -> RawDecoderCapabilities {
    RawDecoderCapabilities {
        backend: "none (rawloader evaluation pending)".into(),
        backend_available: false,
        native_dependencies: false,
        license: "not applicable; no decoder bundled".into(),
        formats: Vec::new(),
        demosaic_algorithms: Vec::new(),
    }
}

/// Errors produced while inspecting an untrusted RAW file.
#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum RawError {
    #[error("the RAW path is not a regular local file")]
    InvalidPath,
    #[error("the RAW file is too large to inspect safely")]
    FileTooLarge,
    #[error("the RAW file could not be read: {0}")]
    Io(String),
    #[error("the RAW file is malformed: {0}")]
    Malformed(String),
    #[error("RAW metadata is invalid: {0}")]
    InvalidMetadata(String),
    /// The file was read successfully but describes something this build does
    /// not implement. Kept separate from `Malformed` so the interface can say
    /// "PhotoForge cannot develop this yet" rather than "your file is broken".
    #[error("this RAW file is not supported: {0}")]
    Unsupported(String),
}

impl From<io::Error> for RawError {
    fn from(error: io::Error) -> Self {
        Self::Io(error.to_string())
    }
}

/// Inspect a path without invoking a decoder or modifying the source.  The
/// whole-file SHA-256 is calculated in bounded chunks so a project can later
/// detect a missing or changed linked source.
pub fn inspect_raw_path(path: &Path) -> Result<RawInspection, RawError> {
    validate_raw_path(path)?;
    let canonical = fs::canonicalize(path).map_err(RawError::from)?;
    let metadata = fs::metadata(&canonical).map_err(RawError::from)?;
    if !metadata.is_file() {
        return Err(RawError::InvalidPath);
    }
    if metadata.len() == 0 {
        return Err(RawError::Malformed("the RAW file is empty".into()));
    }
    if metadata.len() > RAW_MAX_FILE_BYTES {
        return Err(RawError::FileTooLarge);
    }

    let format = canonical
        .extension()
        .and_then(|value| value.to_str())
        .and_then(RawFormat::from_extension);
    let mut file = File::open(&canonical)?;
    let mut header = vec![0_u8; RAW_MAX_HEADER_BYTES.min(metadata.len() as usize)];
    let header_len = file.read(&mut header)?;
    header.truncate(header_len);
    let tiff_header = is_tiff_header(&header);
    if format == Some(RawFormat::Dng) && !tiff_header {
        return Err(RawError::Malformed(
            "a DNG must begin with a little- or big-endian TIFF header".into(),
        ));
    }
    let sha256 = sha256_file(&canonical)?;
    let filename = canonical
        .file_name()
        .and_then(|value| value.to_str())
        .unwrap_or("raw image")
        .to_string();
    Ok(RawInspection {
        filename,
        support: if format.is_some() {
            RawSupport::RecognizedDecoderUnavailable
        } else {
            RawSupport::Unsupported
        },
        format,
        file_size: metadata.len(),
        sha256,
        tiff_header,
        dimensions: None,
        decoder: None,
    })
}

fn sha256_file(path: &Path) -> Result<String, RawError> {
    let mut file = File::open(path)?;
    let mut digest = Sha256::new();
    let mut buffer = [0_u8; 64 * 1024];
    loop {
        let count = file.read(&mut buffer)?;
        if count == 0 {
            break;
        }
        digest.update(&buffer[..count]);
    }
    Ok(format!("{:x}", digest.finalize()))
}

fn is_tiff_header(bytes: &[u8]) -> bool {
    bytes
        .get(..4)
        .is_some_and(|header| header == [b'I', b'I', 42, 0] || header == [b'M', b'M', 0, 42])
}

pub fn validate_dimensions(width: u32, height: u32) -> Result<(), RawError> {
    if width == 0 || height == 0 {
        return Err(RawError::InvalidMetadata(
            "dimensions must be nonzero".into(),
        ));
    }
    let pixels = u64::from(width)
        .checked_mul(u64::from(height))
        .ok_or_else(|| RawError::InvalidMetadata("dimensions overflow".into()))?;
    if pixels > RAW_MAX_PIXELS {
        return Err(RawError::InvalidMetadata(format!(
            "dimensions exceed the {RAW_MAX_PIXELS}-pixel limit"
        )));
    }
    Ok(())
}

/// A linked RAW document state.  This is intentionally serialisable on its
/// own so project-schema work can adopt it without baking pixels or changing
/// legacy projects.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct RawDevelopmentDocument {
    pub source: RawSourceReference,
    pub parameters: DevelopmentParameters,
    pub decoder_id: String,
    pub decoder_version: String,
    pub working_color_space: crate::color::WorkingColorSpace,
}

impl RawDevelopmentDocument {
    pub fn validate(&self) -> Result<(), RawError> {
        self.source.validate()?;
        if self.decoder_id.trim().is_empty() || self.decoder_id.len() > 128 {
            return Err(RawError::InvalidMetadata("decoder id is invalid".into()));
        }
        if self.decoder_version.trim().is_empty() || self.decoder_version.len() > 64 {
            return Err(RawError::InvalidMetadata(
                "decoder version is invalid".into(),
            ));
        }
        self.parameters
            .validate()
            .map_err(|error| RawError::InvalidMetadata(error.to_string()))
    }
}

/// Returns an absolute path only for internal workers; this helper avoids
/// accidentally serialising the user's machine-specific source location.
pub fn canonical_source_path(path: &Path) -> Result<PathBuf, RawError> {
    validate_raw_path(path)?;
    let canonical = fs::canonicalize(path).map_err(RawError::from)?;
    if !canonical.is_file() {
        return Err(RawError::InvalidPath);
    }
    Ok(canonical)
}

fn validate_raw_path(path: &Path) -> Result<(), RawError> {
    let text = path.to_string_lossy();
    if !path.is_absolute()
        || text.starts_with("\\\\")
        || text.starts_with("//")
        || text.contains("://")
        || path
            .components()
            .any(|component| component == std::path::Component::ParentDir)
    {
        return Err(RawError::InvalidPath);
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;
    use tempfile::NamedTempFile;

    #[test]
    fn recognises_priority_and_alias_extensions_case_insensitively() {
        assert_eq!(RawFormat::from_extension(".CR3"), Some(RawFormat::Cr3));
        assert_eq!(RawFormat::from_extension("SR2"), Some(RawFormat::Arw));
        assert!(RawFormat::Cr3.is_priority());
        assert_eq!(RawFormat::Dng.extension(), "dng");
        assert_eq!(RawFormat::from_extension("txt"), None);
    }

    #[test]
    fn inspection_hashes_without_decoding_or_mutating_source() {
        let mut file = NamedTempFile::with_suffix(".NEF").unwrap();
        file.write_all(b"not a real decoder input").unwrap();
        let before = fs::read(file.path()).unwrap();
        let inspection = inspect_raw_path(file.path()).unwrap();
        assert_eq!(inspection.format, Some(RawFormat::Nef));
        assert_eq!(inspection.support, RawSupport::RecognizedDecoderUnavailable);
        assert_eq!(inspection.sha256.len(), 64);
        assert_eq!(fs::read(file.path()).unwrap(), before);
        assert!(inspection.dimensions.is_none());
        assert!(inspection.decoder.is_none());
    }

    #[test]
    fn dng_requires_a_tiff_header_and_random_extensions_are_unsupported() {
        let mut dng = NamedTempFile::with_suffix(".dng").unwrap();
        dng.write_all(b"bad").unwrap();
        assert!(matches!(
            inspect_raw_path(dng.path()),
            Err(RawError::Malformed(_))
        ));

        let mut unknown = NamedTempFile::with_suffix(".bin").unwrap();
        unknown.write_all(b"bytes").unwrap();
        assert_eq!(
            inspect_raw_path(unknown.path()).unwrap().support,
            RawSupport::Unsupported
        );
        assert_eq!(
            inspect_raw_path(Path::new("relative.nef")),
            Err(RawError::InvalidPath)
        );
    }

    #[test]
    fn source_reference_validation_rejects_bad_dimensions_and_hashes() {
        let source = RawSourceReference {
            filename: "capture.nef".into(),
            format: RawFormat::Nef,
            file_size: 10,
            sha256: "0".repeat(64),
            width: 4,
            height: 4,
        };
        assert!(source.validate().is_ok());
        let mut invalid = source.clone();
        invalid.width = 0;
        assert!(invalid.validate().is_err());
        invalid = source;
        invalid.sha256 = "not-a-hash".into();
        assert!(invalid.validate().is_err());
    }

    #[test]
    fn camera_metadata_is_optional_but_numeric_fields_are_checked() {
        assert!(RawCaptureMetadata::default().validate().is_ok());
        let metadata = RawCaptureMetadata {
            orientation: Some(9),
            ..RawCaptureMetadata::default()
        };
        assert!(metadata.validate().is_err());
        let metadata = RawCaptureMetadata {
            aperture: Some(f32::NAN),
            ..RawCaptureMetadata::default()
        };
        assert!(metadata.validate().is_err());
    }

    #[test]
    fn decoder_capabilities_are_explicitly_unavailable() {
        let capabilities = decoder_capabilities();
        assert!(!capabilities.backend_available);
        assert!(capabilities.formats.is_empty());
        assert!(capabilities.demosaic_algorithms.is_empty());
    }

    #[test]
    fn raw_development_document_round_trips_and_validates() {
        let document = RawDevelopmentDocument {
            source: RawSourceReference {
                filename: "capture.dng".into(),
                format: RawFormat::Dng,
                file_size: 1,
                sha256: "a".repeat(64),
                width: 2,
                height: 2,
            },
            parameters: DevelopmentParameters::default(),
            decoder_id: "rawloader".into(),
            decoder_version: "pending".into(),
            working_color_space: crate::color::WorkingColorSpace::LinearSrgb,
        };
        document.validate().unwrap();
        let json = serde_json::to_string(&document).unwrap();
        assert_eq!(
            serde_json::from_str::<RawDevelopmentDocument>(&json).unwrap(),
            document
        );
    }
}
