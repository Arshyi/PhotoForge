//! What a document needs from plugins, and what happens when it cannot have it.
//!
//! A document that uses a plugin records the plugin's identity on the layer that
//! uses it. Opening that document on a machine without the plugin must neither fail
//! nor quietly produce a different picture, so there are exactly three behaviours:
//!
//! * **Saving and reopening keep the reference.** The layer, its parameters and its
//!   identity are part of the document and are written back unchanged. A plugin that
//!   is absent is a fact about this machine, not an edit to the document.
//! * **The preview shows that something is missing.** The layers that need an
//!   unavailable plugin are left out of *that render only*, and the render says so,
//!   by name, for the interface to show. It is a visible gap, not a substitute.
//! * **Anything that would make a file out of it refuses.** Export, merge, flatten
//!   and rasterise stop with the reason, because the result would be a picture the
//!   document is not.
//!
//! Plugins used by the *document-level* operation list (presets, workflows, the
//! global pipeline) are reported the same way, and fail their render with the reason.
use super::store::{Availability, PluginRegistry, Resolution};
use crate::domain::EditOperation;
use crate::error::AppError;
use crate::layers::{Layer, LayerContent, LayerDocument};
use serde::Serialize;
use std::collections::BTreeMap;

/// A plugin version a document uses, and where.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Requirement {
    pub plugin: String,
    pub version: String,
    pub sha256: String,
    pub filters: Vec<String>,
    /// The layers that use it. Empty for a requirement of the operation list.
    pub layer_ids: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RequirementStatus {
    #[serde(flatten)]
    pub requirement: Requirement,
    pub availability: Availability,
    /// A sentence a person can read.
    pub message: String,
}

/// A layer left out of a render because its plugin is not available.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MissingPlugin {
    pub layer_id: String,
    pub layer_name: String,
    pub plugin: String,
    pub version: String,
    pub message: String,
}

/// The plugin reference inside an operation, through a mask.
fn reference(operation: &EditOperation) -> Option<(&str, &str, &str, &str)> {
    match operation {
        EditOperation::PluginFilter {
            plugin,
            version,
            sha256,
            filter,
            ..
        } => Some((plugin, version, sha256, filter)),
        EditOperation::Masked { operation, .. } => reference(operation),
        _ => None,
    }
}

type Key = (String, String, String);

fn gather(layers: &[Layer], found: &mut BTreeMap<Key, Requirement>) {
    for layer in layers {
        match &layer.content {
            LayerContent::Adjustment { operation } => {
                if let Some((plugin, version, sha256, filter)) = reference(operation) {
                    let entry = found
                        .entry((plugin.to_string(), version.to_string(), sha256.to_string()))
                        .or_insert_with(|| Requirement {
                            plugin: plugin.to_string(),
                            version: version.to_string(),
                            sha256: sha256.to_string(),
                            filters: Vec::new(),
                            layer_ids: Vec::new(),
                        });
                    if !entry.filters.iter().any(|name| name == filter) {
                        entry.filters.push(filter.to_string());
                    }
                    entry.layer_ids.push(layer.id.clone());
                }
            }
            LayerContent::Group { children, .. } => gather(children, found),
            _ => {}
        }
    }
}

/// Every plugin version the document's layers use, including inside smart objects.
pub fn requirements(document: &LayerDocument) -> Vec<Requirement> {
    let mut found = BTreeMap::new();
    gather(&document.layers, &mut found);
    for source in document.smart_sources.values() {
        gather(&source.layers, &mut found);
    }
    found.into_values().collect()
}

/// The plugin versions an operation list uses (a preset, a workflow, the pipeline).
pub fn operation_requirements(operations: &[EditOperation]) -> Vec<Requirement> {
    let mut found: BTreeMap<Key, Requirement> = BTreeMap::new();
    for operation in operations {
        if let Some((plugin, version, sha256, filter)) = reference(operation) {
            let entry = found
                .entry((plugin.to_string(), version.to_string(), sha256.to_string()))
                .or_insert_with(|| Requirement {
                    plugin: plugin.to_string(),
                    version: version.to_string(),
                    sha256: sha256.to_string(),
                    filters: Vec::new(),
                    layer_ids: Vec::new(),
                });
            if !entry.filters.iter().any(|name| name == filter) {
                entry.filters.push(filter.to_string());
            }
        }
    }
    found.into_values().collect()
}

fn availability(registry: &PluginRegistry, requirement: &Requirement) -> Availability {
    match registry.resolve(&requirement.plugin, &requirement.sha256) {
        Resolution::Available(loaded) => {
            // Present is not enough: every filter the document uses must be in it.
            match requirement
                .filters
                .iter()
                .find(|filter| loaded.manifest.filter(filter).is_none())
            {
                Some(filter) => Availability::Damaged {
                    reason: format!("it has no filter called {filter}"),
                },
                None => Availability::Available,
            }
        }
        Resolution::Unavailable(why) => why,
    }
}

