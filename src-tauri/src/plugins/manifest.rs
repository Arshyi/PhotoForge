//! The manifest of a `.photoforge-plugin` package, and everything it may declare.
//!
//! A plugin is **data first**. Its commands are lists of registered operations, its
//! panels are lists of rows, its tools are buttons that run a command, and its
//! filters are the only thing that executes code — a WebAssembly function that is
//! given pixels and parameters and returns pixels. There is no scripting: nothing
//! in a manifest is an expression, and nothing a plugin ships can run except
//! through a filter.
//!
//! Default authority is **none**. A manifest names the capabilities it wants; the
//! person installing it grants some or all; and every use is checked against the
//! grant. A capability that would hand a plugin the machine — the filesystem, the
//! network, a process, a shell, the registry — is not merely ungranted: it does not
//! exist, and a manifest that asks for one is refused outright.
//!
//! This is the format version 1 manifest. It is unrelated to the older
//! `domain::plugins::PluginManifest` (schema version 1, `planner` and
//! `restoration_engine` adapters), which describes components that PhotoForge
//! discovers but never runs, and which is unchanged. The two are told apart by
//! their first field: this one has `format`, that one has `schemaVersion`.
use crate::error::AppError;
use crate::operations::registry::OperationKind;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::HashSet;

pub const FORMAT: &str = "photoforge-plugin";
pub const FORMAT_VERSION: u32 = 1;
/// The version of the host interface (`docs/plugin-api.md`) a module is built for.
pub const API_VERSION: u32 = 1;

pub const MAX_MANIFEST_BYTES: usize = 64 * 1024;
pub const MAX_FILTERS: usize = 32;
pub const MAX_COMMANDS: usize = 32;
pub const MAX_PANELS: usize = 8;
pub const MAX_TOOLS: usize = 16;
pub const MAX_PARAMETERS: usize = 32;
pub const MAX_STEPS: usize = 50;
pub const MAX_ROWS: usize = 40;
pub const MAX_CHOICES: usize = 32;
pub const MAX_STEP_JSON_BYTES: usize = 4096;
pub const MAX_REQUESTED_MEMORY_MIB: u32 = 2048;
pub const MAX_LOCAL_RADIUS: u32 = 256;

fn invalid(message: impl Into<String>) -> AppError {
    AppError::InvalidPluginManifest(message.into())
}

// ---- capabilities ---------------------------------------------------------------

/// What a plugin can be allowed to do. This is the whole list.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
pub enum Capability {
    /// Its filters may be run on the pixels of a layer the user chose.
    #[serde(rename = "filter.pixels")]
    FilterPixels,
    /// Its panels may show facts about the open document, from a fixed list.
    #[serde(rename = "document.read")]
    DocumentRead,
    /// Its commands may issue registered operations, as a plugin: layer locks are
    /// respected and the whole command is one undoable transaction.
    #[serde(rename = "document.operations")]
    DocumentOperations,
    /// It may add panels.
    #[serde(rename = "ui.panel")]
    UiPanel,
    /// It may add entries to the tools menu.
    #[serde(rename = "ui.tool")]
    UiTool,
}

impl Capability {
    pub const ALL: [Capability; 5] = [
        Self::FilterPixels,
        Self::DocumentRead,
        Self::DocumentOperations,
        Self::UiPanel,
        Self::UiTool,
    ];

    pub const fn id(self) -> &'static str {
        match self {
            Self::FilterPixels => "filter.pixels",
            Self::DocumentRead => "document.read",
            Self::DocumentOperations => "document.operations",
            Self::UiPanel => "ui.panel",
            Self::UiTool => "ui.tool",
        }
    }

    pub const fn describe(self) -> &'static str {
        match self {
            Self::FilterPixels => "Run its filters on the pixels of layers you choose",
            Self::DocumentRead => "Show facts about the open document in its panels",
            Self::DocumentOperations => {
                "Change the document with its commands (undoable, and respecting locked layers)"
            }
            Self::UiPanel => "Add panels",
            Self::UiTool => "Add entries to the tools menu",
        }
    }

    /// Capabilities a manifest may never ask for, by namespace. Matched by prefix
    /// so a spelling a future version might add is refused, not ignored.
    pub const FORBIDDEN_NAMESPACES: [&'static str; 11] = [
        "filesystem",
        "network",
        "process",
        "shell",
        "registry",
        "env",
        "clipboard",
        "device",
        "native",
        "net",
        "fs",
    ];

    pub fn parse(text: &str) -> Result<Self, AppError> {
        if let Some(found) = Self::ALL
            .into_iter()
            .find(|capability| capability.id() == text)
        {
            return Ok(found);
        }
        let namespace = text.split('.').next().unwrap_or(text);
        if Self::FORBIDDEN_NAMESPACES.contains(&namespace) {
            return Err(invalid(format!(
                "the capability {text:?} does not exist. Plugins are never given the \
                 filesystem, the network, processes, a shell, the registry or the \
                 environment."
            )));
        }
        Err(invalid(format!("unknown capability {text:?}")))
    }
}

// ---- identifiers ------------------------------------------------------------------

/// `com.example.border`: two to eight dot-separated segments of lowercase letters,
/// digits and hyphens. `core` is reserved for the first-party operations.
pub fn valid_plugin_id(id: &str) -> bool {
    if id.len() < 3 || id.len() > 64 {
        return false;
    }
    let segments: Vec<&str> = id.split('.').collect();
    (2..=8).contains(&segments.len())
        && segments[0] != "core"
        // The id names a directory. On Windows `con.example` is the console, whatever
        // follows the dot, so a segment that is a device name is refused anywhere.
        && !segments.iter().any(|segment| is_device_name(segment))
        && segments.iter().all(|segment| {
            !segment.is_empty()
                && segment.len() <= 24
                && segment.as_bytes()[0].is_ascii_alphanumeric()
                && segment
                    .bytes()
                    .all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit() || byte == b'-')
        })
}

