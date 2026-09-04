//! What PhotoForge records about a local inference model, and how it decides a
//! model file is safe to look at.
//!
//! # A model file is untrusted input
//!
//! A model arrives from wherever the user got it. It is parsed by a runtime
//! with a large attack surface, it declares its own tensor shapes, and those
//! shapes drive allocation. Everything here exists to bound what a hostile or
//! merely broken file can do before any of it reaches a runtime.
//!
//! Nothing here executes anything. PhotoForge refuses formats that can carry
//! code — Python pickle above all, which is what `.pt`, `.pth` and `.ckpt`
//! usually are — and accepts only graph-and-weights formats.
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use crate::error::AppError;

/// What a model claims to be able to do.
///
/// A capability is a promise about the shape of the work, not about quality. A
/// model that upscales badly still has the SuperResolution capability.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum Capability {
    SuperResolution,
    Denoise,
    Deblur,
    ArtifactRemoval,
    Segmentation,
    FaceRestoration,
    Inpainting,
}

impl Capability {
    pub const ALL: [Self; 7] = [
        Self::SuperResolution,
        Self::Denoise,
        Self::Deblur,
        Self::ArtifactRemoval,
        Self::Segmentation,
        Self::FaceRestoration,
        Self::Inpainting,
    ];

    /// Whether the capability produces a mask rather than pixels.
    ///
    /// Segmentation output goes into the existing mask engine so the user can
    /// refine, feather, invert and combine it with everything else, rather than
    /// being applied to pixels directly.
    pub const fn produces_mask(self) -> bool {
        matches!(self, Self::Segmentation)
    }
}

/// The colour encoding a model expects to be fed.
///
/// PhotoForge works in linear light. Almost every published imaging model was
/// trained on gamma-encoded sRGB, so the conversion has to be explicit and
/// recorded per model rather than assumed.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum ModelColorSpace {
    /// Gamma-encoded sRGB in [0,1]. What most published models expect.
    EncodedSrgb,
    /// Linear sRGB in [0,1]. Rare, and must be declared to be believed.
    LinearSrgb,
}

/// How pixel values are scaled on the way into the model.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum Normalization {
    /// Values in [0,1].
    UnitRange,
    /// Values in [-1,1].
    SignedUnitRange,
}

/// Model file formats PhotoForge will consider.
///
/// Deliberately short. Every entry is a graph-and-weights container that
/// carries no executable code.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum ModelFormat {
    Onnx,
}

impl ModelFormat {
    pub const fn extension(self) -> &'static str {
        match self {
            Self::Onnx => "onnx",
        }
    }

    /// The bytes a file of this format must start with.
    ///
    /// ONNX is protobuf, which has no magic number, but a valid ModelProto
    /// always begins with field 1 (`ir_version`, varint), tag byte 0x08. That
    /// is a weak check and is treated as one: it rejects obvious rubbish early
    /// and proves nothing about the rest of the file.
    pub const fn leading_byte(self) -> u8 {
        match self {
            Self::Onnx => 0x08,
        }
    }
}

/// Everything PhotoForge records about an installed model.
///
/// The hash is the identity that matters. A workflow or a project refers to a
/// model by `id`, and the hash is what proves the file behind that id is still
/// the one the result came from.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ModelDescriptor {
    /// Stable identifier used by workflows and projects. Never a file path.
    pub id: String,
    pub name: String,
    pub version: String,
    /// The architecture or family, as the publisher describes it. Recorded, not
    /// interpreted: PhotoForge does not special-case any architecture.
    pub architecture: String,
    pub capability: Capability,
    pub format: ModelFormat,
    pub color_space: ModelColorSpace,
    pub normalization: Normalization,
    /// Channels the model consumes and produces. Three means RGB, and alpha has
    /// to be handled outside the model.
    pub input_channels: u32,
    pub output_channels: u32,
    /// Integer upscaling factor. One for everything that is not upscaling.
    pub scale: u32,
    /// Tile edge the model is run at, and how much neighbouring tiles overlap.
    pub tile_size: u32,
    pub tile_overlap: u32,
    pub file_bytes: u64,
    pub sha256: String,
    /// Licence and provenance as supplied by whoever installed it. PhotoForge
    /// records what it was told and does not verify it; an empty licence is
    /// shown as unknown rather than as permissive.
    #[serde(default)]
    pub license: String,
    #[serde(default)]
    pub source: String,
}

