//! Optional local neural inference.
//!
//! # Optional means optional
//!
//! PhotoForge is a complete classical editor with no model installed. Nothing
//! in this module runs, allocates or fails at startup when the model store is
//! empty; the registry simply reports that no capability is available, and the
//! interface offers configuration rather than a button that does nothing.
//!
//! # What is here and what is not
//!
//! This module owns the model store, the descriptors, the security checks and
//! the capability reporting. It does not download anything, ever: models are
//! installed by explicit user action from a file the user already has. It does
//! not shell out, and it does not require Python.
//!
//! Whether an inference *runtime* is compiled in is a separate question from
//! whether a model is installed, and both are reported separately, because
//! "no runtime" and "no model" are different problems with different fixes.
pub mod model;

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::Mutex;

use serde::Serialize;

use crate::error::AppError;
use model::{Capability, ModelDescriptor, ModelLimits};

/// Most models the store will hold.
///
/// Bounded so a directory full of files cannot turn startup into an unbounded
/// scan, and so the manifest stays something a person can read.
pub const MAX_INSTALLED_MODELS: usize = 64;

/// The name of the manifest inside the model directory.
const MANIFEST: &str = "models.json";

/// Whether an inference runtime is compiled into this build at all.
///
/// Distinct from whether a model is installed. Reporting them together would
/// leave a user with no models chasing a runtime problem they do not have.
pub const fn runtime_compiled() -> bool {
    cfg!(feature = "inference")
}

/// The name of the compiled runtime, or why there is none.
pub fn runtime_name() -> &'static str {
    if runtime_compiled() {
        "tract (pure Rust, CPU)"
    } else {
        "none compiled in"
    }
}

/// What the interface needs to describe one capability truthfully.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CapabilityStatus {
    pub capability: Capability,
    /// True only when a runtime is compiled in *and* a validated model for this
    /// capability is installed. Anything else is not available.
    pub available: bool,
    /// Ids of installed models offering it, whether or not a runtime exists.
    pub models: Vec<String>,
    /// Why it is unavailable, in words meant for a person. Empty when it is.
    pub reason: String,
}

/// A summary of the whole inference subsystem.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct InferenceStatus {
    pub runtime_compiled: bool,
    pub runtime: String,
    /// Where models are installed, so the user can find and remove them.
    pub model_directory: String,
    pub installed: Vec<ModelDescriptor>,
    pub capabilities: Vec<CapabilityStatus>,
    /// True when the editor is running with no inference at all, which is the
    /// default and fully supported state.
    pub classical_only: bool,
}

/// The set of models PhotoForge has been told about.
///
/// Backed by a directory the application owns and a manifest inside it. The
/// manifest is the authority on what is installed; a stray file in the
/// directory is not a model until it has been imported.
pub struct ModelRegistry {
    directory: PathBuf,
    inner: Mutex<BTreeMap<String, ModelDescriptor>>,
}

impl Default for ModelRegistry {
    fn default() -> Self {
        Self::new(model::default_model_directory())
    }
}

