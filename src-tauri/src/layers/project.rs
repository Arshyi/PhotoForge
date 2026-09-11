use super::model::{Layer, LayerDocument, LayerMask, MAX_CANVAS_DIMENSION};
use crate::color::{FloatImage, FloatRgba};
use crate::domain::EditOperation;
use crate::error::AppError;
use crate::mask::{MaskBitmap, MaskSnapshot};
use crate::pixel::{DocumentPrecision, PixelBuffer, PixelFormat};
use image::codecs::png::PngEncoder;
use image::{ExtendedColorType, ImageEncoder, ImageFormat, ImageReader, Limits, RgbaImage};
use serde::{Deserialize, Serialize};
use std::fs::{self, File};
use std::io::{BufWriter, Cursor, Read, Write};
use std::path::{Component, Path};
use tempfile::{Builder as TempFileBuilder, NamedTempFile};

/// File extension for an editable PhotoForge project.
pub const PROJECT_EXTENSION: &str = "photoforge";
/// Container magic. The trailing CR/LF pair makes a file mangled by a text-mode
/// transfer fail immediately instead of decoding into nonsense.
pub const PROJECT_MAGIC: &[u8; 8] = b"PFORGE\r\n";
pub const PROJECT_FORMAT_VERSION: u32 = 2;

pub const MAX_PROJECT_BYTES: u64 = 1_073_741_824;
pub const MAX_MANIFEST_BYTES: u64 = 33_554_432;
pub const MAX_PROJECT_ENTRIES: usize = 4_096;
pub const MAX_ENTRY_BYTES: u64 = crate::resources::MAX_WORKING_IMAGE_BYTES;
pub const MAX_ENTRY_NAME_CHARS: usize = 128;
const MAX_DECODED_BYTES: u64 = 256 * 1024 * 1024;

const ENCODING_RAW: u8 = 0;
const ENCODING_PNG: u8 = 1;
const ENCODING_LINEAR_F32: u8 = 2;

fn legacy_pixel_format() -> PixelFormat {
    PixelFormat::SRGBA8
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ProjectPixelEntry {
    pub pixel_id: String,
    pub entry: String,
    pub width: u32,
    pub height: u32,
    #[serde(default = "legacy_pixel_format")]
    pub format: PixelFormat,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ProjectMaskEntry {
    pub layer_id: String,
    pub entry: String,
    pub width: u32,
    pub height: u32,
    pub enabled: bool,
    #[serde(default)]
    pub inverted: bool,
}

/// The JSON manifest at the head of the container.
///
/// The layer tree is stored here with masks removed; coverage bitmaps live in
/// their own PNG entries so the manifest stays small enough to parse eagerly.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ProjectManifest {
    pub format_version: u32,
    pub application: String,
    pub application_version: String,
    #[serde(default)]
    pub created_at: String,
    #[serde(default)]
    pub modified_at: String,
    pub document: LayerDocument,
    #[serde(default)]
    pub masks: Vec<ProjectMaskEntry>,
    #[serde(default)]
    pub pixels: Vec<ProjectPixelEntry>,
    /// The document-level operation pipeline that renders on top of the
    /// composite: global adjustments plus crop, straighten, and perspective.
    #[serde(default)]
    pub document_operations: Vec<EditOperation>,
}

/// Everything needed to restore an editing session from a project file.
pub struct LoadedProject {
    pub document: LayerDocument,
    pub document_operations: Vec<EditOperation>,
    pub pixels: Vec<(String, RgbaImage)>,
    pub linear_pixels: Vec<(String, FloatImage)>,
    pub application_version: String,
    pub created_at: String,
    pub modified_at: String,
}

struct Entry {
    name: String,
    encoding: u8,
    payload: Vec<u8>,
}

// Decode entries borrow the bounded container instead of duplicating up to a
// gigabyte of payload while the reconstructed pixel buffers are being allocated.
struct ReadEntry<'a> {
    name: String,
    encoding: u8,
    payload: &'a [u8],
}

fn fnv1a64(data: &[u8]) -> u64 {
    let mut hash = 0xcbf29ce484222325_u64;
    for byte in data {
        hash ^= u64::from(*byte);
        hash = hash.wrapping_mul(0x100000001b3);
    }
    hash
}

/// Entry names are container-internal labels, never filesystem paths. They are
/// still validated as if they were: no absolute roots, no parent traversal, no
/// backslashes, no empty or dotted segments, and a restricted character set.
fn valid_entry_name(name: &str) -> bool {
    if name.is_empty() || name.len() > MAX_ENTRY_NAME_CHARS {
        return false;
    }
    if name.starts_with('/') || name.ends_with('/') || name.contains('\\') || name.contains("//") {
        return false;
    }
    name.split('/').all(|segment| {
        !segment.is_empty()
            && segment != "."
            && segment != ".."
            && segment.bytes().all(|byte| {
                byte.is_ascii_alphanumeric() || byte == b'.' || byte == b'-' || byte == b'_'
            })
    })
}

fn map_io(error: std::io::Error) -> AppError {
    if error.kind() == std::io::ErrorKind::PermissionDenied {
        AppError::Permission
    } else {
        AppError::ProjectIo(error.to_string())
    }
}

fn validate_local_path(path: &Path) -> Result<(), AppError> {
    let text = path.to_string_lossy();
    if !path.is_absolute()
        || text.starts_with("\\\\")
        || text.starts_with("//")
        || text.contains("://")
        || path.components().any(|part| part == Component::ParentDir)
    {
        return Err(AppError::ProjectIo(
            "project paths must be absolute local paths without parent traversal".into(),
        ));
    }
    Ok(())
}

fn encode_rgba_png(image: &RgbaImage) -> Result<Vec<u8>, AppError> {
    let mut bytes = Vec::new();
    PngEncoder::new(&mut bytes)
        .write_image(
            image.as_raw(),
            image.width(),
            image.height(),
            ExtendedColorType::Rgba8,
        )
        .map_err(|error| AppError::ProjectFormat(format!("layer encoding failed: {error}")))?;
    Ok(bytes)
}

fn encode_mask_png(mask: &MaskBitmap) -> Result<Vec<u8>, AppError> {
    let mut bytes = Vec::new();
    PngEncoder::new(&mut bytes)
        .write_image(
            mask.coverage(),
            mask.width(),
            mask.height(),
            ExtendedColorType::L8,
        )
        .map_err(|error| AppError::ProjectFormat(format!("mask encoding failed: {error}")))?;
    Ok(bytes)
}

/// Decodes an embedded PNG with the same decoder ceilings the image importer
/// uses, so a small entry cannot expand into an enormous allocation.
fn decode_png(bytes: &[u8], expected: (u32, u32)) -> Result<image::DynamicImage, AppError> {
    let mut reader = ImageReader::new(Cursor::new(bytes));
    reader.set_format(ImageFormat::Png);
    let mut limits = Limits::default();
    limits.max_image_width = Some(MAX_CANVAS_DIMENSION);
    limits.max_image_height = Some(MAX_CANVAS_DIMENSION);
    limits.max_alloc = Some(MAX_DECODED_BYTES);
    reader.limits(limits);
    let decoded = reader.decode().map_err(|error| {
        AppError::ProjectFormat(format!("embedded image is unreadable: {error}"))
    })?;
    if (decoded.width(), decoded.height()) != expected {
        return Err(AppError::ProjectFormat(format!(
            "embedded image is {}x{} but the manifest declares {}x{}",
            decoded.width(),
            decoded.height(),
            expected.0,
            expected.1
        )));
    }
    Ok(decoded)
}