/// Hard limits on anything a model file may declare.
///
/// These bound allocation before a runtime is handed the file. They are not
/// tuned to any particular model; they are the point past which PhotoForge
/// would rather refuse than find out.
pub struct ModelLimits;

impl ModelLimits {
    /// Largest model file accepted, in bytes.
    pub const MAX_FILE_BYTES: u64 = 512 * 1024 * 1024;
    /// Smallest file that could possibly be a graph.
    pub const MIN_FILE_BYTES: u64 = 32;
    pub const MAX_TILE_SIZE: u32 = 2048;
    pub const MIN_TILE_SIZE: u32 = 32;
    pub const MAX_SCALE: u32 = 8;
    pub const MAX_CHANNELS: u32 = 4;
    /// Longest identifier, name or licence string kept.
    pub const MAX_TEXT: usize = 200;
}

impl ModelDescriptor {
    /// Whether every declared field is inside its bound.
    ///
    /// Called before a model is registered and again before it is run, because
    /// a descriptor can reach the second point from a saved file rather than
    /// from the import that checked it.
    pub fn validate(&self) -> Result<(), AppError> {
        let text = |value: &str, field: &str| -> Result<(), AppError> {
            if value.is_empty() || value.len() > ModelLimits::MAX_TEXT {
                return Err(AppError::InvalidOperation(format!(
                    "model {field} must be between 1 and {} characters",
                    ModelLimits::MAX_TEXT
                )));
            }
            Ok(())
        };
        text(&self.id, "id")?;
        text(&self.name, "name")?;
        text(&self.version, "version")?;
        text(&self.architecture, "architecture")?;

        // The id becomes a filename, so it may not steer anywhere.
        if !self
            .id
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_' || b == b'.')
            || self.id.starts_with('.')
        {
            return Err(AppError::InvalidOperation(
                "a model id may contain only letters, digits, dash, underscore and dot".into(),
            ));
        }
        if self.sha256.len() != 64 || !self.sha256.bytes().all(|b| b.is_ascii_hexdigit()) {
            return Err(AppError::InvalidOperation(
                "a model hash must be 64 hexadecimal characters".into(),
            ));
        }
        if !(ModelLimits::MIN_FILE_BYTES..=ModelLimits::MAX_FILE_BYTES).contains(&self.file_bytes) {
            return Err(AppError::InvalidOperation(
                "the model file size is outside the supported range".into(),
            ));
        }
        if !(ModelLimits::MIN_TILE_SIZE..=ModelLimits::MAX_TILE_SIZE).contains(&self.tile_size) {
            return Err(AppError::InvalidOperation(
                "the model tile size is outside the supported range".into(),
            ));
        }
        // Overlap has to leave a useful interior, or tiling makes no progress.
        if self.tile_overlap >= self.tile_size / 2 {
            return Err(AppError::InvalidOperation(
                "the model tile overlap must be less than half its tile size".into(),
            ));
        }
        if !(1..=ModelLimits::MAX_SCALE).contains(&self.scale) {
            return Err(AppError::InvalidOperation(
                "the model scale factor is outside the supported range".into(),
            ));
        }
        if self.capability != Capability::SuperResolution && self.scale != 1 {
            return Err(AppError::InvalidOperation(
                "only a super-resolution model may declare a scale factor".into(),
            ));
        }
        for channels in [self.input_channels, self.output_channels] {
            if !(1..=ModelLimits::MAX_CHANNELS).contains(&channels) {
                return Err(AppError::InvalidOperation(
                    "the model channel count is outside the supported range".into(),
                ));
            }
        }
        if self.license.len() > ModelLimits::MAX_TEXT || self.source.len() > ModelLimits::MAX_TEXT {
            return Err(AppError::InvalidOperation(
                "the model licence or source text is too long".into(),
            ));
        }
        Ok(())
    }