impl ModelRegistry {
    /// Opens a registry over a directory, reading the manifest if it exists.
    ///
    /// Never creates the directory and never fails when it is absent: a fresh
    /// install has no models, which is the normal case, not an error.
    pub fn new(directory: impl Into<PathBuf>) -> Self {
        let directory = directory.into();
        let inner = Mutex::new(read_manifest(&directory).unwrap_or_default());
        Self { directory, inner }
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, BTreeMap<String, ModelDescriptor>> {
        self.inner
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }

    pub fn directory(&self) -> &Path {
        &self.directory
    }

    pub fn installed(&self) -> Vec<ModelDescriptor> {
        self.lock().values().cloned().collect()
    }

    pub fn get(&self, id: &str) -> Option<ModelDescriptor> {
        self.lock().get(id).cloned()
    }

    /// Models offering a capability.
    pub fn for_capability(&self, capability: Capability) -> Vec<ModelDescriptor> {
        self.lock()
            .values()
            .filter(|descriptor| descriptor.capability == capability)
            .cloned()
            .collect()
    }

    /// The honest state of every capability.
    pub fn status(&self) -> InferenceStatus {
        let installed = self.installed();
        let capabilities = Capability::ALL
            .iter()
            .map(|capability| {
                let models: Vec<String> = installed
                    .iter()
                    .filter(|descriptor| descriptor.capability == *capability)
                    .map(|descriptor| descriptor.id.clone())
                    .collect();
                let (available, reason) = if !runtime_compiled() {
                    (
                        false,
                        "This build has no inference runtime compiled in.".to_string(),
                    )
                } else if models.is_empty() {
                    (
                        false,
                        "No model for this capability is installed. Import one in the Model \
                         Manager; PhotoForge never downloads models."
                            .to_string(),
                    )
                } else {
                    (true, String::new())
                };
                CapabilityStatus {
                    capability: *capability,
                    available,
                    models,
                    reason,
                }
            })
            .collect();
        InferenceStatus {
            runtime_compiled: runtime_compiled(),
            runtime: runtime_name().to_string(),
            model_directory: self.directory.to_string_lossy().into_owned(),
            classical_only: installed.is_empty() || !runtime_compiled(),
            installed,
            capabilities,
        }
    }

    /// Imports a model file the user chose.
    ///
    /// The file is inspected before anything is copied, the descriptor is
    /// validated against it, and the copy lands in PhotoForge's own directory
    /// under a name derived from the model id. Nothing is fetched and nothing
    /// outside the store is written.
    pub fn import(
        &self,
        source: &Path,
        mut descriptor: ModelDescriptor,
    ) -> Result<ModelDescriptor, AppError> {
        if self.lock().len() >= MAX_INSTALLED_MODELS {
            return Err(AppError::InvalidOperation(format!(
                "PhotoForge holds at most {MAX_INSTALLED_MODELS} models; remove one first"
            )));
        }
        let inspection = model::inspect_model_file(source)?;
        // The file decides its own size and hash, not the descriptor: a
        // descriptor that disagreed would make the recorded identity a fiction.
        descriptor.file_bytes = inspection.bytes;
        descriptor.sha256 = inspection.sha256;
        descriptor.format = inspection.format;
        descriptor.validate()?;
        if self.lock().contains_key(&descriptor.id) {
            return Err(AppError::InvalidOperation(format!(
                "a model with the id {} is already installed",
                descriptor.id
            )));
        }

        std::fs::create_dir_all(&self.directory).map_err(|error| {
            AppError::ProjectIo(format!("could not create the model directory: {error}"))
        })?;
        let target = self.directory.join(descriptor.file_name());
        // Staged then renamed, so an interrupted import cannot leave a
        // half-copied file that looks installed.
        let staged = self.directory.join(format!(".{}.part", descriptor.id));
        std::fs::copy(source, &staged)
            .map_err(|error| AppError::ProjectIo(format!("could not copy the model: {error}")))?;
        std::fs::rename(&staged, &target).map_err(|error| {
            let _ = std::fs::remove_file(&staged);
            AppError::ProjectIo(format!("could not install the model: {error}"))
        })?;

        self.lock()
            .insert(descriptor.id.clone(), descriptor.clone());
        self.write_manifest()?;
        Ok(descriptor)
    }

    /// Removes an installed model and its file.
    ///
    /// Only ever touches the file this registry installed, inside its own
    /// directory. A model that is in the manifest but whose file is already
    /// gone is still removed from the manifest, so the two cannot drift.
    pub fn remove(&self, id: &str) -> Result<(), AppError> {
        let Some(descriptor) = self.lock().remove(id) else {
            return Err(AppError::InvalidOperation(format!(
                "no model with the id {id} is installed"
            )));
        };
        let target = self.directory.join(descriptor.file_name());
        if target.exists() {
            std::fs::remove_file(&target).map_err(|error| {
                AppError::ProjectIo(format!("could not remove the model file: {error}"))
            })?;
        }
        self.write_manifest()
    }

    /// Resolves a model to its file, checking the hash still matches.
    ///
    /// This is what stops a result being attributed to a model that has since
    /// been replaced. A workflow refers to an id; the id is only honoured if
    /// the bytes behind it are the ones that were imported.
    pub fn resolve_file(&self, id: &str) -> Result<(PathBuf, ModelDescriptor), AppError> {
        let descriptor = self.get(id).ok_or_else(|| {
            AppError::InvalidOperation(format!(
                "this step needs the model {id}, which is not installed"
            ))
        })?;
        descriptor.validate()?;
        let path = self.directory.join(descriptor.file_name());
        let inspection = model::inspect_model_file(&path)?;
        if inspection.sha256 != descriptor.sha256 {
            return Err(AppError::InvalidOperation(format!(
                "the file for model {id} does not match the hash it was installed with; \
                 remove and re-import it"
            )));
        }
        Ok((path, descriptor))
    }

    fn write_manifest(&self) -> Result<(), AppError> {
        let models: Vec<ModelDescriptor> = self.lock().values().cloned().collect();
        let encoded = serde_json::to_vec_pretty(&models).map_err(|error| {
            AppError::ProjectIo(format!("could not encode the model manifest: {error}"))
        })?;
        std::fs::create_dir_all(&self.directory).map_err(|error| {
            AppError::ProjectIo(format!("could not create the model directory: {error}"))
        })?;
        let path = self.directory.join(MANIFEST);
        let staged = self.directory.join(".models.json.part");
        std::fs::write(&staged, &encoded).map_err(|error| {
            AppError::ProjectIo(format!("could not write the model manifest: {error}"))
        })?;
        std::fs::rename(&staged, &path).map_err(|error| {
            let _ = std::fs::remove_file(&staged);
            AppError::ProjectIo(format!("could not publish the model manifest: {error}"))
        })?;
        Ok(())
    }
}

/// Reads the manifest, ignoring anything that does not validate.
///
/// A manifest is a file on disk and therefore untrusted. An entry that fails
/// validation is dropped rather than rejecting the whole file, so one bad
/// record cannot make every installed model disappear.
fn read_manifest(directory: &Path) -> Option<BTreeMap<String, ModelDescriptor>> {
    let path = directory.join(MANIFEST);
    let metadata = std::fs::metadata(&path).ok()?;
    // A manifest is a short list of short records.
    if metadata.len() > (MAX_INSTALLED_MODELS as u64) * (ModelLimits::MAX_TEXT as u64) * 16 {
        return None;
    }
    let bytes = std::fs::read(&path).ok()?;
    let models: Vec<ModelDescriptor> = serde_json::from_slice(&bytes).ok()?;
    Some(
        models
            .into_iter()
            .filter(|descriptor| descriptor.validate().is_ok())
            .take(MAX_INSTALLED_MODELS)
            .map(|descriptor| (descriptor.id.clone(), descriptor))
            .collect(),
    )
}

#[cfg(test)]
mod tests {
    use super::model::{ModelColorSpace, ModelFormat, Normalization};
    use super::*;