/// Removes every mask from a copy of the tree, returning the manifest entries
/// that describe where each one was stored.
fn detach_masks(document: &mut LayerDocument) -> Vec<(ProjectMaskEntry, LayerMask)> {
    fn walk(layers: &mut [Layer], detached: &mut Vec<(ProjectMaskEntry, LayerMask)>) {
        for layer in layers {
            if let Some(mask) = layer.mask.take() {
                detached.push((
                    ProjectMaskEntry {
                        layer_id: layer.id.clone(),
                        entry: format!("masks/{}.png", layer.id),
                        width: mask.snapshot.width,
                        height: mask.snapshot.height,
                        enabled: mask.enabled,
                        inverted: mask.inverted,
                    },
                    mask,
                ));
            }
            if let Some(children) = layer.children_mut() {
                walk(children, detached);
            }
        }
    }
    let mut detached = Vec::new();
    walk(&mut document.layers, &mut detached);
    // Each source is stored once, independent of how many instances refer to
    // it. Layer IDs are globally unique, including inside source trees.
    let mut source_ids: Vec<_> = document.smart_sources.keys().cloned().collect();
    source_ids.sort_unstable();
    for id in source_ids {
        walk(
            &mut document
                .smart_sources
                .get_mut(&id)
                .expect("known source")
                .layers,
            &mut detached,
        );
    }
    detached
}

fn attach_mask(layers: &mut [Layer], layer_id: &str, mask: LayerMask) -> bool {
    for layer in layers {
        if layer.id == layer_id {
            layer.mask = Some(mask);
            return true;
        }
        if let Some(children) = layer.children_mut() {
            if attach_mask(children, layer_id, mask.clone()) {
                return true;
            }
        }
    }
    false
}

/// Serializes a project into the container byte layout.
pub fn encode_project(
    document: &LayerDocument,
    document_operations: &[EditOperation],
    pixels: &[(String, &RgbaImage)],
    application_version: &str,
    created_at: &str,
    modified_at: &str,
) -> Result<Vec<u8>, AppError> {
    let sources: Vec<_> = pixels
        .iter()
        .map(|(id, image)| (id.clone(), ProjectPixelRef::Encoded(image)))
        .collect();
    encode_project_sources(
        document,
        document_operations,
        &sources,
        application_version,
        created_at,
        modified_at,
    )
}