    /// Bytes this model needs to process one tile, weights included.
    ///
    /// Deliberately an over-estimate. Intermediate activations depend on the
    /// graph and cannot be known from the descriptor, so a fixed multiple of
    /// the tile stands in for them; a scheduler that under-estimates would
    /// admit work that then exhausts memory.
    pub fn estimated_bytes(&self) -> u64 {
        // Saturating throughout: this is called on descriptors that may not
        // have passed validation yet, and a hostile tile size is precisely the
        // input that would otherwise overflow the estimate to a small number
        // and let the work through the admission check.
        let input = u64::from(self.tile_size)
            .saturating_mul(u64::from(self.tile_size))
            .saturating_mul(4);
        let output = input
            .saturating_mul(u64::from(self.scale))
            .saturating_mul(u64::from(self.scale));
        let activations = input.saturating_mul(8);
        self.file_bytes
            .saturating_add(input.saturating_mul(u64::from(self.input_channels)))
            .saturating_add(output.saturating_mul(u64::from(self.output_channels)))
            .saturating_add(activations)
    }

    /// Where this model's file lives inside the store.
    pub fn file_name(&self) -> String {
        format!("{}.{}", self.id, self.format.extension())
    }
}

/// The result of looking at a model file without running it.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ModelInspection {
    pub bytes: u64,
    pub sha256: String,
    pub format: ModelFormat,
}

/// File extensions that carry executable code, refused by name.
///
/// PyTorch checkpoints are Python pickles. Unpickling runs arbitrary code by
/// design, and no amount of validation makes that safe, so the answer is not to
/// support the format at all rather than to try to sanitise it.
const CODE_BEARING_EXTENSIONS: [&str; 7] = ["pt", "pth", "ckpt", "pkl", "pickle", "bin", "joblib"];

/// Reads a candidate model file and decides whether it may be imported.
///
/// This is a bounded structural check, not a guarantee. It confirms the file is
/// a plausible ONNX container of a sane size and hashes it. It cannot prove the
/// graph inside is safe to execute; that is the runtime's problem, and the
/// runtime is chosen partly for being pure Rust with no code loading.
pub fn inspect_model_file(path: &Path) -> Result<ModelInspection, AppError> {
    let extension = path
        .extension()
        .and_then(|value| value.to_str())
        .unwrap_or("")
        .to_ascii_lowercase();
    if CODE_BEARING_EXTENSIONS.contains(&extension.as_str()) {
        return Err(AppError::InvalidOperation(format!(
            "{extension} files are Python pickles, which execute code when loaded; \
             PhotoForge accepts ONNX graphs only"
        )));
    }
    if extension != ModelFormat::Onnx.extension() {
        return Err(AppError::InvalidOperation(
            "PhotoForge accepts .onnx model files only".into(),
        ));
    }

    let metadata = std::fs::symlink_metadata(path)
        .map_err(|error| AppError::ProjectIo(format!("could not read the model file: {error}")))?;
    if metadata.file_type().is_symlink() || !metadata.is_file() {
        return Err(AppError::InvalidOperation(
            "a model must be a regular file".into(),
        ));
    }
    let bytes = metadata.len();
    if bytes < ModelLimits::MIN_FILE_BYTES {
        return Err(AppError::InvalidOperation(
            "the model file is too small to contain a graph".into(),
        ));
    }
    if bytes > ModelLimits::MAX_FILE_BYTES {
        return Err(AppError::InvalidOperation(format!(
            "the model file is larger than the {} MB limit",
            ModelLimits::MAX_FILE_BYTES / (1024 * 1024)
        )));
    }

    // Hashed in chunks: the size is bounded above, but not so far above that
    // reading it whole into memory is reasonable.
    let file = std::fs::File::open(path)
        .map_err(|error| AppError::ProjectIo(format!("could not open the model file: {error}")))?;
    let mut reader = std::io::BufReader::new(file);
    let mut hasher = Sha256::new();
    let mut buffer = vec![0u8; 64 * 1024];
    let mut first = None;
    let mut total = 0u64;
    loop {
        use std::io::Read;
        let read = reader
            .read(&mut buffer)
            .map_err(|error| AppError::ProjectIo(format!("could not read the model: {error}")))?;
        if read == 0 {
            break;
        }
        if first.is_none() {
            first = Some(buffer[0]);
        }
        total += read as u64;
        if total > ModelLimits::MAX_FILE_BYTES {
            return Err(AppError::InvalidOperation(
                "the model file grew past its declared size while being read".into(),
            ));
        }
        hasher.update(&buffer[..read]);
    }
    if first != Some(ModelFormat::Onnx.leading_byte()) {
        return Err(AppError::InvalidOperation(
            "the file does not begin like an ONNX graph".into(),
        ));
    }

    Ok(ModelInspection {
        bytes: total,
        sha256: format!("{:x}", hasher.finalize()),
        format: ModelFormat::Onnx,
    })
}