    fn descriptor(id: &str, capability: Capability) -> ModelDescriptor {
        ModelDescriptor {
            id: id.into(),
            name: "Test Model".into(),
            version: "1.0".into(),
            architecture: "test".into(),
            capability,
            format: ModelFormat::Onnx,
            color_space: ModelColorSpace::EncodedSrgb,
            normalization: Normalization::UnitRange,
            input_channels: 3,
            output_channels: 3,
            scale: if capability == Capability::SuperResolution {
                2
            } else {
                1
            },
            tile_size: 128,
            tile_overlap: 16,
            file_bytes: 0,
            sha256: String::new(),
            license: "CC0".into(),
            source: "authored for tests".into(),
        }
    }

    fn model_file(folder: &Path, name: &str) -> PathBuf {
        let path = folder.join(name);
        let mut content = vec![0x08u8];
        content.extend_from_slice(name.as_bytes());
        content.extend_from_slice(&[0x07; 2048]);
        std::fs::write(&path, content).unwrap();
        path
    }

    /// The state every PhotoForge install starts in, and the one most stay in.
    #[test]
    fn an_empty_store_is_a_supported_state_not_an_error() {
        let folder = tempfile::tempdir().unwrap();
        let registry = ModelRegistry::new(folder.path().join("does-not-exist"));
        let status = registry.status();
        assert!(status.installed.is_empty());
        assert!(status.classical_only);
        assert_eq!(status.capabilities.len(), Capability::ALL.len());
        for capability in &status.capabilities {
            assert!(
                !capability.available,
                "{:?} claimed to be available with no model installed",
                capability.capability
            );
            assert!(
                !capability.reason.is_empty(),
                "{:?} was unavailable without saying why",
                capability.capability
            );
        }
        // And nothing was created on disk just by asking.
        assert!(!folder.path().join("does-not-exist").exists());
    }

