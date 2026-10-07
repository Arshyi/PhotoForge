//! Installing, keeping and finding plugins.
//!
//! The store holds each plugin version as the **validated package file itself**,
//! under `store/<plugin id>/<content hash>.photoforge-plugin`. Keeping the original
//! rather than an unpacked copy has three consequences worth having:
//!
//! * every time a plugin is loaded it goes through the same hostile-input reader as
//!   when it was installed, so a file altered on disk afterwards is refused, not
//!   trusted because it was once checked;
//! * installing is one atomic file write, so there is no half-unpacked plugin;
//! * the file's name *is* its identity, and loading checks the two agree.
//!
//! A plugin's identity is the **content hash**: SHA-256 over the manifest exactly as
//! shipped and the module's hash. Documents record it, the render cache is keyed by
//! it, and any change to a filter's behaviour or declaration changes it. An update
//! installs a new version beside the old one and only then moves the "active"
//! pointer, so a document that was made with the old version keeps finding it until
//! the person removes it — it is never quietly rendered by a different build.
//!
//! The decisions a person made — enabled or not, which capabilities were granted —
//! live in `state.json`, written atomically.
use super::filter;
use super::limits;
use super::manifest::{valid_plugin_id, Capability, PluginManifest};
use super::package::{read_package, sha256_hex, LoadedPackage, MAX_PACKAGE_BYTES};
use super::{Compiled, PluginError, Runtime};
use crate::color::{FloatImage, FloatRgba};
use crate::error::AppError;
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, HashMap};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, OnceLock, RwLock};

const STATE_VERSION: u32 = 1;
const STATE_FILE: &str = "state.json";
const PACKAGE_EXTENSION: &str = "photoforge-plugin";
const MAX_STATE_BYTES: u64 = 1024 * 1024;
const MAX_PLUGINS: usize = 128;
const MAX_VERSIONS_PER_PLUGIN: usize = 16;
const MAX_REMEMBERED_VALUE_SETS: usize = 64;

fn fail(message: impl Into<String>) -> AppError {
    AppError::InvalidPluginManifest(message.into())
}

/// The identity of a package: its manifest as shipped, and its module.
pub fn content_hash(package: &LoadedPackage) -> String {
    let module = package
        .manifest
        .entry
        .as_ref()
        .map_or("", |entry| entry.sha256.as_str());
    let mut bytes = Vec::with_capacity(package.manifest_json.len() + 1 + module.len());
    bytes.extend_from_slice(package.manifest_json.as_bytes());
    bytes.push(0);
    bytes.extend_from_slice(module.as_bytes());
    sha256_hex(&bytes)
}