/// A name Windows reserves for a device.
pub fn is_device_name(name: &str) -> bool {
    let stem = name.split('.').next().unwrap_or("").to_ascii_lowercase();
    matches!(stem.as_str(), "con" | "prn" | "aux" | "nul")
        || (stem.len() == 4
            && (stem.starts_with("com") || stem.starts_with("lpt"))
            && stem.as_bytes()[3].is_ascii_digit())
}

/// `add_border`: a lowercase name within one plugin.
pub fn valid_local_id(id: &str) -> bool {
    !id.is_empty()
        && id.len() <= 48
        && id.as_bytes()[0].is_ascii_lowercase()
        && id
            .bytes()
            .all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit() || byte == b'_')
}

pub fn valid_version(version: &str) -> bool {
    let segments: Vec<&str> = version.split('.').collect();
    segments.len() == 3
        && segments.iter().all(|segment| {
            !segment.is_empty()
                && segment.len() <= 6
                && segment.bytes().all(|byte| byte.is_ascii_digit())
        })
}

pub fn valid_sha256(text: &str) -> bool {
    text.len() == 64
        && text
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
}

fn plain_text(label: &str, text: &str, min: usize, max: usize) -> Result<(), AppError> {
    let chars = text.chars().count();
    if chars < min || chars > max {
        return Err(invalid(format!(
            "{label} must contain {min} to {max} characters"
        )));
    }
    if text.chars().any(char::is_control) {
        return Err(invalid(format!(
            "{label} must not contain control characters"
        )));
    }
    Ok(())
}