enum ProjectPixelRef<'a> {
    Encoded(&'a RgbaImage),
    Linear(&'a FloatImage),
}

pub fn encode_project_typed(
    document: &LayerDocument,
    document_operations: &[EditOperation],
    pixels: &[(String, PixelBuffer)],
    application_version: &str,
    created_at: &str,
    modified_at: &str,
) -> Result<Vec<u8>, AppError> {
    let sources: Vec<_> = pixels
        .iter()
        .map(|(id, image)| {
            (
                id.clone(),
                match image {
                    PixelBuffer::EncodedSrgba8(image) => ProjectPixelRef::Encoded(image),
                    PixelBuffer::LinearRgbaF32(image) => ProjectPixelRef::Linear(image),
                },
            )
        })
        .collect();
    encode_project_sources(
        document,
        document_operations,
        &sources,
        application_version,
        created_at,
        modified_at,
    )
}

fn encode_project_sources(
    document: &LayerDocument,
    document_operations: &[EditOperation],
    pixels: &[(String, ProjectPixelRef<'_>)],
    application_version: &str,
    created_at: &str,
    modified_at: &str,
) -> Result<Vec<u8>, AppError> {
    document.validate()?;
    for operation in document_operations {
        operation.validate()?;
    }

    let mut entries: Vec<Entry> = Vec::new();
    let mut pixel_entries = Vec::new();
    let referenced = document.referenced_pixel_ids();
    let mut source_bytes = 0_u64;
    for (id, image) in pixels.iter().filter(|(id, _)| referenced.contains(id)) {
        let (w, h, bpp) = match image {
            ProjectPixelRef::Encoded(image) => (image.width(), image.height(), 4),
            ProjectPixelRef::Linear(image) => (image.width(), image.height(), 16),
        };
        super::model::validate_dimensions(w, h)?;
        if document.iter_all().any(|layer| {
            layer.pixel_id() == Some(id.as_str()) && layer.pixel_dimensions() != Some((w, h))
        }) {
            return Err(AppError::ProjectFormat(
                "layer dimensions disagree with stored pixels".into(),
            ));
        }
        source_bytes = source_bytes
            .checked_add(u64::from(w) * u64::from(h) * bpp)
            .ok_or(AppError::OutOfMemoryRisk)?;
    }
    let mask_bytes = document
        .iter_all()
        .filter_map(|l| l.mask.as_ref())
        .try_fold(0_u64, |total, mask| {
            total
                .checked_add(u64::from(mask.snapshot.width) * u64::from(mask.snapshot.height) * 10)
                .ok_or(AppError::OutOfMemoryRisk)
        })?;
    crate::resources::ResourceEstimate::new(
        source_bytes,
        source_bytes
            .checked_mul(2)
            .ok_or(AppError::OutOfMemoryRisk)?,
        mask_bytes,
        64 * 1024 * 1024,
    )?;
    let mut stripped = document.clone();
    let detached = detach_masks(&mut stripped);
    for pixel_id in &referenced {
        let image = pixels
            .iter()
            .find(|(id, _)| id == pixel_id)
            .map(|(_, image)| image)
            .ok_or_else(|| AppError::LayerPixelsMissing(pixel_id.clone()))?;
        let (width, height, format, encoding, payload, extension) = match image {
            ProjectPixelRef::Encoded(image) => (
                image.width(),
                image.height(),
                PixelFormat::SRGBA8,
                ENCODING_PNG,
                encode_rgba_png(image)?,
                "png",
            ),
            ProjectPixelRef::Linear(image) => {
                image.validate()?;
                let len = image
                    .pixels()
                    .len()
                    .checked_mul(16)
                    .ok_or(AppError::OutOfMemoryRisk)?;
                if len as u64 > MAX_ENTRY_BYTES {
                    return Err(AppError::ProjectTooLarge {
                        bytes: len as u64,
                        limit: MAX_ENTRY_BYTES,
                    });
                }
                let mut payload = Vec::new();
                payload
                    .try_reserve_exact(len)
                    .map_err(|_| AppError::OutOfMemoryRisk)?;
                for p in image.pixels() {
                    for c in [p.red, p.green, p.blue, p.alpha] {
                        payload.extend_from_slice(&c.to_le_bytes());
                    }
                }
                (
                    image.width(),
                    image.height(),
                    PixelFormat::LINEAR_RGBA_F32,
                    ENCODING_LINEAR_F32,
                    payload,
                    "f32le",
                )
            }
        };
        let entry = format!("layers/{pixel_id}.{extension}");
        if !valid_entry_name(&entry) {
            return Err(AppError::ProjectFormat(format!(
                "{pixel_id} cannot be stored under a safe entry name"
            )));
        }
        pixel_entries.push(ProjectPixelEntry {
            pixel_id: pixel_id.clone(),
            entry: entry.clone(),
            width,
            height,
            format,
        });
        entries.push(Entry {
            name: entry,
            encoding,
            payload,
        });
    }

    let mut mask_entries = Vec::new();
    for (entry, mask) in detached {
        if !valid_entry_name(&entry.entry) {
            return Err(AppError::ProjectFormat(format!(
                "{} cannot be stored under a safe entry name",
                entry.layer_id
            )));
        }
        let bitmap = mask.snapshot.decode()?;
        entries.push(Entry {
            name: entry.entry.clone(),
            encoding: ENCODING_PNG,
            payload: encode_mask_png(&bitmap)?,
        });
        mask_entries.push(entry);
    }

    let manifest = ProjectManifest {
        format_version: PROJECT_FORMAT_VERSION,
        application: "PhotoForge".into(),
        application_version: application_version.to_string(),
        created_at: created_at.to_string(),
        modified_at: modified_at.to_string(),
        document: stripped,
        masks: mask_entries,
        pixels: pixel_entries,
        document_operations: document_operations.to_vec(),
    };
    let manifest_bytes = serde_json::to_vec(&manifest)
        .map_err(|error| AppError::ProjectFormat(format!("manifest encoding failed: {error}")))?;
    if manifest_bytes.len() as u64 > MAX_MANIFEST_BYTES {
        return Err(AppError::ProjectTooLarge {
            bytes: manifest_bytes.len() as u64,
            limit: MAX_MANIFEST_BYTES,
        });
    }
    if entries.len() > MAX_PROJECT_ENTRIES {
        return Err(AppError::ProjectFormat(format!(
            "a project may contain at most {MAX_PROJECT_ENTRIES} stored entries"
        )));
    }

    let required = entries
        .iter()
        .try_fold(
            PROJECT_MAGIC.len() as u64 + 4 + 8 + manifest_bytes.len() as u64 + 4 + 8,
            |sum, entry| {
                sum.checked_add(
                    2 + entry.name.len() as u64 + 1 + 8 + 8 + entry.payload.len() as u64,
                )
            },
        )
        .ok_or(AppError::OutOfMemoryRisk)?;
    if required > MAX_PROJECT_BYTES {
        return Err(AppError::ProjectTooLarge {
            bytes: required,
            limit: MAX_PROJECT_BYTES,
        });
    }
    let mut bytes = Vec::new();
    bytes
        .try_reserve_exact(required as usize)
        .map_err(|_| AppError::OutOfMemoryRisk)?;
    bytes.extend_from_slice(PROJECT_MAGIC);
    bytes.extend_from_slice(&PROJECT_FORMAT_VERSION.to_le_bytes());
    bytes.extend_from_slice(&(manifest_bytes.len() as u64).to_le_bytes());
    bytes.extend_from_slice(&manifest_bytes);
    bytes.extend_from_slice(&(entries.len() as u32).to_le_bytes());
    for entry in &entries {
        if entry.payload.len() as u64 > MAX_ENTRY_BYTES {
            return Err(AppError::ProjectTooLarge {
                bytes: entry.payload.len() as u64,
                limit: MAX_ENTRY_BYTES,
            });
        }
        bytes.extend_from_slice(&(entry.name.len() as u16).to_le_bytes());
        bytes.extend_from_slice(entry.name.as_bytes());
        bytes.push(entry.encoding);
        bytes.extend_from_slice(&(entry.payload.len() as u64).to_le_bytes());
        bytes.extend_from_slice(&fnv1a64(&entry.payload).to_le_bytes());
        bytes.extend_from_slice(&entry.payload);
        if bytes.len() as u64 > MAX_PROJECT_BYTES {
            return Err(AppError::ProjectTooLarge {
                bytes: bytes.len() as u64,
                limit: MAX_PROJECT_BYTES,
            });
        }
    }
    let trailer = fnv1a64(&bytes);
    bytes.extend_from_slice(&trailer.to_le_bytes());
    Ok(bytes)
}

struct Cursor2<'a> {
    bytes: &'a [u8],
    offset: usize,
}

impl<'a> Cursor2<'a> {
    fn take(&mut self, count: usize) -> Result<&'a [u8], AppError> {
        let end = self
            .offset
            .checked_add(count)
            .ok_or_else(|| AppError::ProjectFormat("entry length overflows the file".into()))?;
        if end > self.bytes.len() {
            return Err(AppError::ProjectFormat(
                "the project file ends earlier than its structure declares".into(),
            ));
        }
        let slice = &self.bytes[self.offset..end];
        self.offset = end;
        Ok(slice)
    }

    fn u8(&mut self) -> Result<u8, AppError> {
        Ok(self.take(1)?[0])
    }

    fn u16(&mut self) -> Result<u16, AppError> {
        let bytes = self.take(2)?;
        Ok(u16::from_le_bytes([bytes[0], bytes[1]]))
    }

    fn u32(&mut self) -> Result<u32, AppError> {
        let bytes = self.take(4)?;
        Ok(u32::from_le_bytes([bytes[0], bytes[1], bytes[2], bytes[3]]))
    }

    fn u64(&mut self) -> Result<u64, AppError> {
        let bytes = self.take(8)?;
        let mut value = [0_u8; 8];
        value.copy_from_slice(bytes);
        Ok(u64::from_le_bytes(value))
    }
}

/// Parses and validates a project container.
///
/// Every structural field is bounds-checked against the bytes actually present
/// before any allocation follows it, so a truncated, padded, or hostile file is
/// rejected instead of driving a large allocation.
pub fn decode_project(bytes: &[u8]) -> Result<LoadedProject, AppError> {
    if bytes.len() as u64 > MAX_PROJECT_BYTES {
        return Err(AppError::ProjectTooLarge {
            bytes: bytes.len() as u64,
            limit: MAX_PROJECT_BYTES,
        });
    }
    if bytes.len() < PROJECT_MAGIC.len() + 4 + 8 + 4 + 8 {
        return Err(AppError::ProjectFormat(
            "the file is too short to be a PhotoForge project".into(),
        ));
    }
    if &bytes[..PROJECT_MAGIC.len()] != PROJECT_MAGIC {
        return Err(AppError::ProjectFormat(
            "the file does not start with the PhotoForge project marker".into(),
        ));
    }

    let body = &bytes[..bytes.len() - 8];
    let mut trailer = [0_u8; 8];
    trailer.copy_from_slice(&bytes[bytes.len() - 8..]);
    if u64::from_le_bytes(trailer) != fnv1a64(body) {
        return Err(AppError::ProjectFormat(
            "the project file failed its integrity check".into(),
        ));
    }

    let mut cursor = Cursor2 {
        bytes: body,
        offset: PROJECT_MAGIC.len(),
    };
    let version = cursor.u32()?;
    if !matches!(version, 1 | PROJECT_FORMAT_VERSION) {
        return Err(AppError::UnsupportedProjectVersion(version));
    }
    let manifest_len = cursor.u64()?;
    if manifest_len > MAX_MANIFEST_BYTES {
        return Err(AppError::ProjectTooLarge {
            bytes: manifest_len,
            limit: MAX_MANIFEST_BYTES,
        });
    }
    let manifest_bytes =
        cursor.take(usize::try_from(manifest_len).map_err(|_| AppError::OutOfMemoryRisk)?)?;
    let manifest: ProjectManifest = serde_json::from_slice(manifest_bytes)
        .map_err(|error| AppError::ProjectFormat(format!("manifest is unreadable: {error}")))?;
    if manifest.format_version != version {
        return Err(AppError::UnsupportedProjectVersion(manifest.format_version));
    }
    // Reject malformed or cyclic semantic trees before allocating/decompressing
    // their pixel entries. Detached masks are checked after restoration below.
    manifest.document.validate()?;
    if version == 1
        && (manifest.document.precision != DocumentPrecision::LegacySrgb8
            || manifest
                .pixels
                .iter()
                .any(|pixel| pixel.format != PixelFormat::SRGBA8))
    {
        return Err(AppError::ProjectFormat(
            "version 1 projects cannot declare float pixel semantics".into(),
        ));
    }
    if manifest.pixels.len() > super::store::MAX_STORE_BUFFERS {
        return Err(AppError::OutOfMemoryRisk);
    }
    // Admit the aggregate declared decode, not merely each PNG separately.
    // Ten mask bytes/pixel covers coverage, PNG conversion, temporary RLE and
    // retained base64 snapshots. Pixel payloads borrow the bounded container.
    let mut declared_pixels = 0_u64;
    for pixel in &manifest.pixels {
        let n = crate::resources::checked_pixels(pixel.width, pixel.height)?;
        let bpp = if pixel.format == PixelFormat::SRGBA8 {
            4
        } else {
            16
        };
        declared_pixels = declared_pixels
            .checked_add(n * bpp)
            .ok_or(AppError::OutOfMemoryRisk)?;
    }
    if declared_pixels > super::store::MAX_STORE_BYTES
        || manifest.masks.len() > super::model::MAX_LAYERS
    {
        return Err(AppError::OutOfMemoryRisk);
    }
    let mut mask_bytes = 0_u64;
    for mask in &manifest.masks {
        let n = crate::resources::checked_pixels(mask.width, mask.height)?;
        mask_bytes = mask_bytes
            .checked_add(n * 10)
            .ok_or(AppError::OutOfMemoryRisk)?;
    }
    crate::resources::ResourceEstimate::new(
        bytes.len() as u64,
        declared_pixels * 2,
        mask_bytes,
        64 * 1024 * 1024,
    )?;

    let entry_count = cursor.u32()? as usize;
    if entry_count > MAX_PROJECT_ENTRIES {
        return Err(AppError::ProjectFormat(format!(
            "the project declares {entry_count} entries; the limit is {MAX_PROJECT_ENTRIES}"
        )));
    }
    let mut entries: Vec<ReadEntry<'_>> = Vec::new();
    for _ in 0..entry_count {
        let name_len = cursor.u16()? as usize;
        if name_len > MAX_ENTRY_NAME_CHARS {
            return Err(AppError::ProjectFormat(
                "an entry name exceeds its length limit".into(),
            ));
        }
        let name_bytes = cursor.take(name_len)?;
        let name = std::str::from_utf8(name_bytes)
            .map_err(|_| AppError::ProjectFormat("an entry name is not valid UTF-8".into()))?
            .to_string();
        if !valid_entry_name(&name) {
            return Err(AppError::ProjectFormat(format!(
                "the entry name {name} is not allowed"
            )));
        }
        if entries.iter().any(|entry| entry.name == name) {
            return Err(AppError::ProjectFormat(format!(
                "the project contains two entries named {name}"
            )));
        }
        let encoding = cursor.u8()?;
        if !matches!(encoding, ENCODING_RAW | ENCODING_PNG | ENCODING_LINEAR_F32)
            || (version == 1 && encoding == ENCODING_LINEAR_F32)
        {
            return Err(AppError::ProjectFormat(format!(
                "entry {name} uses unsupported encoding {encoding}"
            )));
        }
        let payload_len = cursor.u64()?;
        if payload_len > MAX_ENTRY_BYTES {
            return Err(AppError::ProjectTooLarge {
                bytes: payload_len,
                limit: MAX_ENTRY_BYTES,
            });
        }
        let checksum = cursor.u64()?;
        let payload =
            cursor.take(usize::try_from(payload_len).map_err(|_| AppError::OutOfMemoryRisk)?)?;
        if fnv1a64(payload) != checksum {
            return Err(AppError::ProjectFormat(format!(
                "entry {name} failed its integrity check"
            )));
        }
        entries.push(ReadEntry {
            name,
            encoding,
            payload,
        });
    }
    if cursor.offset != body.len() {
        return Err(AppError::ProjectFormat(
            "the project file contains trailing data after its last entry".into(),
        ));
    }

    let find = |name: &str| -> Result<&ReadEntry<'_>, AppError> {
        entries
            .iter()
            .find(|entry| entry.name == name)
            .ok_or_else(|| AppError::ProjectFormat(format!("entry {name} is missing")))
    };

    let mut document = manifest.document;
    let mut restored_pixels = Vec::new();
    let mut linear_pixels = Vec::new();
    let mut decoded_bytes = 0_u64;
    let mut pixel_ids = std::collections::HashSet::new();
    for pixel in &manifest.pixels {
        if !pixel_ids.insert(&pixel.pixel_id) {
            return Err(AppError::ProjectFormat(
                "duplicate pixel identifiers".into(),
            ));
        }
        let entry = find(&pixel.entry)?;
        super::model::validate_dimensions(pixel.width, pixel.height)?;
        if document.iter_all().any(|layer| {
            layer.pixel_id() == Some(pixel.pixel_id.as_str())
                && layer.pixel_dimensions() != Some((pixel.width, pixel.height))
        }) {
            return Err(AppError::ProjectFormat(
                "layer geometry disagrees with the pixel manifest".into(),
            ));
        }
        let count = u64::from(pixel.width) * u64::from(pixel.height);
        let bytes = count
            .checked_mul(if pixel.format == PixelFormat::SRGBA8 {
                4
            } else {
                16
            })
            .ok_or(AppError::OutOfMemoryRisk)?;
        decoded_bytes = decoded_bytes
            .checked_add(bytes)
            .ok_or(AppError::OutOfMemoryRisk)?;
        if decoded_bytes > super::store::MAX_STORE_BYTES {
            return Err(AppError::OutOfMemoryRisk);
        }
        if entry.encoding == ENCODING_PNG && pixel.format == PixelFormat::SRGBA8 {
            let decoded = decode_png(entry.payload, (pixel.width, pixel.height))?;
            restored_pixels.push((pixel.pixel_id.clone(), decoded.to_rgba8()));
        } else if entry.encoding == ENCODING_LINEAR_F32
            && pixel.format == PixelFormat::LINEAR_RGBA_F32
            && version >= 2
        {
            if entry.payload.len() as u64 != bytes {
                return Err(AppError::ProjectFormat(
                    "float pixel payload length disagrees with its dimensions".into(),
                ));
            }
            let mut samples = Vec::new();
            samples
                .try_reserve_exact(count as usize)
                .map_err(|_| AppError::OutOfMemoryRisk)?;
            for sample in entry.payload.chunks_exact(16) {
                let channel = |offset: usize| {
                    f32::from_le_bytes(
                        sample[offset..offset + 4]
                            .try_into()
                            .expect("checked sample width"),
                    )
                };
                samples.push(FloatRgba::new(
                    channel(0),
                    channel(4),
                    channel(8),
                    channel(12),
                ));
            }
            linear_pixels.push((
                pixel.pixel_id.clone(),
                FloatImage::new(pixel.width, pixel.height, samples)?,
            ));
        } else {
            return Err(AppError::ProjectFormat(
                "pixel encoding and color/precision metadata disagree".into(),
            ));
        }
    }

    let mut masked_ids = std::collections::HashSet::new();
    for mask in &manifest.masks {
        if !masked_ids.insert(mask.layer_id.as_str()) {
            return Err(AppError::ProjectFormat(
                "duplicate mask layer identifiers".into(),
            ));
        }
        if !document
            .iter_all()
            .any(|layer| layer.id == mask.layer_id && layer.mask.is_none())
        {
            return Err(AppError::ProjectFormat(format!(
                "mask target {} is missing or already has inline coverage",
                mask.layer_id
            )));
        }
        let entry = find(&mask.entry)?;
        if entry.encoding != ENCODING_PNG {
            return Err(AppError::ProjectFormat(format!(
                "mask entry {} must be stored as PNG",
                mask.entry
            )));
        }
        let decoded = decode_png(entry.payload, (mask.width, mask.height))?;
        let coverage = decoded.to_luma8();
        let bitmap = MaskBitmap::from_coverage(mask.width, mask.height, coverage.into_raw())?;
        let restored = LayerMask {
            snapshot: MaskSnapshot::encode(&bitmap),
            enabled: mask.enabled,
            inverted: mask.inverted,
        };
        let attached = attach_mask(&mut document.layers, &mask.layer_id, restored.clone())
            || document
                .smart_sources
                .values_mut()
                .any(|source| attach_mask(&mut source.layers, &mask.layer_id, restored.clone()));
        if !attached {
            return Err(AppError::ProjectFormat(format!(
                "the manifest stores a mask for the unknown layer {}",
                mask.layer_id
            )));
        }
    }

    // Every pixel the tree references must have arrived with the file.
    for pixel_id in document.referenced_pixel_ids() {
        if !restored_pixels.iter().any(|(id, _)| *id == pixel_id)
            && !linear_pixels.iter().any(|(id, _)| *id == pixel_id)
        {
            return Err(AppError::LayerPixelsMissing(pixel_id));
        }
    }
    for operation in &manifest.document_operations {
        operation.validate()?;
    }
    document.validate()?;

    Ok(LoadedProject {
        document,
        document_operations: manifest.document_operations,
        pixels: restored_pixels,
        linear_pixels,
        application_version: manifest.application_version,
        created_at: manifest.created_at,
        modified_at: manifest.modified_at,
    })
}

