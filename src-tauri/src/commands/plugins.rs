//! Commands for managing plugins and running their commands.
//!
//! Nothing here runs a plugin's code. Installing compiles a module and self-tests
//! its filters; running a *command* issues registered operations through the same
//! transaction engine as everything else; a filter runs only when an operation or a
//! render asks for it, through `plugins::apply`.
use crate::application::AppState;
use crate::domain::EditOperation;
use crate::error::AppError;
use crate::infrastructure::local_path::validated_local_path;
use crate::layers::LayerDocument;
use crate::mask::MaskSnapshot;
use crate::operations::{execute, OperationCall, Origin, TransactionRequest, TransactionResult};
use crate::plugins::document::{operation_requirements, requirements, statuses, RequirementStatus};
use crate::plugins::manifest::{Capability, ParamKind, PluginManifest};
use crate::plugins::package::MAX_PACKAGE_BYTES;
use crate::plugins::store::{
    global, Inspection, InstallReport, PluginSummary, Resolution, TestReport,
};
use crate::plugins::Runtime;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::collections::BTreeMap;
use std::io::Read;
use std::path::Path;
use tauri::State;

fn capability_list(ids: &[String]) -> Result<Vec<Capability>, AppError> {
    ids.iter().map(|id| Capability::parse(id)).collect()
}

/// Reads a package the person chose, bounded, and nothing else about the path.
fn read_chosen_package(path: &str) -> Result<Vec<u8>, AppError> {
    let checked = validated_local_path(path)?;
    if checked
        .extension()
        .and_then(|extension| extension.to_str())
        .is_none_or(|extension| !extension.eq_ignore_ascii_case("photoforge-plugin"))
    {
        return Err(AppError::InvalidPluginManifest(
            "a plugin package ends in .photoforge-plugin".into(),
        ));
    }
    let metadata = std::fs::metadata(&checked).map_err(|error| {
        AppError::InvalidPluginManifest(format!("the package cannot be read: {error}"))
    })?;
    if !metadata.is_file() || metadata.len() > MAX_PACKAGE_BYTES {
        return Err(AppError::InvalidPluginManifest(
            "the package is not a file, or is larger than 48 MiB".into(),
        ));
    }
    // Through a hard cap as well as the size check above, so a file that grows between
    // the two is still not read past the limit.
    let mut bytes = Vec::new();
    std::fs::File::open(Path::new(&checked))
        .and_then(|file| file.take(MAX_PACKAGE_BYTES + 1).read_to_end(&mut bytes))
        .map_err(|error| {
            AppError::InvalidPluginManifest(format!("the package cannot be read: {error}"))
        })?;
    if bytes.len() as u64 > MAX_PACKAGE_BYTES {
        return Err(AppError::InvalidPluginManifest(
            "the package is not a file, or is larger than 48 MiB".into(),
        ));
    }
    Ok(bytes)
}