    #[test]
    fn importing_records_the_files_own_size_and_hash() {
        let folder = tempfile::tempdir().unwrap();
        let source = model_file(folder.path(), "source.onnx");
        let registry = ModelRegistry::new(folder.path().join("store"));

        let mut claimed = descriptor("upscale-2x", Capability::SuperResolution);
        // A descriptor that lies about the file must not be believed.
        claimed.file_bytes = 999_999;
        claimed.sha256 = "b".repeat(64);

        let installed = registry.import(&source, claimed).expect("import");
        let truth = model::inspect_model_file(&source).unwrap();
        assert_eq!(installed.file_bytes, truth.bytes);
        assert_eq!(installed.sha256, truth.sha256);
        assert!(registry.directory().join("upscale-2x.onnx").exists());

        // And the capability is now genuinely available, runtime permitting.
        let status = registry.status();
        assert!(!status.classical_only || !runtime_compiled());
        let super_resolution = status
            .capabilities
            .iter()
            .find(|c| c.capability == Capability::SuperResolution)
            .unwrap();
        assert_eq!(super_resolution.models, vec!["upscale-2x".to_string()]);
        assert_eq!(super_resolution.available, runtime_compiled());
    }

    #[test]
    fn a_registry_reopens_what_was_installed() {
        let folder = tempfile::tempdir().unwrap();
        let source = model_file(folder.path(), "source.onnx");
        let store = folder.path().join("store");
        {
            let registry = ModelRegistry::new(&store);
            registry
                .import(&source, descriptor("denoise-1", Capability::Denoise))
                .unwrap();
        }
        let reopened = ModelRegistry::new(&store);
        assert_eq!(reopened.installed().len(), 1);
        assert!(reopened.get("denoise-1").is_some());
    }

    #[test]
    fn removing_deletes_only_the_model_it_installed() {
        let folder = tempfile::tempdir().unwrap();
        let source = model_file(folder.path(), "source.onnx");
        let store = folder.path().join("store");
        let registry = ModelRegistry::new(&store);
        registry
            .import(&source, descriptor("denoise-1", Capability::Denoise))
            .unwrap();

        let sentinel = store.join("keep-me.txt");
        std::fs::write(&sentinel, b"not a model").unwrap();

        registry.remove("denoise-1").unwrap();
        assert!(registry.get("denoise-1").is_none());
        assert!(!store.join("denoise-1.onnx").exists());
        assert!(sentinel.exists(), "removal touched an unrelated file");
        assert!(source.exists(), "removal deleted the user's original file");
        assert!(registry.remove("denoise-1").is_err());
    }

