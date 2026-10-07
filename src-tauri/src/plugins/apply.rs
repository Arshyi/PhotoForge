//! A plugin's filter as a node in the render pipeline.
//!
//! `EditOperation::PluginFilter` is how a plugin takes part in everything the
//! built-in adjustments take part in — adjustment layers, masks, batch, workflows,
//! the tiled renderer and its cache — without any of them knowing there is a
//! plugin. The operation carries the *identity* of the exact version it was made
//! with (the content hash), which is what keeps a document's picture the same
//! picture, and a declaration of how far the filter reaches, which is what the
//! tiler needs to know without asking a registry.
//!
//! Evaluating one resolves that exact version. If it cannot be resolved — not
//! installed, turned off, a different version, the runtime absent — evaluation
//! **fails with the reason**. It does not fall back to another version, and it does
//! not pass the image through as if the filter were not there; the layers above this
//! one report the missing plugin instead of drawing a picture that is not the
//! document's.
use super::filter;
use super::limits;
use super::manifest::{
    valid_local_id, valid_plugin_id, valid_sha256, valid_version, Locality, MAX_LOCAL_RADIUS,
    MAX_PARAMETERS,
};
use super::store::{global, Resolution};
use crate::color::FloatImage;
use crate::domain::EditOperation;
use crate::error::AppError;
use std::sync::atomic::AtomicBool;

/// Checks the *form* of a plugin filter reference, with no registry in sight.
pub fn validate_reference(
    plugin: &str,
    version: &str,
    sha256: &str,
    filter: &str,
    locality: &Locality,
    parameters: &[f64],
) -> Result<(), AppError> {
    let bad = |reason: &str| AppError::InvalidOperation(format!("plugin filter: {reason}"));
    if !valid_plugin_id(plugin) {
        return Err(bad("the plugin id is not valid"));
    }
    if !valid_version(version) {
        return Err(bad("the plugin version is not valid"));
    }
    if !valid_sha256(sha256) {
        return Err(bad("the plugin identity is not a SHA-256"));
    }
    if !valid_local_id(filter) {
        return Err(bad("the filter id is not valid"));
    }
    if let Locality::Local { radius } = locality {
        if *radius == 0 || *radius > MAX_LOCAL_RADIUS {
            return Err(bad("the declared reach is out of range"));
        }
    }
    if parameters.len() > MAX_PARAMETERS {
        return Err(bad("too many parameters"));
    }
    if parameters.iter().any(|value| !value.is_finite()) {
        return Err(bad("a parameter is not a finite number"));
    }
    Ok(())
}

/// Evaluates a `PluginFilter` operation on `image`.
pub fn apply_operation(
    image: &FloatImage,
    operation: &EditOperation,
    cancel: Option<&AtomicBool>,
) -> Result<FloatImage, AppError> {
    let EditOperation::PluginFilter {
        plugin,
        version,
        sha256,
        filter: filter_id,
        locality,
        parameters,
    } = operation
    else {
        return Err(AppError::InvalidOperation("not a plugin filter".into()));
    };
    let registry = global();
    let loaded = match registry.resolve(plugin, sha256) {
        Resolution::Available(loaded) => loaded,
        Resolution::Unavailable(why) => {
            return Err(AppError::PluginUnavailable {
                plugin: plugin.clone(),
                reason: why.describe(plugin, version),
            })
        }
    };
    let (index, decl) =
        loaded
            .manifest
            .filter(filter_id)
            .ok_or_else(|| AppError::PluginUnavailable {
                plugin: plugin.clone(),
                reason: format!("it has no filter called {filter_id}"),
            })?;
    // The content hash covers the manifest, so this cannot differ for an honest
    // document. It is checked because a document is untrusted input too.
    if decl.locality != *locality {
        return Err(AppError::PluginUnavailable {
            plugin: plugin.clone(),
            reason: "the document's idea of how far this filter reaches does not match the plugin"
                .into(),
        });
    }
    let compiled = loaded
        .compiled
        .as_ref()
        .ok_or_else(|| AppError::PluginUnavailable {
            plugin: plugin.clone(),
            reason: "it has no module to run".into(),
        })?;
    let runtime = registry
        .runtime()
        .map_err(|error| error.for_plugin(plugin))?;
    let allowance = limits::memory_allowance(
        loaded
            .manifest
            .limits
            .as_ref()
            .map(|limit| limit.memory_mib),
        crate::resources::budget().bytes,
    );
    filter::apply(
        runtime,
        compiled,
        decl,
        index,
        parameters,
        image,
        allowance,
        filter::DEFAULT_TILE,
        cancel,
    )
    .map(|run| run.image)
    .map_err(|error| error.for_plugin(plugin))
}