// ---- persisted state ----------------------------------------------------------------

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct VersionRecord {
    pub content_hash: String,
    pub version: String,
    pub installed_at: String,
    pub package_sha256: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct PluginState {
    /// The content hash new work uses.
    active: String,
    enabled: bool,
    granted: Vec<Capability>,
    versions: Vec<VersionRecord>,
    /// The values a person last used for a filter or command's parameters, so the
    /// dialog opens where they left it. Held by the host: a module never sees it.
    #[serde(default)]
    remembered: BTreeMap<String, BTreeMap<String, f64>>,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct State {
    version: u32,
    #[serde(default)]
    plugins: BTreeMap<String, PluginState>,
}

// ---- what is loaded -----------------------------------------------------------------

/// A plugin version that has passed every check, ready to use.
pub struct Loaded {
    pub manifest: PluginManifest,
    pub content_hash: String,
    pub readme: Option<String>,
    pub license: Option<String>,
    pub compiled: Option<Compiled>,
}

/// Why a document's plugin can or cannot be used right now.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum Availability {
    Available,
    /// Nothing by that name is installed.
    Missing,
    /// It is installed and a person has turned it off.
    Disabled,
    /// It is installed, but the capability its filters need has not been granted.
    NotGranted {
        capability: String,
    },
    /// This build has no plugin runtime.
    RuntimeUnavailable,
    /// The file on disk is not what was installed.
    #[serde(rename_all = "camelCase")]
    Damaged {
        reason: String,
    },
    /// A different version is installed. The document's own is not, and is not
    /// replaced by the one that is.
    #[serde(rename_all = "camelCase")]
    OtherVersion {
        installed_version: String,
        installed_hash: String,
    },
}

impl Availability {
    pub fn is_available(&self) -> bool {
        matches!(self, Self::Available)
    }

    pub fn describe(&self, plugin: &str, version: &str) -> String {
        match self {
            Self::Available => format!("{plugin} {version} is installed."),
            Self::Missing => format!("{plugin} {version} is not installed."),
            Self::Disabled => format!("{plugin} is installed but turned off."),
            Self::NotGranted { capability } => {
                format!("{plugin} has not been allowed to {capability}.")
            }
            Self::RuntimeUnavailable => {
                format!("{plugin} needs the plugin runtime, which this build of PhotoForge does not include.")
            }
            Self::Damaged { reason } => format!("The installed copy of {plugin} is damaged: {reason}"),
            Self::OtherVersion { installed_version, .. } => format!(
                "This needs {plugin} {version}; version {installed_version} is installed instead. \
                 PhotoForge will not render with a different version unless you install the exact one."
            ),
        }
    }
}

pub enum Resolution {
    Available(Arc<Loaded>),
    Unavailable(Availability),
}

// ---- reports ------------------------------------------------------------------------

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CapabilityInfo {
    pub id: String,
    pub description: String,
}

/// What a person is shown before they install: everything the package would do.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Inspection {
    pub manifest: PluginManifest,
    pub content_hash: String,
    pub package_sha256: String,
    pub package_bytes: u64,
    pub capabilities: Vec<CapabilityInfo>,
    /// The version installed now, if this is an update or a reinstall.
    pub installed_version: Option<String>,
    pub already_installed: bool,
    pub older_than_installed: bool,
    /// Capabilities this version asks for that the installed one was not granted.
    pub new_capabilities: Vec<CapabilityInfo>,
    pub has_module: bool,
    pub runtime_available: bool,
    pub readme: Option<String>,
    pub license: Option<String>,
    /// Always says so: PhotoForge does not verify who made a plugin.
    pub signature: String,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct FilterTest {
    pub filter: String,
    pub passed: bool,
    pub message: String,
    pub tiles: usize,
    pub fuel_used: u64,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TestReport {
    pub plugin: String,
    pub passed: bool,
    pub filters: Vec<FilterTest>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct InstallReport {
    pub plugin: String,
    pub version: String,
    pub content_hash: String,
    pub updated_from: Option<String>,
    pub test: TestReport,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PluginSummary {
    pub id: String,
    pub enabled: bool,
    pub granted: Vec<String>,
    pub active_hash: String,
    pub versions: Vec<VersionRecord>,
    /// The active version's manifest, or `None` if its file could not be read.
    pub manifest: Option<PluginManifest>,
    pub availability: Availability,
    pub readme: Option<String>,
    pub license: Option<String>,
}

/// A checkerboard-and-gradient image for self-tests: edges at several scales, so a
/// filter that reads the wrong neighbour changes the answer.
pub fn test_image(width: u32, height: u32) -> FloatImage {
    let mut image = FloatImage::blank(width, height, FloatRgba::TRANSPARENT)
        .expect("a small test image is always allowed");
    let mut state = 0x2545_f491u32;
    for y in 0..height {
        for x in 0..width {
            state = state.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
            let noise = (state >> 8) as f32 / (1u32 << 24) as f32;
            let ramp = ((x * 5 + y * 3) % 61) as f32 / 60.0;
            let edge = if (x / 4 + y / 5) % 2 == 0 { 0.2 } else { 0.0 };
            image.pixels_mut()[(y * width + x) as usize] = FloatRgba::new(
                (ramp * 0.7 + edge + noise * 0.08).min(1.2),
                (noise * 0.6 + edge).min(1.2),
                (x as f32 / width as f32 * 0.8 + noise * 0.1).min(1.2),
                if (x + y) % 9 == 0 { 0.5 } else { 1.0 },
            );
        }
    }
    image
}

// ---- the registry -------------------------------------------------------------------

pub struct PluginRegistry {
    root: PathBuf,
    state: Mutex<State>,
    runtime: OnceLock<Result<Runtime, PluginError>>,
    loaded: Mutex<HashMap<String, Arc<Loaded>>>,
    /// Problems found reading `state.json`, so a damaged file is reported, not
    /// silently replaced by an empty one.
    state_warning: Mutex<Option<String>>,
}

impl PluginRegistry {
    /// Opens the store at `root`. A missing store is an empty one; an unreadable
    /// state file is reported and treated as empty *without being overwritten*
    /// until something changes it.
    pub fn open(root: impl Into<PathBuf>) -> Self {
        let root = root.into();
        let (state, warning) = match read_state(&root) {
            Ok(state) => (state, None),
            Err(message) => (State::default(), Some(message)),
        };
        Self {
            root,
            state: Mutex::new(state),
            runtime: OnceLock::new(),
            loaded: Mutex::new(HashMap::new()),
            state_warning: Mutex::new(warning),
        }
    }

    pub fn root(&self) -> &Path {
        &self.root
    }

    pub fn state_warning(&self) -> Option<String> {
        self.state_warning
            .lock()
            .ok()
            .and_then(|warning| warning.clone())
    }

    /// The runtime, started the first time something needs it.
    pub fn runtime(&self) -> Result<&Runtime, PluginError> {
        self.runtime
            .get_or_init(Runtime::new)
            .as_ref()
            .map_err(Clone::clone)
    }

    fn package_path(&self, id: &str, hash: &str) -> PathBuf {
        self.root
            .join("store")
            .join(id)
            .join(format!("{hash}.{PACKAGE_EXTENSION}"))
    }

    fn lock_state(&self) -> Result<std::sync::MutexGuard<'_, State>, AppError> {
        self.state
            .lock()
            .map_err(|_| AppError::ProcessingFailure("the plugin list is unavailable".into()))
    }

    fn persist(&self, state: &State) -> Result<(), AppError> {
        write_atomic(
            &self.root.join(STATE_FILE),
            &serde_json::to_vec_pretty(state)
                .map_err(|error| AppError::ProcessingFailure(error.to_string()))?,
        )?;
        if let Ok(mut warning) = self.state_warning.lock() {
            *warning = None;
        }
        Ok(())
    }

    /// Loads one version, through the full package checks again.
    fn load(&self, id: &str, hash: &str) -> Result<Arc<Loaded>, String> {
        let key = format!("{id}/{hash}");
        if let Some(found) = self
            .loaded
            .lock()
            .ok()
            .and_then(|cache| cache.get(&key).cloned())
        {
            return Ok(found);
        }
        let path = self.package_path(id, hash);
        let metadata = std::fs::metadata(&path).map_err(|_| "its file is missing".to_string())?;
        if metadata.len() > MAX_PACKAGE_BYTES {
            return Err("its file is too large".into());
        }
        let bytes =
            std::fs::read(&path).map_err(|error| format!("its file cannot be read: {error}"))?;
        let package = read_package(&bytes).map_err(|error| error.to_string())?;
        if package.manifest.id != id {
            return Err("its file holds a different plugin than its name says".into());
        }
        if content_hash(&package) != hash {
            return Err("its contents are not what was installed".into());
        }
        let compiled = match &package.module {
            None => None,
            Some(module) => Some(
                self.runtime()
                    .and_then(|runtime| runtime.compile(module))
                    .map_err(|error| error.to_string())?,
            ),
        };
        let loaded = Arc::new(Loaded {
            manifest: package.manifest,
            content_hash: hash.to_string(),
            readme: package.readme,
            license: package.license,
            compiled,
        });
        if let Ok(mut cache) = self.loaded.lock() {
            cache.insert(key, Arc::clone(&loaded));
        }
        Ok(loaded)
    }

    fn forget(&self, id: &str, hash: Option<&str>) {
        if let Ok(mut cache) = self.loaded.lock() {
            cache.retain(|key, _| {
                let (plugin, version) = key.split_once('/').unwrap_or((key, ""));
                plugin != id || hash.is_some_and(|hash| hash != version)
            });
        }
    }

    // ---- looking things up -------------------------------------------------------

    /// Resolves the exact version a document names.
    pub fn resolve(&self, id: &str, hash: &str) -> Resolution {
        let state = match self.state.lock() {
            Ok(state) => state.clone(),
            Err(_) => {
                return Resolution::Unavailable(Availability::Damaged {
                    reason: "the plugin list is unavailable".into(),
                })
            }
        };
        let Some(plugin) = state.plugins.get(id) else {
            return Resolution::Unavailable(Availability::Missing);
        };
        if !plugin
            .versions
            .iter()
            .any(|record| record.content_hash == hash)
        {
            let active = plugin
                .versions
                .iter()
                .find(|record| record.content_hash == plugin.active);
            return Resolution::Unavailable(match active {
                Some(record) => Availability::OtherVersion {
                    installed_version: record.version.clone(),
                    installed_hash: record.content_hash.clone(),
                },
                None => Availability::Missing,
            });
        }
        if !plugin.enabled {
            return Resolution::Unavailable(Availability::Disabled);
        }
        let loaded = match self.load(id, hash) {
            Ok(loaded) => loaded,
            Err(reason) => {
                if !Runtime::available() && reason.contains("runtime") {
                    return Resolution::Unavailable(Availability::RuntimeUnavailable);
                }
                return Resolution::Unavailable(Availability::Damaged { reason });
            }
        };
        if !loaded.manifest.filters.is_empty() {
            if !Runtime::available() {
                return Resolution::Unavailable(Availability::RuntimeUnavailable);
            }
            if !plugin.granted.contains(&Capability::FilterPixels) {
                return Resolution::Unavailable(Availability::NotGranted {
                    capability: Capability::FilterPixels.describe().to_lowercase(),
                });
            }
        }
        Resolution::Available(loaded)
    }

    /// Resolves the version new work should use.
    pub fn active(&self, id: &str) -> Resolution {
        let hash = self
            .state
            .lock()
            .ok()
            .and_then(|state| state.plugins.get(id).map(|plugin| plugin.active.clone()));
        match hash {
            Some(hash) => self.resolve(id, &hash),
            None => Resolution::Unavailable(Availability::Missing),
        }
    }

    /// Whether a plugin has been granted a capability.
    pub fn granted(&self, id: &str, capability: Capability) -> bool {
        self.state
            .lock()
            .ok()
            .and_then(|state| {
                state
                    .plugins
                    .get(id)
                    .map(|plugin| plugin.granted.contains(&capability))
            })
            .unwrap_or(false)
    }

    pub fn list(&self) -> Vec<PluginSummary> {
        let state = match self.state.lock() {
            Ok(state) => state.clone(),
            Err(_) => return Vec::new(),
        };
        state
            .plugins
            .iter()
            .map(|(id, plugin)| {
                let (manifest, readme, license) = match self.load(id, &plugin.active) {
                    Ok(loaded) => (
                        Some(loaded.manifest.clone()),
                        loaded.readme.clone(),
                        loaded.license.clone(),
                    ),
                    Err(_) => (None, None, None),
                };
                let availability = match self.resolve(id, &plugin.active) {
                    Resolution::Available(_) => Availability::Available,
                    Resolution::Unavailable(why) => why,
                };
                PluginSummary {
                    id: id.clone(),
                    enabled: plugin.enabled,
                    granted: plugin.granted.iter().map(|c| c.id().to_string()).collect(),
                    active_hash: plugin.active.clone(),
                    versions: plugin.versions.clone(),
                    manifest,
                    availability,
                    readme,
                    license,
                }
            })
            .collect()
    }

    // ---- installing --------------------------------------------------------------

    /// Describes a package without changing anything.
    pub fn inspect(&self, bytes: &[u8]) -> Result<Inspection, AppError> {
        let package = read_package(bytes)?;
        let hash = content_hash(&package);
        let declared = package.manifest.declared_capabilities()?;
        let describe = |capability: &Capability| CapabilityInfo {
            id: capability.id().to_string(),
            description: capability.describe().to_string(),
        };
        let state = self.lock_state()?.clone();
        let installed = state.plugins.get(&package.manifest.id);
        let installed_record = installed.and_then(|plugin| {
            plugin
                .versions
                .iter()
                .find(|record| record.content_hash == plugin.active)
        });
        let previously_granted: &[Capability] = installed.map_or(&[], |plugin| &plugin.granted);
        Ok(Inspection {
            capabilities: declared.iter().map(describe).collect(),
            new_capabilities: declared
                .iter()
                .filter(|capability| installed.is_some() && !previously_granted.contains(capability))
                .map(describe)
                .collect(),
            installed_version: installed_record.map(|record| record.version.clone()),
            already_installed: installed
                .is_some_and(|plugin| plugin.versions.iter().any(|record| record.content_hash == hash)),
            older_than_installed: installed_record
                .is_some_and(|record| version_key(&package.manifest.version) < version_key(&record.version)),
            has_module: package.module.is_some(),
            runtime_available: Runtime::available(),
            content_hash: hash,
            package_sha256: package.package_sha256.clone(),
            package_bytes: bytes.len() as u64,
            readme: package.readme.clone(),
            license: package.license.clone(),
            signature: "Not signed. PhotoForge does not verify who made a plugin; what it limits is what a plugin can do.".into(),
            manifest: package.manifest,
        })
    }

    /// Installs a package, granting `grant` (a subset of what it declares).
    ///
    /// `expected_hash` is the content hash the person was shown by `inspect`: if the
    /// file changed between looking and installing, nothing is installed.
    ///
    /// All or nothing: the module must compile, and every filter must pass its
    /// self-test (run twice with identical results, and the same whole and tiled).
    pub fn install(
        &self,
        bytes: &[u8],
        expected_hash: Option<&str>,
        grant: &[Capability],
        now: &str,
    ) -> Result<InstallReport, AppError> {
        let package = read_package(bytes)?;
        let hash = content_hash(&package);
        if expected_hash.is_some_and(|expected| expected != hash) {
            return Err(fail(
                "the package changed after it was inspected, so it was not installed",
            ));
        }
        let id = package.manifest.id.clone();
        let declared = package.manifest.declared_capabilities()?;
        for capability in grant {
            if !declared.contains(capability) {
                return Err(fail(format!(
                    "{} was granted {}, which it did not ask for",
                    id,
                    capability.id()
                )));
            }
        }
        let mut grant: Vec<Capability> = grant.to_vec();
        grant.sort();
        grant.dedup();

        // Everything that can fail before the disk is touched.
        let mut test = TestReport {
            plugin: id.clone(),
            passed: true,
            filters: Vec::new(),
        };
        if let Some(module) = &package.module {
            let runtime = self.runtime().map_err(|error| fail(error.to_string()))?;
            let compiled = runtime
                .compile(module)
                .map_err(|error| fail(error.to_string()))?;
            test = self_test(runtime, &compiled, &package.manifest);
            if !test.passed {
                let first = test.filters.iter().find(|result| !result.passed);
                return Err(fail(format!(
                    "{} failed its self-test and was not installed: {}",
                    package.manifest.name,
                    first.map_or("a filter did not pass".into(), |result| format!(
                        "{}: {}",
                        result.filter, result.message
                    ))
                )));
            }
        }

        let mut state = self.lock_state()?;
        if state.plugins.len() >= MAX_PLUGINS && !state.plugins.contains_key(&id) {
            return Err(fail(format!(
                "at most {MAX_PLUGINS} plugins may be installed"
            )));
        }
        if state.plugins.get(&id).is_some_and(|plugin| {
            plugin.versions.len() >= MAX_VERSIONS_PER_PLUGIN
                && !plugin
                    .versions
                    .iter()
                    .any(|record| record.content_hash == hash)
        }) {
            return Err(fail(format!(
                "{id} already has {MAX_VERSIONS_PER_PLUGIN} versions installed; remove old ones first"
            )));
        }

        // Write the file, atomically, if it is not already there.
        let path = self.package_path(&id, &hash);
        let created = !path.exists();
        if created {
            write_atomic(&path, bytes)?;
        }

        let updated_from = state
            .plugins
            .get(&id)
            .and_then(|plugin| {
                plugin
                    .versions
                    .iter()
                    .find(|record| record.content_hash == plugin.active)
            })
            .map(|record| record.version.clone());
        let mut next = state.clone();
        let record = VersionRecord {
            content_hash: hash.clone(),
            version: package.manifest.version.clone(),
            installed_at: now.to_string(),
            package_sha256: package.package_sha256.clone(),
        };
        match next.plugins.get_mut(&id) {
            Some(plugin) => {
                if !plugin
                    .versions
                    .iter()
                    .any(|existing| existing.content_hash == hash)
                {
                    plugin.versions.push(record);
                }
                // A grant carries to the new version only as far as the person
                // chose in this call; nothing is inherited silently.
                plugin.granted = grant;
                plugin.active = hash.clone();
            }
            None => {
                next.plugins.insert(
                    id.clone(),
                    PluginState {
                        active: hash.clone(),
                        enabled: true,
                        granted: grant,
                        versions: vec![record],
                        remembered: BTreeMap::new(),
                    },
                );
            }
        }
        next.version = STATE_VERSION;
        if let Err(error) = self.persist(&next) {
            if created {
                let _ = std::fs::remove_file(&path);
            }
            return Err(error);
        }
        *state = next;
        drop(state);
        self.forget(&id, None);
        Ok(InstallReport {
            plugin: id,
            version: package.manifest.version,
            content_hash: hash,
            updated_from,
            test,
        })
    }

    fn change(
        &self,
        id: &str,
        edit: impl FnOnce(&mut State) -> Result<(), AppError>,
    ) -> Result<(), AppError> {
        let mut state = self.lock_state()?;
        if !state.plugins.contains_key(id) {
            return Err(fail(format!("{id} is not installed")));
        }
        let mut next = state.clone();
        edit(&mut next)?;
        next.version = STATE_VERSION;
        self.persist(&next)?;
        *state = next;
        Ok(())
    }

    pub fn set_enabled(&self, id: &str, enabled: bool) -> Result<(), AppError> {
        self.change(id, |state| {
            state.plugins.get_mut(id).expect("checked").enabled = enabled;
            Ok(())
        })
    }

    pub fn set_grants(&self, id: &str, grant: &[Capability]) -> Result<(), AppError> {
        let (active, current) = {
            let state = self.lock_state()?;
            let plugin = state
                .plugins
                .get(id)
                .ok_or_else(|| fail(format!("{id} is not installed")))?;
            (plugin.active.clone(), plugin.granted.clone())
        };
        // What the plugin asked for. If its file cannot be read, a grant can only be
        // reduced, never widened: nothing is granted that cannot be shown.
        let declared = match self.load(id, &active) {
            Ok(loaded) => loaded.manifest.declared_capabilities()?,
            Err(_) => Vec::new(),
        };
        let mut allowed: Vec<Capability> = grant.to_vec();
        allowed.sort();
        allowed.dedup();
        if let Some(extra) = allowed
            .iter()
            .find(|capability| !declared.contains(capability) && !current.contains(capability))
        {
            return Err(fail(format!(
                "{} is not something {id} asked for",
                extra.id()
            )));
        }
        self.change(id, |state| {
            state.plugins.get_mut(id).expect("checked").granted = allowed;
            Ok(())
        })
    }

    /// Removes a plugin and every version of it. Documents that used it will say so.
    pub fn remove(&self, id: &str) -> Result<(), AppError> {
        if !valid_plugin_id(id) {
            return Err(fail("not a plugin id"));
        }
        self.change(id, |state| {
            state.plugins.remove(id);
            Ok(())
        })?;
        self.forget(id, None);
        // The state no longer names it, so a failure to delete the files leaves a
        // harmless orphan, not a plugin that is half there.
        let _ = std::fs::remove_dir_all(self.root.join("store").join(id));
        Ok(())
    }

    /// Removes one old version. The active version cannot be removed this way.
    pub fn remove_version(&self, id: &str, hash: &str) -> Result<(), AppError> {
        self.change(id, |state| {
            let plugin = state.plugins.get_mut(id).expect("checked");
            if plugin.active == hash {
                return Err(fail(
                    "the version in use cannot be removed; remove the plugin instead",
                ));
            }
            let before = plugin.versions.len();
            plugin.versions.retain(|record| record.content_hash != hash);
            if plugin.versions.len() == before {
                return Err(fail("that version is not installed"));
            }
            Ok(())
        })?;
        self.forget(id, Some(hash));
        let _ = std::fs::remove_file(self.package_path(id, hash));
        Ok(())
    }

    // ---- remembered values -------------------------------------------------------

    pub fn remembered(&self, id: &str, key: &str) -> BTreeMap<String, f64> {
        self.state
            .lock()
            .ok()
            .and_then(|state| {
                state
                    .plugins
                    .get(id)
                    .and_then(|plugin| plugin.remembered.get(key).cloned())
            })
            .unwrap_or_default()
    }

    /// Remembers the values last used for a filter or command, bounded in size.
    pub fn remember(
        &self,
        id: &str,
        key: &str,
        values: BTreeMap<String, f64>,
    ) -> Result<(), AppError> {
        if key.len() > 100 || values.len() > 32 || values.values().any(|value| !value.is_finite()) {
            return Err(fail("those values cannot be remembered"));
        }
        self.change(id, |state| {
            let plugin = state.plugins.get_mut(id).expect("checked");
            if plugin.remembered.len() >= MAX_REMEMBERED_VALUE_SETS
                && !plugin.remembered.contains_key(key)
            {
                return Err(fail("too many remembered settings"));
            }
            plugin.remembered.insert(key.to_string(), values);
            Ok(())
        })
    }

    // ---- testing -----------------------------------------------------------------

    /// Runs the self-test on the active version, as the manager's Test button does.
    pub fn test(&self, id: &str) -> Result<TestReport, AppError> {
        match self.active(id) {
            Resolution::Unavailable(why) => Err(AppError::PluginUnavailable {
                plugin: id.to_string(),
                reason: why.describe(id, ""),
            }),
            Resolution::Available(loaded) => match &loaded.compiled {
                None => Ok(TestReport {
                    plugin: id.to_string(),
                    passed: true,
                    filters: Vec::new(),
                }),
                Some(compiled) => {
                    let runtime = self.runtime().map_err(|error| fail(error.to_string()))?;
                    Ok(self_test(runtime, compiled, &loaded.manifest))
                }
            },
        }
    }
}

/// Runs every filter of a plugin on a test image: it must succeed, give the same
/// answer twice, and give the same answer whole and tiled.
pub fn self_test(runtime: &Runtime, compiled: &Compiled, manifest: &PluginManifest) -> TestReport {
    let image = test_image(48, 36);
    let allowance =
        limits::memory_allowance(manifest.limits.as_ref().map(|l| l.memory_mib), 8 << 30);
    let mut report = TestReport {
        plugin: manifest.id.clone(),
        passed: true,
        filters: Vec::new(),
    };
    for (index, decl) in manifest.filters.iter().enumerate() {
        let parameters = decl.default_parameters();
        let mut result = FilterTest {
            filter: decl.id.clone(),
            passed: false,
            message: String::new(),
            tiles: 0,
            fuel_used: 0,
        };
        let first = filter::apply(
            runtime,
            compiled,
            decl,
            index,
            &parameters,
            &image,
            allowance,
            16,
            None,
        );
        match first {
            Err(error) => result.message = format!("it failed on a test image: {error}"),
            Ok(first) => {
                result.tiles = first.tiles;
                result.fuel_used = first.fuel_used;
                let second = filter::apply(
                    runtime,
                    compiled,
                    decl,
                    index,
                    &parameters,
                    &image,
                    allowance,
                    16,
                    None,
                );
                match second {
                    Err(error) => result.message = format!("it failed the second time: {error}"),
                    Ok(second) => {
                        let identical =
                            first
                                .image
                                .pixels()
                                .iter()
                                .zip(second.image.pixels())
                                .all(|(a, b)| {
                                    a.red.to_bits() == b.red.to_bits()
                                        && a.green.to_bits() == b.green.to_bits()
                                        && a.blue.to_bits() == b.blue.to_bits()
                                        && a.alpha.to_bits() == b.alpha.to_bits()
                                });
                        if !identical {
                            result.message =
                                "it gave different results for the same input; a filter must be a pure function of its pixels and parameters".into();
                        } else {
                            match filter::verify_locality(
                                runtime, compiled, decl, index, &image, allowance, 16,
                            ) {
                                Err(error) => {
                                    result.message = format!("the locality check failed: {error}")
                                }
                                Ok(locality) if !locality.honest => {
                                    result.message = format!(
                                        "it declares {:?} but its result depends on where tile edges fall: {} pixels differ, first at {:?}",
                                        decl.locality, locality.differing_pixels, locality.first_difference
                                    );
                                }
                                Ok(_) => {
                                    result.passed = true;
                                    result.message = "ok".into();
                                }
                            }
                        }
                    }
                }
            }
        }
        report.passed &= result.passed;
        report.filters.push(result);
    }
    report
}

/// A comparable form of `1.2.3`.
fn version_key(version: &str) -> (u32, u32, u32) {
    let mut parts = version
        .split('.')
        .map(|part| part.parse::<u32>().unwrap_or(0));
    (
        parts.next().unwrap_or(0),
        parts.next().unwrap_or(0),
        parts.next().unwrap_or(0),
    )
}

fn read_state(root: &Path) -> Result<State, String> {
    let path = root.join(STATE_FILE);
    let metadata = match std::fs::metadata(&path) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(State::default()),
        Err(error) => return Err(format!("the plugin list cannot be read: {error}")),
    };
    if metadata.len() > MAX_STATE_BYTES {
        return Err("the plugin list is too large to be one".into());
    }
    let text = std::fs::read_to_string(&path)
        .map_err(|error| format!("the plugin list cannot be read: {error}"))?;
    let state: State = serde_json::from_str(&text)
        .map_err(|error| format!("the plugin list is damaged: {error}"))?;
    if state.version != STATE_VERSION {
        return Err(format!(
            "the plugin list is version {}, not {STATE_VERSION}",
            state.version
        ));
    }
    Ok(state)
}

/// Writes `bytes` to `path` by writing a sibling file and renaming it over, so a
/// crash leaves the old file or the new one and never a half-written one.
fn write_atomic(path: &Path, bytes: &[u8]) -> Result<(), AppError> {
    let io =
        |what: &str, error: std::io::Error| AppError::ProcessingFailure(format!("{what}: {error}"));
    let directory = path
        .parent()
        .ok_or_else(|| AppError::ProcessingFailure("a plugin path has no directory".into()))?;
    std::fs::create_dir_all(directory)
        .map_err(|error| io("could not create the plugin folder", error))?;
    let name = path
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or("file");
    let temporary = directory.join(format!(".{name}.{}.tmp", std::process::id()));
    std::fs::write(&temporary, bytes).map_err(|error| io("could not write the plugin", error))?;
    std::fs::rename(&temporary, path).map_err(|error| {
        let _ = std::fs::remove_file(&temporary);
        io("could not finish writing the plugin", error)
    })
}

// ---- the process-wide registry --------------------------------------------------------

static GLOBAL: RwLock<Option<Arc<PluginRegistry>>> = RwLock::new(None);

/// Where plugins live: under local application data, beside the settings.
pub fn default_root() -> PathBuf {
    std::env::var_os("LOCALAPPDATA")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("."))
        .join("PhotoForge")
        .join("plugins")
}

/// Installs the registry the renderer and the operations consult.
pub fn set_global(registry: Option<Arc<PluginRegistry>>) {
    if let Ok(mut slot) = GLOBAL.write() {
        *slot = registry;
    }
}

/// The registry in force, opened at the default location the first time.
pub fn global() -> Arc<PluginRegistry> {
    if let Some(registry) = GLOBAL.read().ok().and_then(|slot| slot.clone()) {
        return registry;
    }
    let mut slot = GLOBAL
        .write()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    Arc::clone(slot.get_or_insert_with(|| Arc::new(PluginRegistry::open(default_root()))))
}