fn now() -> String {
    use crate::operations::support::{Clock, SystemClock};
    SystemClock.now()
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PluginStatus {
    /// Whether this build can run plugins at all.
    pub runtime_available: bool,
    pub plugin_folder: String,
    /// A problem found reading the list of installed plugins, if any.
    pub warning: Option<String>,
    pub plugins: Vec<PluginSummary>,
}

#[tauri::command]
pub async fn list_plugins() -> Result<PluginStatus, AppError> {
    tauri::async_runtime::spawn_blocking(|| {
        let registry = global();
        Ok(PluginStatus {
            runtime_available: Runtime::available(),
            plugin_folder: registry.root().to_string_lossy().into_owned(),
            warning: registry.state_warning(),
            plugins: registry.list(),
        })
    })
    .await
    .map_err(|_| AppError::ProcessingFailure("the plugin worker stopped".into()))?
}

/// Describes a package: everything installing it would allow. Changes nothing.
#[tauri::command]
pub async fn inspect_plugin_package(path: String) -> Result<Inspection, AppError> {
    tauri::async_runtime::spawn_blocking(move || global().inspect(&read_chosen_package(&path)?))
        .await
        .map_err(|_| AppError::ProcessingFailure("the plugin worker stopped".into()))?
}

/// Installs the package the person inspected, granting what they chose.
#[tauri::command]
pub async fn install_plugin_package(
    path: String,
    expected_hash: String,
    grant: Vec<String>,
) -> Result<InstallReport, AppError> {
    let grant = capability_list(&grant)?;
    tauri::async_runtime::spawn_blocking(move || {
        let bytes = read_chosen_package(&path)?;
        global().install(&bytes, Some(&expected_hash), &grant, &now())
    })
    .await
    .map_err(|_| AppError::ProcessingFailure("the plugin worker stopped".into()))?
}

#[tauri::command]
pub async fn set_plugin_enabled(plugin: String, enabled: bool) -> Result<(), AppError> {
    global().set_enabled(&plugin, enabled)
}

#[tauri::command]
pub async fn set_plugin_grants(plugin: String, grant: Vec<String>) -> Result<(), AppError> {
    global().set_grants(&plugin, &capability_list(&grant)?)
}

#[tauri::command]
pub async fn remove_plugin(plugin: String) -> Result<(), AppError> {
    tauri::async_runtime::spawn_blocking(move || global().remove(&plugin))
        .await
        .map_err(|_| AppError::ProcessingFailure("the plugin worker stopped".into()))?
}

#[tauri::command]
pub async fn remove_plugin_version(plugin: String, content_hash: String) -> Result<(), AppError> {
    tauri::async_runtime::spawn_blocking(move || global().remove_version(&plugin, &content_hash))
        .await
        .map_err(|_| AppError::ProcessingFailure("the plugin worker stopped".into()))?
}

/// Runs the plugin's self-test again: every filter, twice, whole and tiled.
#[tauri::command]
pub async fn test_plugin(plugin: String) -> Result<TestReport, AppError> {
    tauri::async_runtime::spawn_blocking(move || global().test(&plugin))
        .await
        .map_err(|_| AppError::ProcessingFailure("the plugin worker stopped".into()))?
}

/// What a document (and the operation list that goes with it) needs from plugins,
/// and whether each can be had right now.
#[tauri::command]
pub async fn document_plugin_status(
    document: Option<LayerDocument>,
    operations: Vec<EditOperation>,
) -> Result<Vec<RequirementStatus>, AppError> {
    let mut needed = Vec::new();
    if let Some(document) = &document {
        needed.extend(requirements(document));
    }
    needed.extend(operation_requirements(&operations));
    tauri::async_runtime::spawn_blocking(move || Ok(statuses(&global(), needed)))
        .await
        .map_err(|_| AppError::ProcessingFailure("the plugin worker stopped".into()))?
}

/// The values a person last used for a filter or command.
#[tauri::command]
pub async fn plugin_remembered_values(
    plugin: String,
    key: String,
) -> Result<BTreeMap<String, f64>, AppError> {
    Ok(global().remembered(&plugin, &key))
}

/// Remembers the values a person last used for a filter or command, so the dialog
/// opens where they left it. Held by the host, never given to a module.
#[tauri::command]
pub async fn remember_plugin_values(
    plugin: String,
    key: String,
    values: BTreeMap<String, f64>,
) -> Result<(), AppError> {
    global().remember(&plugin, &key, values)
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct RunPluginCommand {
    pub plugin: String,
    pub command: String,
    #[serde(default)]
    pub values: BTreeMap<String, f64>,
    pub document: LayerDocument,
    #[serde(default)]
    pub selection: Option<MaskSnapshot>,
    #[serde(default)]
    pub expected_revision: Option<String>,
}

/// Replaces `$self` and `{"$param": name}` in a command's step with the plugin's id
/// and the values the person gave. This is the whole of "templating".
fn substitute(
    template: &Value,
    plugin: &str,
    manifest: &PluginManifest,
    command: &str,
    values: &BTreeMap<String, f64>,
) -> Value {
    match template {
        Value::String(text) if text == "$self" => json!(plugin),
        Value::Array(items) => Value::Array(
            items
                .iter()
                .map(|item| substitute(item, plugin, manifest, command, values))
                .collect(),
        ),
        Value::Object(map) => {
            if let Some(Value::String(name)) = map.get("$param") {
                let value = values.get(name).copied().unwrap_or(0.0);
                let is_bool = manifest
                    .command(command)
                    .and_then(|declared| declared.parameters.iter().find(|p| &p.id == name))
                    .is_some_and(|p| matches!(p.kind, ParamKind::Bool { .. }));
                return if is_bool {
                    json!(value != 0.0)
                } else {
                    json!(value)
                };
            }
            Value::Object(
                map.iter()
                    .map(|(key, item)| {
                        (
                            key.clone(),
                            substitute(item, plugin, manifest, command, values),
                        )
                    })
                    .collect(),
            )
        }
        other => other.clone(),
    }
}

/// What a plugin command comes to once its values are checked.
pub struct CommandPlan {
    /// The undo entry's name.
    pub label: String,
    pub steps: Vec<OperationCall>,
    /// Every parameter's value, defaults filled in.
    pub values: BTreeMap<String, f64>,
}

/// Builds the operations a plugin command stands for, checking the values given.
pub fn command_steps(
    manifest: &PluginManifest,
    command_id: &str,
    values: &BTreeMap<String, f64>,
) -> Result<CommandPlan, AppError> {
    let command = manifest
        .command(command_id)
        .ok_or_else(|| AppError::PluginUnavailable {
            plugin: manifest.id.clone(),
            reason: format!("it has no command called {command_id}"),
        })?;
    if let Some(unknown) = values
        .keys()
        .find(|key| !command.parameters.iter().any(|p| &p.id == *key))
    {
        return Err(AppError::InvalidOperation(format!(
            "{command_id} has no parameter called {unknown}"
        )));
    }
    let mut complete = BTreeMap::new();
    for declared in &command.parameters {
        let value = values
            .get(&declared.id)
            .copied()
            .unwrap_or_else(|| declared.default_value());
        if !declared.accepts(value) {
            return Err(AppError::InvalidOperation(format!(
                "{value} is not a value the parameter {} accepts",
                declared.id
            )));
        }
        complete.insert(declared.id.clone(), value);
    }
    let steps = command
        .steps
        .iter()
        .map(|step| OperationCall {
            op: step.op.clone(),
            params: substitute(&step.params, &manifest.id, manifest, command_id, &complete),
            when: None,
        })
        .collect();
    Ok(CommandPlan {
        label: command.title.clone(),
        steps,
        values: complete,
    })
}

/// Runs one plugin command as one transaction.
///
/// It is the same engine the Layers panel's merge and the workflow replay use, so
/// it is atomic, validated after every step, and one undo entry. What is particular
/// to a plugin is the *origin*: layer locks are respected, destructive operations
/// are not available, and it may apply only its own filters.
#[tauri::command]
pub async fn run_plugin_command(
    request: RunPluginCommand,
    state: State<'_, AppState>,
) -> Result<TransactionResult, AppError> {
    let registry = global();
    let loaded = match registry.active(&request.plugin) {
        Resolution::Available(loaded) => loaded,
        Resolution::Unavailable(why) => {
            return Err(AppError::PluginUnavailable {
                plugin: request.plugin.clone(),
                reason: why.describe(&request.plugin, ""),
            })
        }
    };
    if !registry.granted(&request.plugin, Capability::DocumentOperations) {
        return Err(AppError::PluginUnavailable {
            plugin: request.plugin.clone(),
            reason: format!(
                "it has not been allowed to {}",
                Capability::DocumentOperations.describe().to_lowercase()
            ),
        });
    }
    let CommandPlan {
        label,
        steps,
        values,
    } = command_steps(&loaded.manifest, &request.command, &request.values)?;
    let _permit = state.layer_gate.lock().await;
    let result = execute(
        &state,
        TransactionRequest {
            document: request.document,
            label,
            steps,
            expected_revision: request.expected_revision,
            selection: request.selection,
            origin: Origin::Plugin,
            plugin: Some(request.plugin.clone()),
        },
    )
    .await?;
    // Best effort: failing to remember a dialog's values must not fail the edit.
    let _ = registry.remember(
        &request.plugin,
        &format!("command:{}", request.command),
        values,
    );
    Ok(result)
}