/// Where imported models live.
///
/// One directory PhotoForge owns, under the same local application data path as
/// the component registry and the render cache. Models are never read from
/// arbitrary locations at run time: a project refers to a model id, and the id
/// is resolved here.
pub fn default_model_directory() -> PathBuf {
    std::env::var_os("LOCALAPPDATA")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("."))
        .join("PhotoForge")
        .join("inference-models")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn descriptor() -> ModelDescriptor {
        ModelDescriptor {
            id: "test-upscale-2x".into(),
            name: "Test Upscaler".into(),
            version: "1.0".into(),
            architecture: "test".into(),
            capability: Capability::SuperResolution,
            format: ModelFormat::Onnx,
            color_space: ModelColorSpace::EncodedSrgb,
            normalization: Normalization::UnitRange,
            input_channels: 3,
            output_channels: 3,
            scale: 2,
            tile_size: 128,
            tile_overlap: 16,
            file_bytes: 4096,
            sha256: "a".repeat(64),
            license: "CC0".into(),
            source: "authored for tests".into(),
        }
    }

    #[test]
    fn a_well_formed_descriptor_validates() {
        descriptor().validate().expect("descriptor should validate");
    }

    /// A descriptor can reach the runtime from a saved project rather than from
    /// the import that checked it, so every bound is asserted directly.
    #[test]
    fn out_of_range_descriptors_are_refused() {
        type Mutation = (&'static str, Box<dyn Fn(&mut ModelDescriptor)>);
        let cases: Vec<Mutation> = vec![
            ("empty id", Box::new(|d: &mut ModelDescriptor| d.id.clear())),
            (
                "path traversal in id",
                Box::new(|d: &mut ModelDescriptor| d.id = "../escape".into()),
            ),
            (
                "leading dot id",
                Box::new(|d: &mut ModelDescriptor| d.id = ".hidden".into()),
            ),
            (
                "id with a separator",
                Box::new(|d: &mut ModelDescriptor| d.id = "a/b".into()),
            ),
            (
                "short hash",
                Box::new(|d: &mut ModelDescriptor| d.sha256 = "abc".into()),
            ),
            (
                "non-hex hash",
                Box::new(|d: &mut ModelDescriptor| d.sha256 = "z".repeat(64)),
            ),
            (
                "enormous file",
                Box::new(|d: &mut ModelDescriptor| d.file_bytes = u64::MAX),
            ),
            (
                "empty file",
                Box::new(|d: &mut ModelDescriptor| d.file_bytes = 0),
            ),
            (
                "enormous tile",
                Box::new(|d: &mut ModelDescriptor| d.tile_size = 65535),
            ),
            (
                "overlap swallows the tile",
                Box::new(|d: &mut ModelDescriptor| d.tile_overlap = 120),
            ),
            (
                "absurd scale",
                Box::new(|d: &mut ModelDescriptor| d.scale = 64),
            ),
            (
                "zero scale",
                Box::new(|d: &mut ModelDescriptor| d.scale = 0),
            ),
            (
                "scale on a non-upscaler",
                Box::new(|d: &mut ModelDescriptor| {
                    d.capability = Capability::Denoise;
                    d.scale = 2;
                }),
            ),
            (
                "too many channels",
                Box::new(|d: &mut ModelDescriptor| d.input_channels = 99),
            ),
            (
                "zero channels",
                Box::new(|d: &mut ModelDescriptor| d.output_channels = 0),
            ),
            (
                "overlong licence",
                Box::new(|d: &mut ModelDescriptor| d.license = "x".repeat(5000)),
            ),
        ];
        for (name, mutate) in cases {
            let mut candidate = descriptor();
            mutate(&mut candidate);
            assert!(
                candidate.validate().is_err(),
                "{name} was accepted by validation"
            );
        }
    }

    /// The memory estimate drives admission, so it must never be zero and must
    /// grow with the things that actually cost memory.
    #[test]
    fn the_memory_estimate_grows_with_tile_and_scale() {
        let base = descriptor();
        assert!(base.estimated_bytes() > base.file_bytes);

        let mut bigger_tile = base.clone();
        bigger_tile.tile_size = 512;
        assert!(bigger_tile.estimated_bytes() > base.estimated_bytes());

        let mut bigger_scale = base.clone();
        bigger_scale.scale = 4;
        assert!(bigger_scale.estimated_bytes() > base.estimated_bytes());

        // Saturating rather than overflowing, even on nonsense a descriptor
        // would never pass validation with.
        let mut absurd = base;
        absurd.file_bytes = u64::MAX;
        absurd.tile_size = u32::MAX;
        assert!(absurd.estimated_bytes() > 0);
    }

    #[test]
    fn code_bearing_formats_are_refused_by_name() {
        let folder = tempfile::tempdir().unwrap();
        for extension in CODE_BEARING_EXTENSIONS {
            let path = folder.path().join(format!("model.{extension}"));
            std::fs::write(&path, vec![0x08; 1024]).unwrap();
            let error = inspect_model_file(&path).expect_err("should be refused");
            let message = format!("{error}");
            assert!(
                message.contains("pickle") || message.contains("ONNX"),
                "{extension} was refused with an unhelpful message: {message}"
            );
        }
    }

    #[test]
    fn malformed_model_files_are_refused() {
        let folder = tempfile::tempdir().unwrap();

        let empty = folder.path().join("empty.onnx");
        std::fs::write(&empty, b"").unwrap();
        assert!(
            inspect_model_file(&empty).is_err(),
            "an empty file was accepted"
        );

        let tiny = folder.path().join("tiny.onnx");
        std::fs::write(&tiny, b"\x08tiny").unwrap();
        assert!(
            inspect_model_file(&tiny).is_err(),
            "a 5-byte file was accepted"
        );

        let rubbish = folder.path().join("rubbish.onnx");
        std::fs::write(&rubbish, vec![0xffu8; 4096]).unwrap();
        assert!(
            inspect_model_file(&rubbish).is_err(),
            "a file that does not start like a graph was accepted"
        );

        let missing = folder.path().join("missing.onnx");
        assert!(inspect_model_file(&missing).is_err());

        let directory = folder.path().join("directory.onnx");
        std::fs::create_dir(&directory).unwrap();
        assert!(
            inspect_model_file(&directory).is_err(),
            "a directory was accepted as a model"
        );
    }

    /// A plausible file is accepted and hashed. The hash is the identity a
    /// project relies on, so it has to be the hash of the file's real bytes.
    #[test]
    fn a_plausible_file_is_accepted_and_hashed() {
        let folder = tempfile::tempdir().unwrap();
        let path = folder.path().join("model.onnx");
        let mut content = vec![0x08u8];
        content.extend_from_slice(&[0x07; 4095]);
        std::fs::write(&path, &content).unwrap();

        let inspection = inspect_model_file(&path).expect("should be accepted");
        assert_eq!(inspection.bytes, 4096);
        assert_eq!(inspection.format, ModelFormat::Onnx);
        let expected = format!("{:x}", Sha256::digest(&content));
        assert_eq!(inspection.sha256, expected);
    }

    #[test]
    fn a_model_file_name_is_derived_from_the_id_and_format() {
        let descriptor = descriptor();
        assert_eq!(descriptor.file_name(), "test-upscale-2x.onnx");
    }
}