/// Writes a project atomically: the bytes land in a sibling temporary file that
/// is flushed to disk and then renamed over the destination, so a failure part
/// way through never truncates an existing project.
pub fn save_project(
    path: &Path,
    document: &LayerDocument,
    document_operations: &[EditOperation],
    pixels: &[(String, &RgbaImage)],
    application_version: &str,
    created_at: &str,
    modified_at: &str,
) -> Result<u64, AppError> {
    let bytes = encode_project(
        document,
        document_operations,
        pixels,
        application_version,
        created_at,
        modified_at,
    )?;
    save_project_bytes(path, &bytes)
}

pub fn save_project_typed(
    path: &Path,
    document: &LayerDocument,
    document_operations: &[EditOperation],
    pixels: &[(String, PixelBuffer)],
    application_version: &str,
    created_at: &str,
    modified_at: &str,
) -> Result<u64, AppError> {
    let bytes = encode_project_typed(
        document,
        document_operations,
        pixels,
        application_version,
        created_at,
        modified_at,
    )?;
    save_project_bytes(path, &bytes)
}

fn save_project_bytes(path: &Path, bytes: &[u8]) -> Result<u64, AppError> {
    validate_local_path(path)?;
    if path
        .extension()
        .and_then(|value| value.to_str())
        .map(str::to_ascii_lowercase)
        .as_deref()
        != Some(PROJECT_EXTENSION)
    {
        return Err(AppError::ProjectIo(format!(
            "project files must use the .{PROJECT_EXTENSION} extension"
        )));
    }
    let parent = path
        .parent()
        .ok_or_else(|| AppError::ProjectIo("project path has no parent folder".into()))?;
    let temporary: NamedTempFile = TempFileBuilder::new()
        .prefix(".photoforge-project-")
        .suffix(".tmp")
        .tempfile_in(parent)
        .map_err(map_io)?;
    {
        let mut writer = BufWriter::new(temporary.as_file());
        writer.write_all(bytes).map_err(map_io)?;
        writer.flush().map_err(map_io)?;
    }
    temporary.as_file().sync_all().map_err(map_io)?;
    temporary
        .persist(path)
        .map_err(|error| map_io(error.error))?;
    Ok(bytes.len() as u64)
}