// ---- declarations -----------------------------------------------------------------

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ModuleRef {
    /// Path of the WebAssembly module inside the package.
    pub path: String,
    /// SHA-256 of that module's bytes. It is the module's identity everywhere: in
    /// documents, in the render cache, and in the update check.
    pub sha256: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct LimitRequest {
    /// Linear memory the module asks for, in MiB. The host's own ceiling applies.
    pub memory_mib: u32,
}

/// What a filter reads, so the renderer can tile it correctly.
///
/// It is a *declaration the host does not take on trust*: the plugin manager's test
/// runs the filter whole and tiled and refuses one whose results differ.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum Locality {
    /// Each output pixel depends on the same pixel of the input and nothing else.
    Pointwise,
    /// Each output pixel depends on input pixels within `radius` of it.
    Local { radius: u32 },
    /// An output pixel may depend on any input pixel. The whole image is one call.
    Global,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case", deny_unknown_fields)]
pub enum ParamKind {
    Number {
        min: f64,
        max: f64,
        default: f64,
        #[serde(default)]
        step: Option<f64>,
    },
    Integer {
        min: i64,
        max: i64,
        default: i64,
    },
    Bool {
        default: bool,
    },
    /// An index into `options`.
    Choice {
        options: Vec<String>,
        default: u32,
    },
}

/// A parameter as it is written in a manifest.
///
/// `serde` cannot combine `flatten` with `deny_unknown_fields`, and a manifest that
/// silently ignored a misspelt `"maxx"` would be a manifest that did not mean what it
/// said. So a parameter is read into this flat, strict form and then turned into a
/// [`ParamKind`], refusing any field that does not belong to its type.
#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct RawParam {
    id: String,
    title: String,
    #[serde(default)]
    description: String,
    #[serde(rename = "type")]
    kind: String,
    min: Option<Value>,
    max: Option<Value>,
    default: Option<Value>,
    step: Option<Value>,
    options: Option<Vec<String>>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", try_from = "RawParam")]
pub struct ParamDecl {
    pub id: String,
    pub title: String,
    #[serde(default)]
    pub description: String,
    #[serde(flatten)]
    pub kind: ParamKind,
}

impl TryFrom<RawParam> for ParamDecl {
    type Error = String;

    fn try_from(raw: RawParam) -> Result<Self, String> {
        let label = raw.id.clone();
        let number = |value: &Option<Value>, field: &str| -> Result<f64, String> {
            value
                .as_ref()
                .and_then(Value::as_f64)
                .ok_or_else(|| format!("parameter {label} needs a numeric {field}"))
        };
        let integer = |value: &Option<Value>, field: &str| -> Result<i64, String> {
            value
                .as_ref()
                .and_then(Value::as_i64)
                .ok_or_else(|| format!("parameter {label} needs an integer {field}"))
        };
        let allowed = |present: &[(&str, bool)]| -> Result<(), String> {
            present
                .iter()
                .find(|(_, set)| *set)
                .map_or(Ok(()), |(field, _)| {
                    Err(format!(
                        "parameter {label} is a {} and does not take {field}",
                        raw.kind
                    ))
                })
        };
        let kind =
            match raw.kind.as_str() {
                "number" => {
                    allowed(&[("options", raw.options.is_some())])?;
                    ParamKind::Number {
                        min: number(&raw.min, "min")?,
                        max: number(&raw.max, "max")?,
                        default: number(&raw.default, "default")?,
                        step: match &raw.step {
                            None => None,
                            Some(_) => Some(number(&raw.step, "step")?),
                        },
                    }
                }
                "integer" => {
                    allowed(&[
                        ("step", raw.step.is_some()),
                        ("options", raw.options.is_some()),
                    ])?;
                    ParamKind::Integer {
                        min: integer(&raw.min, "min")?,
                        max: integer(&raw.max, "max")?,
                        default: integer(&raw.default, "default")?,
                    }
                }
                "bool" => {
                    allowed(&[
                        ("min", raw.min.is_some()),
                        ("max", raw.max.is_some()),
                        ("step", raw.step.is_some()),
                        ("options", raw.options.is_some()),
                    ])?;
                    ParamKind::Bool {
                        default: raw.default.as_ref().and_then(Value::as_bool).ok_or_else(
                            || format!("parameter {label} needs a true or false default"),
                        )?,
                    }
                }
                "choice" => {
                    allowed(&[
                        ("min", raw.min.is_some()),
                        ("max", raw.max.is_some()),
                        ("step", raw.step.is_some()),
                    ])?;
                    ParamKind::Choice {
                        options: raw
                            .options
                            .clone()
                            .ok_or_else(|| format!("parameter {label} needs its options"))?,
                        default: u32::try_from(integer(&raw.default, "default")?)
                            .map_err(|_| format!("parameter {label} has an invalid default"))?,
                    }
                }
                other => return Err(format!("parameter {label} has an unknown type {other:?}")),
            };
        Ok(Self {
            id: raw.id,
            title: raw.title,
            description: raw.description,
            kind,
        })
    }
}

impl ParamDecl {
    pub fn default_value(&self) -> f64 {
        match &self.kind {
            ParamKind::Number { default, .. } => *default,
            ParamKind::Integer { default, .. } => *default as f64,
            ParamKind::Bool { default } => f64::from(u8::from(*default)),
            ParamKind::Choice { default, .. } => f64::from(*default),
        }
    }

    /// Whether `value` is one this parameter accepts.
    pub fn accepts(&self, value: f64) -> bool {
        if !value.is_finite() {
            return false;
        }
        match &self.kind {
            ParamKind::Number { min, max, .. } => value >= *min && value <= *max,
            ParamKind::Integer { min, max, .. } => {
                value.fract() == 0.0 && value >= *min as f64 && value <= *max as f64
            }
            ParamKind::Bool { .. } => value == 0.0 || value == 1.0,
            ParamKind::Choice { options, .. } => {
                value.fract() == 0.0 && value >= 0.0 && (value as usize) < options.len()
            }
        }
    }

    fn validate(&self) -> Result<(), AppError> {
        if !valid_local_id(&self.id) {
            return Err(invalid(format!(
                "parameter id {:?} is not a valid identifier",
                self.id
            )));
        }
        plain_text("a parameter title", &self.title, 1, 60)?;
        plain_text("a parameter description", &self.description, 0, 200)?;
        match &self.kind {
            ParamKind::Number {
                min,
                max,
                default,
                step,
            } => {
                if !(min.is_finite() && max.is_finite() && default.is_finite()) || min >= max {
                    return Err(invalid(format!(
                        "parameter {} has an invalid range",
                        self.id
                    )));
                }
                if default < min || default > max {
                    return Err(invalid(format!(
                        "parameter {} has a default outside its range",
                        self.id
                    )));
                }
                if step.is_some_and(|step| !step.is_finite() || step <= 0.0 || step > max - min) {
                    return Err(invalid(format!(
                        "parameter {} has an invalid step",
                        self.id
                    )));
                }
            }
            ParamKind::Integer { min, max, default } => {
                if min >= max || default < min || default > max {
                    return Err(invalid(format!(
                        "parameter {} has an invalid range",
                        self.id
                    )));
                }
                // Parameters reach the module as f64, which is exact up to 2^53.
                if min.unsigned_abs() > (1 << 53) || max.unsigned_abs() > (1 << 53) {
                    return Err(invalid(format!("parameter {} range is too large", self.id)));
                }
            }
            ParamKind::Bool { .. } => {}
            ParamKind::Choice { options, default } => {
                if options.len() < 2 || options.len() > MAX_CHOICES {
                    return Err(invalid(format!(
                        "parameter {} must offer 2 to {MAX_CHOICES} choices",
                        self.id
                    )));
                }
                for option in options {
                    plain_text("a choice", option, 1, 40)?;
                }
                if *default as usize >= options.len() {
                    return Err(invalid(format!(
                        "parameter {} defaults to a missing choice",
                        self.id
                    )));
                }
            }
        }
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct FilterDecl {
    pub id: String,
    pub title: String,
    #[serde(default)]
    pub description: String,
    pub locality: Locality,
    /// Whether the filter is a pure function of its input and parameters. A filter
    /// that says `true` and is not is a defect the manager's test looks for.
    #[serde(default = "default_true")]
    pub deterministic: bool,
    #[serde(default)]
    pub parameters: Vec<ParamDecl>,
}

fn default_true() -> bool {
    true
}

impl FilterDecl {
    /// The values every parameter takes if the caller says nothing.
    pub fn default_parameters(&self) -> Vec<f64> {
        self.parameters
            .iter()
            .map(ParamDecl::default_value)
            .collect()
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct StepDecl {
    /// A registered operation id.
    pub op: String,
    /// Its parameters, in which `{"$param": "name"}` stands for a value the person
    /// gives when running the command and `"$self"` for this plugin's own id. There
    /// is nothing else: no expressions, no arithmetic, no conditions.
    #[serde(default)]
    pub params: Value,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct CommandDecl {
    pub id: String,
    pub title: String,
    #[serde(default)]
    pub description: String,
    #[serde(default)]
    pub parameters: Vec<ParamDecl>,
    pub steps: Vec<StepDecl>,
}

/// Facts about the open document a panel may show. A closed list: a panel cannot
/// ask for anything that is not here.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum HostFact {
    LayerCount,
    PixelLayerCount,
    CanvasWidth,
    CanvasHeight,
    Precision,
    ActiveLayerName,
    ActiveLayerKind,
    ActiveLayerOpacity,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum PanelRow {
    Text { text: String },
    Fact { label: String, fact: HostFact },
    Command { label: String, command: String },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct PanelDecl {
    pub id: String,
    pub title: String,
    pub rows: Vec<PanelRow>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ToolDecl {
    pub id: String,
    pub title: String,
    #[serde(default)]
    pub description: String,
    /// The command it runs. A tool is a menu entry, not a canvas gesture: a plugin
    /// cannot receive pointer events.
    pub command: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct PluginManifest {
    pub format: String,
    pub format_version: u32,
    pub id: String,
    pub name: String,
    pub version: String,
    pub api_version: u32,
    pub publisher: String,
    #[serde(default)]
    pub description: String,
    #[serde(default)]
    pub license: String,
    /// The module that implements the filters. Absent for a plugin that is only
    /// commands, panels and tools.
    #[serde(default)]
    pub entry: Option<ModuleRef>,
    #[serde(default)]
    pub capabilities: Vec<String>,
    #[serde(default)]
    pub limits: Option<LimitRequest>,
    #[serde(default)]
    pub filters: Vec<FilterDecl>,
    #[serde(default)]
    pub commands: Vec<CommandDecl>,
    #[serde(default)]
    pub panels: Vec<PanelDecl>,
    #[serde(default)]
    pub tools: Vec<ToolDecl>,
}

/// Operations a plugin's command may not issue: those that destroy work or change
/// what protects it. A person can still do any of them by hand.
pub const PLUGIN_DENIED_OPERATIONS: [OperationKind; 4] = [
    OperationKind::Delete,
    OperationKind::Flatten,
    OperationKind::MergeDown,
    OperationKind::SetLocked,
];

impl PluginManifest {
    pub fn from_json(json: &str) -> Result<Self, AppError> {
        if json.len() > MAX_MANIFEST_BYTES {
            return Err(invalid("the manifest exceeds 64 KiB"));
        }
        let manifest: Self = serde_json::from_str(json)
            .map_err(|error| invalid(format!("the manifest is not valid: {error}")))?;
        manifest.validate()?;
        Ok(manifest)
    }

    /// The capabilities this manifest declares, parsed.
    pub fn declared_capabilities(&self) -> Result<Vec<Capability>, AppError> {
        let mut seen = HashSet::new();
        let mut out = Vec::new();
        for text in &self.capabilities {
            let capability = Capability::parse(text)?;
            if !seen.insert(capability) {
                return Err(invalid(format!("the capability {text:?} is listed twice")));
            }
            out.push(capability);
        }
        out.sort();
        Ok(out)
    }

    pub fn filter(&self, id: &str) -> Option<(usize, &FilterDecl)> {
        self.filters
            .iter()
            .enumerate()
            .find(|(_, filter)| filter.id == id)
    }

    pub fn command(&self, id: &str) -> Option<&CommandDecl> {
        self.commands.iter().find(|command| command.id == id)
    }

    pub fn validate(&self) -> Result<(), AppError> {
        if self.format != FORMAT {
            return Err(invalid(format!("format must be {FORMAT:?}")));
        }
        if self.format_version != FORMAT_VERSION {
            return Err(invalid(format!(
                "this build reads plugin format {FORMAT_VERSION}, not {}",
                self.format_version
            )));
        }
        if self.api_version != API_VERSION {
            return Err(invalid(format!(
                "this plugin was built for host interface {}, and this build provides {API_VERSION}",
                self.api_version
            )));
        }
        if !valid_plugin_id(&self.id) {
            return Err(invalid(
                "id must be dot-separated lowercase segments such as com.example.border, and \
                 may not begin with core.",
            ));
        }
        plain_text("name", &self.name, 1, 60)?;
        plain_text("publisher", &self.publisher, 1, 80)?;
        plain_text("description", &self.description, 0, 500)?;
        plain_text("license", &self.license, 0, 80)?;
        if !valid_version(&self.version) {
            return Err(invalid(
                "version must be three numeric segments, such as 1.0.0",
            ));
        }

        let capabilities = self.declared_capabilities()?;
        let has = |capability: Capability| capabilities.contains(&capability);

        if let Some(limits) = &self.limits {
            if limits.memory_mib == 0 || limits.memory_mib > MAX_REQUESTED_MEMORY_MIB {
                return Err(invalid(format!(
                    "limits.memoryMib must be 1 to {MAX_REQUESTED_MEMORY_MIB}"
                )));
            }
        }
        if let Some(entry) = &self.entry {
            validate_package_path(&entry.path)?;
            if !valid_sha256(&entry.sha256) {
                return Err(invalid(
                    "entry.sha256 must be 64 lowercase hexadecimal characters",
                ));
            }
        }

        // Counts first, so a hostile manifest cannot make the loops below long.
        for (label, count, limit) in [
            ("filters", self.filters.len(), MAX_FILTERS),
            ("commands", self.commands.len(), MAX_COMMANDS),
            ("panels", self.panels.len(), MAX_PANELS),
            ("tools", self.tools.len(), MAX_TOOLS),
        ] {
            if count > limit {
                return Err(invalid(format!("at most {limit} {label} may be declared")));
            }
        }

        // What each kind of declaration needs, so a capability is never exercised
        // without having been asked for and shown.
        if !self.filters.is_empty() {
            if !has(Capability::FilterPixels) {
                return Err(invalid("filters need the filter.pixels capability"));
            }
            if self.entry.is_none() {
                return Err(invalid("filters need an entry module to run"));
            }
        }
        if self.entry.is_some() && self.filters.is_empty() {
            return Err(invalid(
                "an entry module with no filters would never be run",
            ));
        }
        if !self.commands.is_empty() && !has(Capability::DocumentOperations) {
            return Err(invalid("commands need the document.operations capability"));
        }
        if !self.panels.is_empty() && !has(Capability::UiPanel) {
            return Err(invalid("panels need the ui.panel capability"));
        }
        if !self.tools.is_empty() && !has(Capability::UiTool) {
            return Err(invalid("tools need the ui.tool capability"));
        }

        let mut filter_ids = HashSet::new();
        for filter in &self.filters {
            if !valid_local_id(&filter.id) || !filter_ids.insert(filter.id.as_str()) {
                return Err(invalid(format!(
                    "filter id {:?} is invalid or repeated",
                    filter.id
                )));
            }
            plain_text("a filter title", &filter.title, 1, 60)?;
            plain_text("a filter description", &filter.description, 0, 300)?;
            // A filter that is not a pure function of its input cannot be tiled, cached
            // or re-rendered to the same picture, which is what a document needs of it.
            if !filter.deterministic {
                return Err(invalid(format!(
                    "filter {} declares itself non-deterministic; interface {API_VERSION} supports only                      filters that are a pure function of their pixels and parameters",
                    filter.id
                )));
            }
            if let Locality::Local { radius } = filter.locality {
                if radius == 0 || radius > MAX_LOCAL_RADIUS {
                    return Err(invalid(format!(
                        "filter {} declares a radius outside 1 to {MAX_LOCAL_RADIUS}; use \
                         pointwise for none and global for more",
                        filter.id
                    )));
                }
            }
            validate_parameters(&filter.parameters)?;
        }

        let mut command_ids = HashSet::new();
        for command in &self.commands {
            if !valid_local_id(&command.id) || !command_ids.insert(command.id.as_str()) {
                return Err(invalid(format!(
                    "command id {:?} is invalid or repeated",
                    command.id
                )));
            }
            plain_text("a command title", &command.title, 1, 60)?;
            plain_text("a command description", &command.description, 0, 300)?;
            validate_parameters(&command.parameters)?;
            self.validate_steps(command)?;
        }

        let mut panel_ids = HashSet::new();
        for panel in &self.panels {
            if !valid_local_id(&panel.id) || !panel_ids.insert(panel.id.as_str()) {
                return Err(invalid(format!(
                    "panel id {:?} is invalid or repeated",
                    panel.id
                )));
            }
            plain_text("a panel title", &panel.title, 1, 60)?;
            if panel.rows.is_empty() || panel.rows.len() > MAX_ROWS {
                return Err(invalid(format!(
                    "panel {} must have 1 to {MAX_ROWS} rows",
                    panel.id
                )));
            }
            for row in &panel.rows {
                match row {
                    PanelRow::Text { text } => plain_text("panel text", text, 1, 300)?,
                    PanelRow::Fact { label, .. } => {
                        plain_text("a fact label", label, 1, 60)?;
                        if !has(Capability::DocumentRead) {
                            return Err(invalid(
                                "showing document facts needs the document.read capability",
                            ));
                        }
                    }
                    PanelRow::Command { label, command } => {
                        plain_text("a button label", label, 1, 60)?;
                        if !command_ids.contains(command.as_str()) {
                            return Err(invalid(format!(
                                "panel button names an unknown command {command:?}"
                            )));
                        }
                    }
                }
            }
        }

        let mut tool_ids = HashSet::new();
        for tool in &self.tools {
            if !valid_local_id(&tool.id) || !tool_ids.insert(tool.id.as_str()) {
                return Err(invalid(format!(
                    "tool id {:?} is invalid or repeated",
                    tool.id
                )));
            }
            plain_text("a tool title", &tool.title, 1, 60)?;
            plain_text("a tool description", &tool.description, 0, 300)?;
            if !command_ids.contains(tool.command.as_str()) {
                return Err(invalid(format!(
                    "tool {} names an unknown command {:?}",
                    tool.id, tool.command
                )));
            }
        }
        Ok(())
    }

    fn validate_steps(&self, command: &CommandDecl) -> Result<(), AppError> {
        if command.steps.is_empty() || command.steps.len() > MAX_STEPS {
            return Err(invalid(format!(
                "command {} must have 1 to {MAX_STEPS} steps",
                command.id
            )));
        }
        for (index, step) in command.steps.iter().enumerate() {
            let at = |reason: &str| {
                invalid(format!(
                    "command {} step {}: {reason}",
                    command.id,
                    index + 1
                ))
            };
            let kind = OperationKind::from_id(&step.op)
                .ok_or_else(|| at(&format!("{:?} is not a registered operation", step.op)))?;
            if PLUGIN_DENIED_OPERATIONS.contains(&kind) {
                return Err(at(&format!(
                    "{} is not available to plugins: it destroys work or changes what protects it",
                    step.op
                )));
            }
            if serde_json::to_vec(&step.params)
                .map_or(true, |bytes| bytes.len() > MAX_STEP_JSON_BYTES)
            {
                return Err(at("its parameters are too large"));
            }
            let declared: HashSet<&str> =
                command.parameters.iter().map(|p| p.id.as_str()).collect();
            walk_template(&step.params, None, 0, &declared).map_err(|reason| at(&reason))?;
            if kind == OperationKind::ApplyPluginFilter {
                // A plugin runs its own filters. Naming another plugin would let one
                // plugin spend the authority another was granted.
                let params = step
                    .params
                    .as_object()
                    .ok_or_else(|| at("parameters must be an object"))?;
                if params.get("plugin") != Some(&Value::String("$self".into())) {
                    return Err(at(
                        "a plugin may only apply its own filters (\"plugin\": \"$self\")",
                    ));
                }
                let filter = params
                    .get("filter")
                    .and_then(Value::as_str)
                    .ok_or_else(|| at("a filter step must name its filter"))?;
                if self.filter(filter).is_none() {
                    return Err(at(&format!(
                        "it names a filter {filter:?} this plugin does not declare"
                    )));
                }
            }
        }
        Ok(())
    }
}

fn validate_parameters(parameters: &[ParamDecl]) -> Result<(), AppError> {
    if parameters.len() > MAX_PARAMETERS {
        return Err(invalid(format!(
            "at most {MAX_PARAMETERS} parameters may be declared"
        )));
    }
    let mut seen = HashSet::new();
    for parameter in parameters {
        parameter.validate()?;
        if !seen.insert(parameter.id.as_str()) {
            return Err(invalid(format!(
                "parameter {:?} is declared twice",
                parameter.id
            )));
        }
    }
    Ok(())
}

/// Checks a step's parameters contain only literals, `{"$param": id}` for a declared
/// parameter, and `"$self"` as the value of a `plugin` key.
fn walk_template(
    value: &Value,
    key: Option<&str>,
    depth: usize,
    declared: &HashSet<&str>,
) -> Result<(), String> {
    if depth > 8 {
        return Err("its parameters are nested too deeply".into());
    }
    match value {
        Value::String(text) if text.starts_with('$') => {
            if text == "$self" && key == Some("plugin") {
                Ok(())
            } else {
                Err(format!("{text:?} is not a placeholder: only \"$self\" (as \"plugin\") and {{\"$param\": name}} exist"))
            }
        }
        Value::Array(items) => items
            .iter()
            .try_for_each(|item| walk_template(item, None, depth + 1, declared)),
        Value::Object(map) => {
            if let Some(name) = map.get("$param") {
                let name = name.as_str().ok_or("$param must name a parameter")?;
                if map.len() != 1 {
                    return Err("a $param placeholder may have no other keys".into());
                }
                return if declared.contains(name) {
                    Ok(())
                } else {
                    Err(format!(
                        "$param names {name:?}, which the command does not declare"
                    ))
                };
            }
            if let Some(bad) = map.keys().find(|key| key.starts_with('$')) {
                return Err(format!("{bad:?} is not a placeholder"));
            }
            map.iter()
                .try_for_each(|(key, item)| walk_template(item, Some(key), depth + 1, declared))
        }
        _ => Ok(()),
    }
}

/// A path inside a package: relative, forward slashes, plain characters, no
/// traversal, no drive, no stream, no reserved device name.
pub fn validate_package_path(path: &str) -> Result<(), AppError> {
    let bad = |reason: &str| invalid(format!("the path {path:?} {reason}"));
    if path.is_empty() || path.len() > 128 {
        return Err(bad("must contain 1 to 128 characters"));
    }
    if path.starts_with('/') || path.contains('\\') || path.contains(':') {
        return Err(bad(
            "must be relative, with forward slashes and no drive or stream",
        ));
    }
    for segment in path.split('/') {
        if segment.is_empty() || segment == "." || segment == ".." {
            return Err(bad("must not contain empty, '.' or '..' segments"));
        }
        if segment.ends_with('.') || segment.ends_with(' ') {
            return Err(bad("must not have a segment ending in a dot or space"));
        }
        if !segment
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || b"-_. ".contains(&byte))
        {
            return Err(bad("may contain only letters, digits and - _ . and spaces"));
        }
        if is_device_name(segment) {
            return Err(bad("uses a reserved device name"));
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn base() -> Value {
        json!({
            "format": "photoforge-plugin", "formatVersion": 1, "apiVersion": 1,
            "id": "com.example.border", "name": "Border", "version": "1.0.0",
            "publisher": "Example", "description": "Adds a border.", "license": "MIT",
            "entry": { "path": "plugin.wasm", "sha256": "a".repeat(64) },
            "capabilities": ["filter.pixels"],
            "filters": [{
                "id": "add_border", "title": "Add border",
                "locality": { "kind": "pointwise" },
                "parameters": [{ "id": "width", "title": "Width", "type": "integer",
                                 "min": 0, "max": 100, "default": 8 }]
            }]
        })
    }

    fn parse(value: &Value) -> Result<PluginManifest, AppError> {
        PluginManifest::from_json(&value.to_string())
    }

    fn reason(value: &Value) -> String {
        parse(value).unwrap_err().to_string()
    }

    #[test]
    fn a_valid_manifest_parses_and_defaults_are_filled() {
        let manifest = parse(&base()).unwrap();
        assert_eq!(manifest.filters[0].default_parameters(), vec![8.0]);
        assert!(manifest.filters[0].deterministic);
        assert_eq!(
            manifest.declared_capabilities().unwrap(),
            vec![Capability::FilterPixels]
        );
    }

    #[test]
    fn authority_that_hands_over_the_machine_does_not_exist() {
        for name in [
            "filesystem.read",
            "filesystem.write",
            "network.http",
            "network.connect",
            "process.spawn",
            "shell.exec",
            "registry.read",
            "env.read",
            "clipboard.read",
            "fs.read",
            "net.fetch",
            "native.dll",
            "device.camera",
        ] {
            let mut value = base();
            value["capabilities"] = json!(["filter.pixels", name]);
            let message = reason(&value);
            assert!(message.contains("does not exist"), "{name}: {message}");
        }
        let mut value = base();
        value["capabilities"] = json!(["filter.pixels", "time.now"]);
        assert!(reason(&value).contains("unknown capability"));
        // A look-alike spelling is not the capability.
        value["capabilities"] = json!(["Filter.Pixels"]);
        assert!(parse(&value).is_err());
    }

    #[test]
    fn default_authority_is_none() {
        let mut value = base();
        value["capabilities"] = json!([]);
        assert!(reason(&value).contains("filter.pixels"));
        let mut value = base();
        value["commands"] = json!([{ "id": "go", "title": "Go",
            "steps": [{ "op": "core.layer.add_group", "params": {} }] }]);
        assert!(reason(&value).contains("document.operations"));
        let mut value = base();
        value["panels"] =
            json!([{ "id": "p", "title": "P", "rows": [{ "kind": "text", "text": "hi" }] }]);
        assert!(reason(&value).contains("ui.panel"));
        let mut value = base();
        value["capabilities"] = json!(["filter.pixels", "ui.panel"]);
        value["panels"] = json!([{ "id": "p", "title": "P",
            "rows": [{ "kind": "fact", "label": "Layers", "fact": "layer_count" }] }]);
        assert!(reason(&value).contains("document.read"));
    }

    #[test]
    fn unknown_fields_and_wrong_versions_are_refused() {
        let mut value = base();
        value["postInstall"] = json!("run.exe");
        assert!(parse(&value).is_err());
        let mut value = base();
        value["apiVersion"] = json!(2);
        assert!(reason(&value).contains("host interface"));
        let mut value = base();
        value["formatVersion"] = json!(2);
        assert!(reason(&value).contains("format"));
        let mut value = base();
        value["format"] = json!("something-else");
        assert!(parse(&value).is_err());
    }

    #[test]
    fn identifiers_are_strict() {
        for good in ["com.example.border", "photoforge.example.x1", "a-b.c-d"] {
            assert!(valid_plugin_id(good), "{good}");
        }
        for bad in [
            "",
            "ab",
            "core.layer.set_opacity",
            "nodots",
            "Com.Example",
            "com..example",
            "com.example.",
            ".com.example",
            "com.exa mple",
            "com.example/../x",
            "com._x",
            &"a.".repeat(40),
        ] {
            assert!(!valid_plugin_id(bad), "{bad:?}");
        }
        assert!(valid_local_id("add_border") && !valid_local_id("Add") && !valid_local_id("1a"));
        assert!(valid_version("1.2.3") && !valid_version("1.2") && !valid_version("1.2.x"));
        assert!(
            valid_sha256(&"0".repeat(64))
                && !valid_sha256(&"G".repeat(64))
                && !valid_sha256(&"A".repeat(64))
        );
    }

    #[test]
    fn package_paths_cannot_escape_or_name_devices() {
        for good in ["plugin.wasm", "dir/plugin.wasm", "a b/c-d_e.wasm"] {
            validate_package_path(good).unwrap();
        }
        for bad in [
            "",
            "/abs",
            "..",
            "../x",
            "a/../b",
            "a//b",
            "a\\b",
            "C:/x",
            "x:stream",
            "./x",
            "a/./b",
            "CON",
            "nul.wasm",
            "com1.wasm",
            "LPT9",
            "dir/aux.txt",
            "trailingdot.",
            "trailingspace ",
            "unicode\u{e9}.wasm",
            "tab\t.wasm",
            &"x".repeat(129),
        ] {
            assert!(validate_package_path(bad).is_err(), "{bad:?}");
        }
    }

    #[test]
    fn parameters_are_bounded_and_checked_against_their_declaration() {
        let number = ParamDecl {
            id: "x".into(),
            title: "X".into(),
            description: String::new(),
            kind: ParamKind::Number {
                min: 0.0,
                max: 1.0,
                default: 0.5,
                step: None,
            },
        };
        assert!(number.accepts(0.0) && number.accepts(1.0) && !number.accepts(1.5));
        assert!(!number.accepts(f64::NAN) && !number.accepts(f64::INFINITY));
        let integer = ParamDecl {
            kind: ParamKind::Integer {
                min: -3,
                max: 3,
                default: 0,
            },
            ..number.clone()
        };
        assert!(integer.accepts(2.0) && !integer.accepts(2.5) && !integer.accepts(4.0));
        let boolean = ParamDecl {
            kind: ParamKind::Bool { default: true },
            ..number.clone()
        };
        assert!(boolean.accepts(0.0) && boolean.accepts(1.0) && !boolean.accepts(2.0));
        let choice = ParamDecl {
            kind: ParamKind::Choice {
                options: vec!["a".into(), "b".into()],
                default: 1,
            },
            ..number
        };
        assert!(choice.accepts(1.0) && !choice.accepts(2.0) && !choice.accepts(-1.0));
        assert_eq!(choice.default_value(), 1.0);

        let mut value = base();
        value["filters"][0]["parameters"][0]["default"] = json!(101);
        assert!(parse(&value).is_err(), "a default outside its range");
        let mut value = base();
        value["filters"][0]["parameters"][0]["min"] = json!(5);
        value["filters"][0]["parameters"][0]["max"] = json!(5);
        assert!(parse(&value).is_err(), "an empty range");
        let mut value = base();
        let many: Vec<Value> = (0..33)
            .map(|i| {
                json!({ "id": format!("p{i}"), "title": "P",
            "type": "bool", "default": false })
            })
            .collect();
        value["filters"][0]["parameters"] = json!(many);
        assert!(parse(&value).is_err(), "too many parameters");
    }

    #[test]
    fn locality_is_declared_honestly() {
        let mut value = base();
        value["filters"][0]["locality"] = json!({ "kind": "local", "radius": 3 });
        assert!(parse(&value).is_ok());
        for radius in [0, 257] {
            value["filters"][0]["locality"] = json!({ "kind": "local", "radius": radius });
            assert!(parse(&value).is_err(), "radius {radius}");
        }
        value["filters"][0]["locality"] = json!({ "kind": "global" });
        assert!(parse(&value).is_ok());
        value["filters"][0]["locality"] = json!({ "kind": "nonlocal" });
        assert!(parse(&value).is_err());
    }

    fn with_command(steps: Value, parameters: Value) -> Value {
        let mut value = base();
        value["capabilities"] = json!(["filter.pixels", "document.operations"]);
        value["commands"] =
            json!([{ "id": "go", "title": "Go", "parameters": parameters, "steps": steps }]);
        value
    }

    #[test]
    fn commands_are_lists_of_registered_operations_with_placeholders_and_nothing_else() {
        let ok = with_command(
            json!([
                { "op": "core.layer.add_pixel", "params": { "name": "Border" } },
                { "op": "core.plugin.apply_filter", "params": {
                    "selector": { "type": "last_created" }, "plugin": "$self", "filter": "add_border",
                    "parameters": { "width": { "$param": "w" } } } }
            ]),
            json!([{ "id": "w", "title": "W", "type": "integer", "min": 0, "max": 9, "default": 2 }]),
        );
        parse(&ok).unwrap();

        let cases: Vec<(&str, Value)> = vec![
            (
                "not a registered",
                json!([{ "op": "core.layer.explode", "params": {} }]),
            ),
            (
                "not a registered",
                json!([{ "op": "plugin.other.thing", "params": {} }]),
            ),
            (
                "not available to plugins",
                json!([{ "op": "core.layer.delete", "params": { "selector": { "type": "active" } } }]),
            ),
            (
                "not available to plugins",
                json!([{ "op": "core.document.flatten", "params": {} }]),
            ),
            (
                "not available to plugins",
                json!([{ "op": "core.layer.merge_down", "params": { "selector": { "type": "active" } } }]),
            ),
            (
                "not available to plugins",
                json!([{ "op": "core.layer.set_locked", "params": { "selector": { "type": "active" }, "locked": false } }]),
            ),
            (
                "not a placeholder",
                json!([{ "op": "core.layer.rename", "params": { "selector": { "type": "active" }, "name": "$env" } }]),
            ),
            (
                "not a placeholder",
                json!([{ "op": "core.layer.rename", "params": { "selector": { "type": "active" }, "name": { "$eval": "1+1" } } }]),
            ),
            (
                "does not declare",
                json!([{ "op": "core.layer.rename", "params": { "selector": { "type": "active" }, "name": { "$param": "ghost" } } }]),
            ),
            (
                "only apply its own",
                json!([{ "op": "core.plugin.apply_filter", "params": { "selector": { "type": "active" }, "plugin": "com.evil.x", "filter": "f" } }]),
            ),
            (
                "does not declare",
                json!([{ "op": "core.plugin.apply_filter", "params": { "selector": { "type": "active" }, "plugin": "$self", "filter": "nope" } }]),
            ),
        ];
        for (expect, steps) in cases {
            let message = reason(&with_command(steps.clone(), json!([])));
            assert!(message.contains(expect), "{steps}: {message}");
        }
        // "$self" anywhere but as the plugin is not a placeholder either.
        let message = reason(&with_command(
            json!([{ "op": "core.layer.rename", "params": { "selector": { "type": "active" }, "name": "$self" } }]),
            json!([]),
        ));
        assert!(message.contains("not a placeholder"), "{message}");
        // An empty command, and one that is too long.
        assert!(parse(&with_command(json!([]), json!([]))).is_err());
        let long: Vec<Value> = (0..51)
            .map(|_| json!({ "op": "core.layer.add_group", "params": {} }))
            .collect();
        assert!(parse(&with_command(json!(long), json!([]))).is_err());
        // Placeholders cannot be nested without bound.
        let mut deep = json!("x");
        for _ in 0..12 {
            deep = json!([deep]);
        }
        assert!(parse(&with_command(
            json!([{ "op": "core.layer.add_group", "params": { "name": deep } }]),
            json!([])
        ))
        .is_err());
    }

    #[test]
    fn panels_and_tools_may_only_name_what_exists() {
        let mut value = base();
        value["capabilities"] = json!([
            "filter.pixels",
            "document.operations",
            "ui.panel",
            "ui.tool",
            "document.read"
        ]);
        value["commands"] = json!([{ "id": "go", "title": "Go",
            "steps": [{ "op": "core.layer.add_group", "params": {} }] }]);
        value["panels"] = json!([{ "id": "info", "title": "Info", "rows": [
            { "kind": "text", "text": "Hello" },
            { "kind": "fact", "label": "Layers", "fact": "layer_count" },
            { "kind": "command", "label": "Go", "command": "go" }] }]);
        value["tools"] = json!([{ "id": "tool", "title": "Tool", "command": "go" }]);
        parse(&value).unwrap();

        let mut bad = value.clone();
        bad["panels"][0]["rows"][2]["command"] = json!("missing");
        assert!(reason(&bad).contains("unknown command"));
        let mut bad = value.clone();
        bad["tools"][0]["command"] = json!("missing");
        assert!(reason(&bad).contains("unknown command"));
        let mut bad = value.clone();
        // A fact the host does not offer is not a fact.
        bad["panels"][0]["rows"][1]["fact"] = json!("every_layer_name");
        assert!(parse(&bad).is_err());
        let mut bad = value.clone();
        bad["panels"][0]["rows"] = json!([]);
        assert!(parse(&bad).is_err());
    }

    #[test]
    fn a_module_and_its_filters_come_together() {
        let mut value = base();
        value.as_object_mut().unwrap().remove("entry");
        assert!(reason(&value).contains("entry module"));
        let mut value = base();
        value["filters"] = json!([]);
        assert!(reason(&value).contains("never be run"));
        let mut value = base();
        value["entry"]["sha256"] = json!("nothex");
        assert!(parse(&value).is_err());
        let mut value = base();
        value["entry"]["path"] = json!("../plugin.wasm");
        assert!(parse(&value).is_err());
        let mut value = base();
        value["limits"] = json!({ "memoryMib": 4096 });
        assert!(parse(&value).is_err());
        value["limits"] = json!({ "memoryMib": 0 });
        assert!(parse(&value).is_err());
    }

    #[test]
    fn a_manifest_of_commands_alone_needs_no_module() {
        let value = json!({
            "format": "photoforge-plugin", "formatVersion": 1, "apiVersion": 1,
            "id": "com.example.panel", "name": "Panel", "version": "0.1.0", "publisher": "Example",
            "capabilities": ["ui.panel", "document.read"],
            "panels": [{ "id": "p", "title": "P", "rows": [
                { "kind": "fact", "label": "Width", "fact": "canvas_width" }] }]
        });
        let manifest = parse(&value).unwrap();
        assert!(manifest.entry.is_none() && manifest.filters.is_empty());
    }

    #[test]
    fn text_is_bounded_and_free_of_control_characters() {
        let mut value = base();
        value["name"] = json!("x".repeat(61));
        assert!(parse(&value).is_err());
        let mut value = base();
        value["name"] = json!("Bad\u{0}name");
        assert!(parse(&value).is_err());
        let mut value = base();
        value["publisher"] = json!("");
        assert!(parse(&value).is_err());
        let huge = " ".repeat(MAX_MANIFEST_BYTES + 1);
        assert!(PluginManifest::from_json(&huge).is_err());
    }
}