/// Whether each plugin a list of requirements names can be used right now.
pub fn statuses(
    registry: &PluginRegistry,
    requirements: Vec<Requirement>,
) -> Vec<RequirementStatus> {
    requirements
        .into_iter()
        .map(|requirement| {
            let availability = availability(registry, &requirement);
            let message = availability.describe(&requirement.plugin, &requirement.version);
            RequirementStatus {
                requirement,
                availability,
                message,
            }
        })
        .collect()
}

/// Leaves out of `document` the adjustment layers whose plugin is unavailable, for
/// the render that is about to be made, and says which. The document handed to the
/// caller's own state is not this one.
pub fn hide_unavailable(
    registry: &PluginRegistry,
    document: &mut LayerDocument,
) -> Vec<MissingPlugin> {
    let unavailable: BTreeMap<Key, Availability> = statuses(registry, requirements(document))
        .into_iter()
        .filter(|status| !status.availability.is_available())
        .map(|status| {
            (
                (
                    status.requirement.plugin,
                    status.requirement.version,
                    status.requirement.sha256,
                ),
                status.availability,
            )
        })
        .collect();
    let mut missing = Vec::new();
    if unavailable.is_empty() {
        return missing;
    }
    fn visit(
        layers: &mut [Layer],
        unavailable: &BTreeMap<Key, Availability>,
        missing: &mut Vec<MissingPlugin>,
    ) {
        for layer in layers {
            match &mut layer.content {
                LayerContent::Adjustment { operation } => {
                    if let Some((plugin, version, sha256, _)) = reference(operation) {
                        let key = (plugin.to_string(), version.to_string(), sha256.to_string());
                        if let Some(why) = unavailable.get(&key) {
                            missing.push(MissingPlugin {
                                layer_id: layer.id.clone(),
                                layer_name: layer.name.clone(),
                                plugin: key.0.clone(),
                                version: key.1.clone(),
                                message: why.describe(&key.0, &key.1),
                            });
                            layer.visible = false;
                        }
                    }
                }
                LayerContent::Group { children, .. } => visit(children, unavailable, missing),
                _ => {}
            }
        }
    }
    visit(&mut document.layers, &unavailable, &mut missing);
    for source in document.smart_sources.values_mut() {
        visit(&mut source.layers, &unavailable, &mut missing);
    }
    missing
}

/// Refuses if a layer that would be drawn needs a plugin that is not available.
///
/// For everything that turns a render into something that lasts.
pub fn ensure_available(
    registry: &PluginRegistry,
    document: &LayerDocument,
) -> Result<(), AppError> {
    let mut view = document.clone();
    let missing = hide_unavailable(registry, &mut view);
    // A layer that is hidden in the document itself — or inside a hidden group — is
    // not drawn either way, so only one that *would have been drawn* counts. A layer
    // whose place cannot be found (inside a smart object) is counted: refusing is the
    // safe error.
    for entry in missing {
        let drawn = match document.path_to(&entry.layer_id) {
            Some(path) => (1..=path.len()).all(|end| {
                document
                    .layer_at(&path[..end])
                    .is_none_or(|layer| layer.visible)
            }),
            None => true,
        };
        if drawn {
            return Err(AppError::PluginUnavailable {
                plugin: entry.plugin,
                reason: format!(
                    "{} Layer \"{}\" needs it, so a picture made now would not be this document's.",
                    entry.message, entry.layer_name
                ),
            });
        }
    }
    Ok(())
}

/// As [`ensure_available`], for an operation list.
pub fn ensure_operations_available(
    registry: &PluginRegistry,
    operations: &[EditOperation],
) -> Result<(), AppError> {
    for status in statuses(registry, operation_requirements(operations)) {
        if !status.availability.is_available() {
            return Err(AppError::PluginUnavailable {
                plugin: status.requirement.plugin,
                reason: status.message,
            });
        }
    }
    Ok(())
}

/// The operations of a list whose plugin is available, in order. For the render that
/// is made so a document can open; the list itself is not changed.
pub fn without_unavailable(
    registry: &PluginRegistry,
    operations: &[EditOperation],
) -> Vec<EditOperation> {
    let blocked: std::collections::BTreeSet<Key> =
        statuses(registry, operation_requirements(operations))
            .into_iter()
            .filter(|status| !status.availability.is_available())
            .map(|status| {
                (
                    status.requirement.plugin,
                    status.requirement.version,
                    status.requirement.sha256,
                )
            })
            .collect();
    operations
        .iter()
        .filter(|operation| {
            reference(operation).is_none_or(|(plugin, version, sha256, _)| {
                !blocked.contains(&(plugin.to_string(), version.to_string(), sha256.to_string()))
            })
        })
        .cloned()
        .collect()
}
