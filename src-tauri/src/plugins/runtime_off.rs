//! The stand-in for the runtime in a build without the `plugins` feature.
//!
//! It has the same shape as `runtime.rs` so the rest of the plugin code compiles
//! unchanged, and it can do exactly one thing: say that it cannot. Nothing in it can
//! compile or run a module, and the build carries no WebAssembly engine.
use super::job::{TileJob, TileResult};
use super::limits::CallLimits;
use super::PluginError;
use std::sync::atomic::AtomicBool;
use std::sync::Arc;

#[derive(Clone)]
pub struct Compiled;

pub struct Runtime;

impl Runtime {
    pub const fn available() -> bool {
        false
    }

    pub fn new() -> Result<Self, PluginError> {
        Err(PluginError::RuntimeUnavailable)
    }

    pub fn compile(&self, _wasm: &[u8]) -> Result<Compiled, PluginError> {
        Err(PluginError::RuntimeUnavailable)
    }

    pub fn call(
        &self,
        _compiled: &Compiled,
        _job: &TileJob<'_>,
        _limits: CallLimits,
        _cancel: Option<&Arc<AtomicBool>>,
    ) -> Result<TileResult, PluginError> {
        Err(PluginError::RuntimeUnavailable)
    }
}
