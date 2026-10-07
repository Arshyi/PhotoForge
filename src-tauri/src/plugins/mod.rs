//! Plugins: extending PhotoForge without giving anything the machine.
//!
//! A plugin is a ZIP file (`.photoforge-plugin`) holding a manifest and, if it has
//! filters, one WebAssembly module. What it can do is **declared**, **shown** to the
//! person installing it, **granted** by them, and **checked on every use** — and
//! the default is nothing. See `docs/plugins.md` for the whole picture,
//! `docs/plugin-security.md` for the threat model and its honest limits, and
//! `docs/plugin-api.md` for the interface a module is written against.
//!
//! There is no second editing engine. A plugin's commands are lists of registered
//! operations run by the same transaction engine as everything else; its filters
//! become a node in the same render pipeline as the built-in adjustments; and a
//! document that needs a plugin says which, and is never quietly changed when it is
//! missing.
//!
//! * [`manifest`] — what a plugin may declare, and the capabilities.
//! * [`package`] — reading the ZIP as hostile input.
//! * [`limits`] — the sandbox's numbers.
//! * [`runtime`] — the WebAssembly engine, behind the `plugins` feature.
//! * [`filter`] — running a filter over an image, tile by tile.
//! * [`store`] — installing, finding and removing plugins.
//! * [`apply`] — a filter as a node in the render pipeline.
//! * [`document`] — what a document needs from plugins, and what happens without them.
pub mod apply;
pub mod document;
pub mod filter;
pub mod job;
pub mod limits;
pub mod manifest;
pub mod package;
#[cfg(feature = "plugins")]
mod runtime;
#[cfg(not(feature = "plugins"))]
#[path = "runtime_off.rs"]
mod runtime;
pub mod store;
#[doc(hidden)]
pub mod testing;

pub use runtime::{Compiled, Runtime};

use crate::error::AppError;

/// Why a plugin could not do what it was asked. Every variant is something a person
/// can be told in a sentence.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum PluginError {
    #[error("this build of PhotoForge does not include the plugin runtime")]
    RuntimeUnavailable,
    #[error("the plugin runtime failed: {0}")]
    Runtime(String),
    #[error("the plugin's module is not usable: {0}")]
    Module(String),
    #[error("it cannot be run: {0}")]
    Refused(String),
    #[error("the plugin stopped with an error: {0}")]
    Trap(String),
    #[error("the plugin used up the work it is allowed and was stopped")]
    OutOfFuel,
    #[error("the plugin ran out of time and was stopped")]
    TimedOut,
    #[error("cancelled")]
    Cancelled,
    #[error("the plugin reported error {code}")]
    Reported { code: i32 },
    #[error("the plugin returned something unusable: {0}")]
    BadOutput(String),
}

impl PluginError {
    /// A stable code for the interface and for tests.
    pub fn code(&self) -> &'static str {
        match self {
            Self::RuntimeUnavailable => "runtime_unavailable",
            Self::Runtime(_) => "runtime",
            Self::Module(_) => "module",
            Self::Refused(_) => "refused",
            Self::Trap(_) => "trap",
            Self::OutOfFuel => "out_of_fuel",
            Self::TimedOut => "timed_out",
            Self::Cancelled => "cancelled",
            Self::Reported { .. } => "reported",
            Self::BadOutput(_) => "bad_output",
        }
    }

    /// This error attributed to a plugin, for a person to read.
    pub fn for_plugin(self, plugin: &str) -> AppError {
        match self {
            Self::Cancelled => AppError::RenderCancelled,
            other => AppError::Plugin {
                plugin: plugin.to_string(),
                code: other.code().to_string(),
                message: other.to_string(),
            },
        }
    }
}