    /// The hash is what ties a saved result to the model that produced it.
    #[test]
    fn a_replaced_model_file_is_refused_rather_than_used() {
        let folder = tempfile::tempdir().unwrap();
        let source = model_file(folder.path(), "source.onnx");
        let store = folder.path().join("store");
        let registry = ModelRegistry::new(&store);
        registry
            .import(
                &source,
                descriptor("upscale-2x", Capability::SuperResolution),
            )
            .unwrap();
        assert!(registry.resolve_file("upscale-2x").is_ok());

        // Swap the bytes behind the id.
        let installed = store.join("upscale-2x.onnx");
        let mut different = vec![0x08u8];
        different.extend_from_slice(&[0x09; 4096]);
        std::fs::write(&installed, different).unwrap();

        let error = registry
            .resolve_file("upscale-2x")
            .expect_err("a swapped model should be refused");
        assert!(
            format!("{error}").contains("hash"),
            "the refusal did not explain itself: {error}"
        );
    }

    #[test]
    fn a_missing_model_names_itself_in_the_error() {
        let folder = tempfile::tempdir().unwrap();
        let registry = ModelRegistry::new(folder.path());
        let error = registry
            .resolve_file("some-model")
            .expect_err("a missing model should fail closed");
        assert!(format!("{error}").contains("some-model"));
    }

    #[test]
    fn duplicate_ids_are_refused() {
        let folder = tempfile::tempdir().unwrap();
        let source = model_file(folder.path(), "source.onnx");
        let registry = ModelRegistry::new(folder.path().join("store"));
        registry
            .import(&source, descriptor("same-id", Capability::Denoise))
            .unwrap();
        assert!(registry
            .import(&source, descriptor("same-id", Capability::Denoise))
            .is_err());
    }

    /// A manifest is a file on disk and therefore untrusted.
    #[test]
    fn a_corrupt_manifest_does_not_break_startup() {
        let folder = tempfile::tempdir().unwrap();
        let store = folder.path().join("store");
        std::fs::create_dir_all(&store).unwrap();
        for content in [
            &b"not json at all"[..],
            &b"{}"[..],
            &b"[{\"id\":\"\"}]"[..],
            &b"[]"[..],
        ] {
            std::fs::write(store.join(MANIFEST), content).unwrap();
            let registry = ModelRegistry::new(&store);
            assert!(
                registry.installed().is_empty(),
                "a corrupt manifest produced models"
            );
            assert!(registry.status().classical_only);
        }
    }

    /// One bad record must not hide the good ones.
    #[test]
    fn an_invalid_manifest_entry_is_dropped_and_the_rest_survive() {
        let folder = tempfile::tempdir().unwrap();
        let store = folder.path().join("store");
        std::fs::create_dir_all(&store).unwrap();
        let mut good = descriptor("good-model", Capability::Denoise);
        good.file_bytes = 4096;
        good.sha256 = "c".repeat(64);
        let mut bad = good.clone();
        bad.id = "../escape".into();
        let manifest = serde_json::to_vec(&vec![bad, good]).unwrap();
        std::fs::write(store.join(MANIFEST), manifest).unwrap();

        let registry = ModelRegistry::new(&store);
        let installed = registry.installed();
        assert_eq!(installed.len(), 1);
        assert_eq!(installed[0].id, "good-model");
    }

    #[test]
    fn the_store_is_bounded() {
        let folder = tempfile::tempdir().unwrap();
        let source = model_file(folder.path(), "source.onnx");
        let registry = ModelRegistry::new(folder.path().join("store"));
        for index in 0..MAX_INSTALLED_MODELS {
            registry
                .import(
                    &source,
                    descriptor(&format!("model-{index}"), Capability::Denoise),
                )
                .unwrap();
        }
        assert!(registry
            .import(&source, descriptor("one-too-many", Capability::Denoise))
            .is_err());
    }

    /// Whether a runtime is compiled in is a different question from whether a
    /// model is installed, and the two must be reported separately.
    #[test]
    fn runtime_and_model_availability_are_reported_separately() {
        let folder = tempfile::tempdir().unwrap();
        let registry = ModelRegistry::new(folder.path());
        let status = registry.status();
        assert_eq!(status.runtime_compiled, runtime_compiled());
        assert!(!status.runtime.is_empty());
        if !runtime_compiled() {
            assert!(status
                .capabilities
                .iter()
                .all(|c| c.reason.contains("runtime")));
        }
    }
}