pub fn load_project(path: &Path) -> Result<LoadedProject, AppError> {
    validate_local_path(path)?;
    let metadata = fs::metadata(path).map_err(map_io)?;
    if !metadata.is_file() {
        return Err(AppError::ProjectIo("that path is not a file".into()));
    }
    if metadata.len() > MAX_PROJECT_BYTES {
        return Err(AppError::ProjectTooLarge {
            bytes: metadata.len(),
            limit: MAX_PROJECT_BYTES,
        });
    }
    let file = File::open(path).map_err(map_io)?;
    let mut bytes = Vec::with_capacity(metadata.len().min(MAX_PROJECT_BYTES) as usize);
    file.take(MAX_PROJECT_BYTES + 1)
        .read_to_end(&mut bytes)
        .map_err(map_io)?;
    if bytes.len() as u64 > MAX_PROJECT_BYTES {
        return Err(AppError::ProjectTooLarge {
            bytes: bytes.len() as u64,
            limit: MAX_PROJECT_BYTES,
        });
    }
    decode_project(&bytes)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::layers::model::fixtures::*;
    use crate::layers::model::{LayerContent, LAYER_SCHEMA_VERSION};
    use crate::layers::smart::{SmartLink, SmartObjectContent, SmartSource};
    use image::Rgba;

    fn image(width: u32, height: u32, value: u8) -> RgbaImage {
        RgbaImage::from_pixel(width, height, Rgba([value, value / 2, value / 3, 255]))
    }

    fn sample_document() -> LayerDocument {
        let mut document = LayerDocument::new(8, 8);
        document.layers = vec![
            pixel_layer("base", 8, 8),
            group_layer(
                "g",
                vec![
                    pixel_layer("child", 4, 4),
                    adjustment_layer("adj", EditOperation::Brightness { amount: 0.25 }),
                ],
            ),
        ];
        document.active_layer_id = Some("base".into());
        document
    }

    fn sample_pixels() -> Vec<(String, RgbaImage)> {
        vec![
            ("pxbase".to_string(), image(8, 8, 90)),
            ("pxchild".to_string(), image(4, 4, 180)),
        ]
    }

    fn borrowed(pixels: &[(String, RgbaImage)]) -> Vec<(String, &RgbaImage)> {
        pixels
            .iter()
            .map(|(id, image)| (id.clone(), image))
            .collect()
    }

    fn encode(document: &LayerDocument, pixels: &[(String, RgbaImage)]) -> Vec<u8> {
        encode_project(
            document,
            &[],
            &borrowed(pixels),
            "0.8.0",
            "2026-08-24T00:00:00Z",
            "2026-08-24T00:00:00Z",
        )
        .unwrap()
    }

    #[test]
    fn a_project_round_trips_every_documented_property() {
        let mut document = sample_document();
        document.layers[0].name = "Background".into();
        document.layers[0].opacity = 0.75;
        document.layers[0].blend_mode = crate::layers::BlendMode::Multiply;
        document.layers[0].locked = true;
        document.layers[1].visible = false;
        document.layers[1].collapsed = true;
        document.layers[0].transform = crate::layers::LayerTransform {
            translate_x: 3.0,
            translate_y: -2.0,
            scale_x: 1.5,
            scale_y: 1.5,
            rotation_degrees: 12.0,
            flip_horizontal: true,
            flip_vertical: false,
            interpolation: crate::layers::LayerInterpolation::Bilinear,
        };
        document.layers[0]
            .metadata
            .custom
            .insert("source".into(), "scanner".into());
        document.layers[0].metadata.created_at = "2026-01-01T00:00:00Z".into();

        let pixels = sample_pixels();
        let operations = vec![EditOperation::Contrast { amount: 0.1 }];
        let bytes = encode_project(
            &document,
            &operations,
            &borrowed(&pixels),
            "0.8.0",
            "created",
            "modified",
        )
        .unwrap();
        let loaded = decode_project(&bytes).unwrap();

        assert_eq!(loaded.document, document);
        assert_eq!(loaded.document_operations, operations);
        assert_eq!(loaded.application_version, "0.8.0");
        assert_eq!(loaded.created_at, "created");
        assert_eq!(loaded.modified_at, "modified");
        assert_eq!(loaded.pixels.len(), 2);
        for (id, restored) in &loaded.pixels {
            let original = pixels.iter().find(|(name, _)| name == id).unwrap();
            assert_eq!(restored.as_raw(), original.1.as_raw());
        }
    }

    #[test]
    fn smart_sources_links_masks_and_pixels_survive_a_project_round_trip() {
        let mut document = LayerDocument::new(8, 8);
        document.precision = DocumentPrecision::LinearSrgbF32;
        let mut source_layer = pixel_layer("source-pixels", 4, 4);
        let mut mask = MaskBitmap::empty(4, 4).unwrap();
        mask.set(1, 1, 200);
        source_layer.mask = Some(LayerMask {
            snapshot: MaskSnapshot::encode(&mask),
            enabled: true,
            inverted: false,
        });
        let mut source = SmartSource::new(4, 4, vec![source_layer]);
        source.link = Some(SmartLink {
            path: "C:/photos/logo.png".into(),
            digest: "a".repeat(64),
            bytes: 123,
        });
        document.smart_sources.insert("logo-source".into(), source);
        let mut instance = pixel_layer("logo-instance", 4, 4);
        instance.content = LayerContent::SmartObject {
            smart: Box::new(SmartObjectContent::new("logo-source")),
        };
        document.layers = vec![instance];
        document.active_layer_id = Some("logo-instance".into());

        let pixels = vec![("pxsource-pixels".to_string(), image(4, 4, 180))];
        let loaded = decode_project(&encode(&document, &pixels)).unwrap();

        assert_eq!(loaded.document, document);
        assert_eq!(
            loaded.document.smart_sources["logo-source"].link,
            document.smart_sources["logo-source"].link
        );
        assert_eq!(loaded.pixels.len(), 1);
        let restored_mask = loaded.document.smart_sources["logo-source"].layers[0]
            .mask
            .as_ref()
            .unwrap()
            .snapshot
            .decode()
            .unwrap();
        assert_eq!(restored_mask.get(1, 1), 200);
    }

    #[test]
    fn masks_survive_the_round_trip_with_their_flags() {
        let mut document = sample_document();
        let mut bitmap = MaskBitmap::empty(8, 8).unwrap();
        bitmap.set(0, 0, 255);
        bitmap.set(1, 0, 128);
        document.layers[0].mask = Some(LayerMask {
            snapshot: MaskSnapshot::encode(&bitmap),
            enabled: true,
            inverted: true,
        });

        let pixels = sample_pixels();
        let loaded = decode_project(&encode(&document, &pixels)).unwrap();
        let mask = loaded.document.layers[0].mask.as_ref().unwrap();
        assert!(mask.enabled && mask.inverted);
        let restored = mask.snapshot.decode().unwrap();
        assert_eq!(restored.get(0, 0), 255);
        assert_eq!(restored.get(1, 0), 128);
        assert_eq!(restored.get(7, 7), 0);
    }

    #[test]
    fn nested_group_masks_are_restored_to_the_right_layer() {
        let mut document = sample_document();
        let bitmap = MaskBitmap::full(4, 4).unwrap();
        if let Some(children) = document.layers[1].children_mut() {
            children[0].mask = Some(LayerMask {
                snapshot: MaskSnapshot::encode(&bitmap),
                enabled: true,
                inverted: false,
            });
        }
        let pixels = sample_pixels();
        let loaded = decode_project(&encode(&document, &pixels)).unwrap();
        assert!(loaded.document.layers[1].children()[0].mask.is_some());
        assert!(loaded.document.layers[0].mask.is_none());
    }

    #[test]
    fn a_project_stores_the_tree_rather_than_a_flattened_image() {
        let document = sample_document();
        let pixels = sample_pixels();
        let loaded = decode_project(&encode(&document, &pixels)).unwrap();
        assert_eq!(loaded.document.layer_count(), 4);
        assert!(matches!(
            loaded.document.layers[1].children()[1].content,
            LayerContent::Adjustment { .. }
        ));
    }

    #[test]
    fn saving_and_loading_from_disk_round_trips() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("sample.photoforge");
        let document = sample_document();
        let pixels = sample_pixels();
        let written = save_project(
            &path,
            &document,
            &[],
            &borrowed(&pixels),
            "0.8.0",
            "created",
            "modified",
        )
        .unwrap();
        assert!(written > 0);
        let loaded = load_project(&path).unwrap();
        assert_eq!(loaded.document, document);
    }

    #[test]
    fn a_failed_save_leaves_the_previous_project_intact() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("sample.photoforge");
        let document = sample_document();
        let pixels = sample_pixels();
        save_project(
            &path,
            &document,
            &[],
            &borrowed(&pixels),
            "0.8.0",
            "created",
            "modified",
        )
        .unwrap();
        let original = fs::read(&path).unwrap();

        // A document that references a buffer the caller did not supply fails
        // during encoding, before the temporary file is ever renamed.
        let mut broken = document.clone();
        broken.layers.push(pixel_layer("missing", 8, 8));
        assert!(save_project(
            &path,
            &broken,
            &[],
            &borrowed(&pixels),
            "0.8.0",
            "created",
            "modified"
        )
        .is_err());
        assert_eq!(fs::read(&path).unwrap(), original);
        // No temporary files were left behind.
        let leftovers: Vec<_> = fs::read_dir(directory.path())
            .unwrap()
            .filter_map(Result::ok)
            .filter(|entry| entry.file_name().to_string_lossy().ends_with(".tmp"))
            .collect();
        assert!(leftovers.is_empty());
    }

    #[test]
    fn only_the_project_extension_is_accepted_for_saving() {
        let directory = tempfile::tempdir().unwrap();
        let document = sample_document();
        let pixels = sample_pixels();
        for name in ["sample.png", "sample", "sample.pforge"] {
            let path = directory.path().join(name);
            assert!(matches!(
                save_project(
                    &path,
                    &document,
                    &[],
                    &borrowed(&pixels),
                    "0.8.0",
                    "created",
                    "modified"
                ),
                Err(AppError::ProjectIo(_))
            ));
        }
    }

    #[test]
    fn relative_and_traversing_paths_are_rejected() {
        let document = sample_document();
        let pixels = sample_pixels();
        assert!(save_project(
            Path::new("relative.photoforge"),
            &document,
            &[],
            &borrowed(&pixels),
            "0.8.0",
            "created",
            "modified"
        )
        .is_err());
        assert!(load_project(Path::new("relative.photoforge")).is_err());
    }

    #[test]
    fn a_file_without_the_marker_is_rejected() {
        let bytes = vec![0_u8; 256];
        assert!(matches!(
            decode_project(&bytes),
            Err(AppError::ProjectFormat(_))
        ));
    }

    #[test]
    fn a_truncated_project_is_rejected() {
        let document = sample_document();
        let pixels = sample_pixels();
        let bytes = encode(&document, &pixels);
        for cut in [8, 32, bytes.len() / 2, bytes.len() - 9] {
            assert!(
                decode_project(&bytes[..cut]).is_err(),
                "a project truncated at {cut} bytes was accepted"
            );
        }
    }

    #[test]
    fn corrupting_any_byte_fails_the_integrity_check() {
        let document = sample_document();
        let pixels = sample_pixels();
        let bytes = encode(&document, &pixels);
        for offset in [12_usize, 40, bytes.len() / 2, bytes.len() - 20] {
            let mut corrupted = bytes.clone();
            corrupted[offset] ^= 0xff;
            assert!(
                decode_project(&corrupted).is_err(),
                "corruption at byte {offset} was accepted"
            );
        }
    }

    #[test]
    fn trailing_data_after_the_last_entry_is_rejected() {
        let document = sample_document();
        let pixels = sample_pixels();
        let bytes = encode(&document, &pixels);
        let mut padded = bytes[..bytes.len() - 8].to_vec();
        padded.extend_from_slice(b"extra");
        let trailer = fnv1a64(&padded);
        padded.extend_from_slice(&trailer.to_le_bytes());
        assert!(matches!(
            decode_project(&padded),
            Err(AppError::ProjectFormat(_))
        ));
    }

    #[test]
    fn a_future_format_version_is_rejected_safely() {
        let document = sample_document();
        let pixels = sample_pixels();
        let bytes = encode(&document, &pixels);
        let mut future = bytes[..bytes.len() - 8].to_vec();
        future[8..12].copy_from_slice(&(PROJECT_FORMAT_VERSION + 1).to_le_bytes());
        let trailer = fnv1a64(&future);
        future.extend_from_slice(&trailer.to_le_bytes());
        assert!(matches!(
            decode_project(&future),
            Err(AppError::UnsupportedProjectVersion(_))
        ));
    }

    #[test]
    fn entry_names_that_traverse_or_escape_are_rejected() {
        for name in [
            "../escape.png",
            "/absolute.png",
            "layers/../../escape.png",
            "layers\\windows.png",
            "C:/absolute.png",
            "layers//double.png",
            "layers/./same.png",
            "",
            "layers/",
        ] {
            assert!(!valid_entry_name(name), "{name} should be rejected");
        }
        for name in ["layers/px1.png", "masks/a-b_c.png", "manifest.json"] {
            assert!(valid_entry_name(name), "{name} should be accepted");
        }
    }

    #[test]
    fn a_container_carrying_a_traversing_entry_name_is_rejected_on_load() {
        let document = sample_document();
        let pixels = sample_pixels();
        let bytes = encode(&document, &pixels);
        let hostile = b"../escape.png";
        let original = b"layers/pxbase.png";
        let position = bytes
            .windows(original.len())
            .position(|window| window == original)
            .unwrap();
        let mut corrupted = bytes[..bytes.len() - 8].to_vec();
        corrupted[position..position + hostile.len()].copy_from_slice(hostile);
        // Keep the declared name length consistent with the replacement.
        corrupted[position - 11..position - 9]
            .copy_from_slice(&(hostile.len() as u16).to_le_bytes());
        let trailer = fnv1a64(&corrupted);
        corrupted.extend_from_slice(&trailer.to_le_bytes());
        assert!(decode_project(&corrupted).is_err());
    }

    #[test]
    fn an_entry_declaring_more_bytes_than_the_file_holds_is_rejected() {
        let mut bytes = Vec::new();
        bytes.extend_from_slice(PROJECT_MAGIC);
        bytes.extend_from_slice(&PROJECT_FORMAT_VERSION.to_le_bytes());
        let manifest = serde_json::to_vec(&ProjectManifest {
            format_version: PROJECT_FORMAT_VERSION,
            application: "PhotoForge".into(),
            application_version: "0.8.0".into(),
            created_at: String::new(),
            modified_at: String::new(),
            document: LayerDocument::new(4, 4),
            masks: Vec::new(),
            pixels: Vec::new(),
            document_operations: Vec::new(),
        })
        .unwrap();
        bytes.extend_from_slice(&(manifest.len() as u64).to_le_bytes());
        bytes.extend_from_slice(&manifest);
        bytes.extend_from_slice(&1_u32.to_le_bytes());
        let name = b"layers/big.png";
        bytes.extend_from_slice(&(name.len() as u16).to_le_bytes());
        bytes.extend_from_slice(name);
        bytes.push(ENCODING_PNG);
        // Declare a gigantic payload that the file cannot possibly contain.
        bytes.extend_from_slice(&(MAX_ENTRY_BYTES - 1).to_le_bytes());
        bytes.extend_from_slice(&0_u64.to_le_bytes());
        let trailer = fnv1a64(&bytes);
        bytes.extend_from_slice(&trailer.to_le_bytes());
        assert!(matches!(
            decode_project(&bytes),
            Err(AppError::ProjectFormat(_))
        ));
    }

    #[test]
    fn an_entry_declaring_more_bytes_than_the_ceiling_is_rejected() {
        let mut bytes = Vec::new();
        bytes.extend_from_slice(PROJECT_MAGIC);
        bytes.extend_from_slice(&PROJECT_FORMAT_VERSION.to_le_bytes());
        let manifest = serde_json::to_vec(&ProjectManifest {
            format_version: PROJECT_FORMAT_VERSION,
            application: "PhotoForge".into(),
            application_version: "0.8.0".into(),
            created_at: String::new(),
            modified_at: String::new(),
            document: LayerDocument::new(4, 4),
            masks: Vec::new(),
            pixels: Vec::new(),
            document_operations: Vec::new(),
        })
        .unwrap();
        bytes.extend_from_slice(&(manifest.len() as u64).to_le_bytes());
        bytes.extend_from_slice(&manifest);
        bytes.extend_from_slice(&1_u32.to_le_bytes());
        let name = b"layers/big.png";
        bytes.extend_from_slice(&(name.len() as u16).to_le_bytes());
        bytes.extend_from_slice(name);
        bytes.push(ENCODING_PNG);
        bytes.extend_from_slice(&u64::MAX.to_le_bytes());
        bytes.extend_from_slice(&0_u64.to_le_bytes());
        let trailer = fnv1a64(&bytes);
        bytes.extend_from_slice(&trailer.to_le_bytes());
        assert!(matches!(
            decode_project(&bytes),
            Err(AppError::ProjectTooLarge { .. })
        ));
    }

    #[test]
    fn an_oversized_declared_entry_count_is_rejected_before_allocation() {
        let mut bytes = Vec::new();
        bytes.extend_from_slice(PROJECT_MAGIC);
        bytes.extend_from_slice(&PROJECT_FORMAT_VERSION.to_le_bytes());
        let manifest = serde_json::to_vec(&ProjectManifest {
            format_version: PROJECT_FORMAT_VERSION,
            application: "PhotoForge".into(),
            application_version: "0.8.0".into(),
            created_at: String::new(),
            modified_at: String::new(),
            document: LayerDocument::new(4, 4),
            masks: Vec::new(),
            pixels: Vec::new(),
            document_operations: Vec::new(),
        })
        .unwrap();
        bytes.extend_from_slice(&(manifest.len() as u64).to_le_bytes());
        bytes.extend_from_slice(&manifest);
        bytes.extend_from_slice(&u32::MAX.to_le_bytes());
        let trailer = fnv1a64(&bytes);
        bytes.extend_from_slice(&trailer.to_le_bytes());
        assert!(matches!(
            decode_project(&bytes),
            Err(AppError::ProjectFormat(_))
        ));
    }

    #[test]
    fn a_manifest_declaring_more_bytes_than_the_ceiling_is_rejected() {
        let mut bytes = Vec::new();
        bytes.extend_from_slice(PROJECT_MAGIC);
        bytes.extend_from_slice(&PROJECT_FORMAT_VERSION.to_le_bytes());
        bytes.extend_from_slice(&(MAX_MANIFEST_BYTES + 1).to_le_bytes());
        bytes.extend_from_slice(&[0_u8; 64]);
        let trailer = fnv1a64(&bytes);
        bytes.extend_from_slice(&trailer.to_le_bytes());
        assert!(matches!(
            decode_project(&bytes),
            Err(AppError::ProjectTooLarge { .. })
        ));
    }

    /// Builds a container by hand so tests can store payloads the encoder would
    /// never produce.
    fn handmade(manifest: &ProjectManifest, entries: &[(&str, Vec<u8>)]) -> Vec<u8> {
        let manifest_bytes = serde_json::to_vec(manifest).unwrap();
        let mut bytes = Vec::new();
        bytes.extend_from_slice(PROJECT_MAGIC);
        bytes.extend_from_slice(&PROJECT_FORMAT_VERSION.to_le_bytes());
        bytes.extend_from_slice(&(manifest_bytes.len() as u64).to_le_bytes());
        bytes.extend_from_slice(&manifest_bytes);
        bytes.extend_from_slice(&(entries.len() as u32).to_le_bytes());
        for (name, payload) in entries {
            bytes.extend_from_slice(&(name.len() as u16).to_le_bytes());
            bytes.extend_from_slice(name.as_bytes());
            bytes.push(ENCODING_PNG);
            bytes.extend_from_slice(&(payload.len() as u64).to_le_bytes());
            bytes.extend_from_slice(&fnv1a64(payload).to_le_bytes());
            bytes.extend_from_slice(payload);
        }
        let trailer = fnv1a64(&bytes);
        bytes.extend_from_slice(&trailer.to_le_bytes());
        bytes
    }

    fn manifest_for(document: LayerDocument, pixels: Vec<ProjectPixelEntry>) -> ProjectManifest {
        ProjectManifest {
            format_version: PROJECT_FORMAT_VERSION,
            application: "PhotoForge".into(),
            application_version: "0.8.0".into(),
            created_at: String::new(),
            modified_at: String::new(),
            document,
            masks: Vec::new(),
            pixels,
            document_operations: Vec::new(),
        }
    }

    #[test]
    fn version_one_without_precision_metadata_keeps_exact_legacy_pixels() {
        let mut document = LayerDocument::new(8, 8);
        document.layers = vec![pixel_layer("base", 8, 8)];
        let manifest = manifest_for(
            document.clone(),
            vec![ProjectPixelEntry {
                format: PixelFormat::SRGBA8,
                pixel_id: "pxbase".into(),
                entry: "layers/pxbase.png".into(),
                width: 8,
                height: 8,
            }],
        );
        let payload = encode_rgba_png(&image(8, 8, 37)).unwrap();
        let mut value = serde_json::to_value(&manifest).unwrap();
        value["formatVersion"] = 1.into();
        value["document"]
            .as_object_mut()
            .unwrap()
            .remove("precision");
        value["pixels"][0].as_object_mut().unwrap().remove("format");
        let json = serde_json::to_vec(&value).unwrap();
        let original = handmade(&manifest, &[("layers/pxbase.png", payload)]);
        let original_json_len = u64::from_le_bytes(original[12..20].try_into().unwrap()) as usize;
        let mut bytes = PROJECT_MAGIC.to_vec();
        bytes.extend_from_slice(&1u32.to_le_bytes());
        bytes.extend_from_slice(&(json.len() as u64).to_le_bytes());
        bytes.extend_from_slice(&json);
        bytes.extend_from_slice(&original[20 + original_json_len..original.len() - 8]);
        let checksum = fnv1a64(&bytes);
        bytes.extend_from_slice(&checksum.to_le_bytes());
        let loaded = decode_project(&bytes).unwrap();
        assert_eq!(loaded.document, document);
        assert_eq!(loaded.pixels[0].1, image(8, 8, 37));
        assert!(loaded.linear_pixels.is_empty());
    }

    #[test]
    fn aggregate_mask_bomb_is_rejected_before_payload_decode() {
        let mut manifest = manifest_for(LayerDocument::new(8, 8), Vec::new());
        manifest.masks = (0..16)
            .map(|n| ProjectMaskEntry {
                layer_id: format!("mask{n}"),
                entry: format!("masks/{n}.png"),
                width: 9504,
                height: 6336,
                enabled: true,
                inverted: false,
            })
            .collect();
        assert!(matches!(
            decode_project(&handmade(&manifest, &[])),
            Err(AppError::ResourceBudget { .. })
        ));
    }

    #[test]
    fn an_embedded_image_whose_size_disagrees_with_the_manifest_is_rejected() {
        let mut document = LayerDocument::new(8, 8);
        document.layers = vec![pixel_layer("base", 8, 8)];
        // The manifest declares 8x8 but the stored payload is 4x4.
        let manifest = manifest_for(
            document,
            vec![ProjectPixelEntry {
                format: PixelFormat::SRGBA8,
                pixel_id: "pxbase".into(),
                entry: "layers/pxbase.png".into(),
                width: 8,
                height: 8,
            }],
        );
        let payload = encode_rgba_png(&image(4, 4, 10)).unwrap();
        let bytes = handmade(&manifest, &[("layers/pxbase.png", payload)]);
        assert!(matches!(
            decode_project(&bytes),
            Err(AppError::ProjectFormat(_))
        ));
    }

    #[test]
    fn an_embedded_image_declaring_impossible_dimensions_is_rejected_before_decoding() {
        let mut document = LayerDocument::new(8, 8);
        document.layers = vec![pixel_layer("base", 8, 8)];
        let manifest = manifest_for(
            document,
            vec![ProjectPixelEntry {
                format: PixelFormat::SRGBA8,
                pixel_id: "pxbase".into(),
                entry: "layers/pxbase.png".into(),
                width: 19_000,
                height: 19_000,
            }],
        );
        let payload = encode_rgba_png(&image(8, 8, 10)).unwrap();
        let bytes = handmade(&manifest, &[("layers/pxbase.png", payload)]);
        assert!(matches!(
            decode_project(&bytes),
            Err(AppError::ImageTooLarge { .. })
        ));
    }

    #[test]
    fn a_payload_that_is_not_a_readable_image_is_rejected() {
        let mut document = LayerDocument::new(8, 8);
        document.layers = vec![pixel_layer("base", 8, 8)];
        let manifest = manifest_for(
            document,
            vec![ProjectPixelEntry {
                format: PixelFormat::SRGBA8,
                pixel_id: "pxbase".into(),
                entry: "layers/pxbase.png".into(),
                width: 8,
                height: 8,
            }],
        );
        let bytes = handmade(&manifest, &[("layers/pxbase.png", vec![0_u8; 256])]);
        assert!(matches!(
            decode_project(&bytes),
            Err(AppError::ProjectFormat(_))
        ));
    }

    #[test]
    fn a_manifest_referencing_a_missing_entry_is_rejected() {
        let manifest = manifest_for(
            sample_document(),
            vec![ProjectPixelEntry {
                format: PixelFormat::SRGBA8,
                pixel_id: "pxbase".into(),
                entry: "layers/absent.png".into(),
                width: 8,
                height: 8,
            }],
        );
        assert!(matches!(
            decode_project(&handmade(&manifest, &[])),
            Err(AppError::ProjectFormat(_))
        ));
    }

    #[test]
    fn a_document_referencing_pixels_the_file_omits_is_rejected() {
        let manifest = manifest_for(sample_document(), Vec::new());
        assert!(matches!(
            decode_project(&handmade(&manifest, &[])),
            Err(AppError::LayerPixelsMissing(_))
        ));
    }

    #[test]
    fn an_invalid_layer_tree_inside_a_valid_container_is_rejected() {
        let mut document = sample_document();
        document.layers[0].opacity = 4.0;
        let pixels = sample_pixels();
        // Encoding validates too, so build the container by hand.
        let mut stripped = document.clone();
        detach_masks(&mut stripped);
        let manifest = ProjectManifest {
            format_version: PROJECT_FORMAT_VERSION,
            application: "PhotoForge".into(),
            application_version: "0.8.0".into(),
            created_at: String::new(),
            modified_at: String::new(),
            document: stripped,
            masks: Vec::new(),
            pixels: vec![
                ProjectPixelEntry {
                    format: PixelFormat::SRGBA8,
                    pixel_id: "pxbase".into(),
                    entry: "layers/pxbase.png".into(),
                    width: 8,
                    height: 8,
                },
                ProjectPixelEntry {
                    format: PixelFormat::SRGBA8,
                    pixel_id: "pxchild".into(),
                    entry: "layers/pxchild.png".into(),
                    width: 4,
                    height: 4,
                },
            ],
            document_operations: Vec::new(),
        };
        let manifest_bytes = serde_json::to_vec(&manifest).unwrap();
        let mut bytes = Vec::new();
        bytes.extend_from_slice(PROJECT_MAGIC);
        bytes.extend_from_slice(&PROJECT_FORMAT_VERSION.to_le_bytes());
        bytes.extend_from_slice(&(manifest_bytes.len() as u64).to_le_bytes());
        bytes.extend_from_slice(&manifest_bytes);
        bytes.extend_from_slice(&2_u32.to_le_bytes());
        for (name, image) in [
            ("layers/pxbase.png", &pixels[0].1),
            ("layers/pxchild.png", &pixels[1].1),
        ] {
            let payload = encode_rgba_png(image).unwrap();
            bytes.extend_from_slice(&(name.len() as u16).to_le_bytes());
            bytes.extend_from_slice(name.as_bytes());
            bytes.push(ENCODING_PNG);
            bytes.extend_from_slice(&(payload.len() as u64).to_le_bytes());
            bytes.extend_from_slice(&fnv1a64(&payload).to_le_bytes());
            bytes.extend_from_slice(&payload);
        }
        let trailer = fnv1a64(&bytes);
        bytes.extend_from_slice(&trailer.to_le_bytes());
        assert!(matches!(
            decode_project(&bytes),
            Err(AppError::InvalidLayerDocument(_))
        ));
    }

    #[test]
    fn encoding_rejects_a_document_whose_pixels_were_not_supplied() {
        let document = sample_document();
        assert!(matches!(
            encode_project(&document, &[], &[], "0.8.0", "", ""),
            Err(AppError::LayerPixelsMissing(_))
        ));
    }

    #[test]
    fn encoding_rejects_invalid_document_operations() {
        let document = sample_document();
        let pixels = sample_pixels();
        assert!(encode_project(
            &document,
            &[EditOperation::Gamma { value: 0.0 }],
            &borrowed(&pixels),
            "0.8.0",
            "",
            ""
        )
        .is_err());
    }

    #[test]
    fn an_empty_document_round_trips() {
        let document = LayerDocument::new(16, 16);
        let bytes = encode_project(&document, &[], &[], "0.8.0", "", "").unwrap();
        let loaded = decode_project(&bytes).unwrap();
        assert_eq!(loaded.document.layer_count(), 0);
        assert_eq!(loaded.document.canvas_width, 16);
    }

    #[test]
    fn unknown_manifest_fields_are_rejected() {
        let json = r#"{"formatVersion":1,"application":"PhotoForge","applicationVersion":"0.8.0","document":{"schemaVersion":1,"canvasWidth":4,"canvasHeight":4,"layers":[]},"surprise":true}"#;
        assert!(serde_json::from_str::<ProjectManifest>(json).is_err());
    }

    #[test]
    fn the_schema_version_inside_the_document_is_still_enforced() {
        let mut document = LayerDocument::new(8, 8);
        document.schema_version = LAYER_SCHEMA_VERSION + 5;
        assert!(matches!(
            encode_project(&document, &[], &[], "0.8.0", "", ""),
            Err(AppError::UnsupportedLayerSchema(_))
        ));
    }

    #[test]
    fn loading_a_directory_or_missing_file_fails_cleanly() {
        let directory = tempfile::tempdir().unwrap();
        assert!(load_project(directory.path()).is_err());
        assert!(load_project(&directory.path().join("absent.photoforge")).is_err());
    }
}
