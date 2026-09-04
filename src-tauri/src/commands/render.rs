//! Reporting and control for the render engine.
//!
//! Everything here is observation or an explicit user action. The interface
//! can say what the renderer is doing, and the user can take memory back or
//! turn a device off, but nothing here changes what a render produces.
use serde::Serialize;
use tauri::State;

use crate::application::AppState;
use crate::error::AppError;
use crate::layers::{CacheStats, DEFAULT_CACHE_BYTES};

/// Largest render cache a user may ask for.
///
/// The cache holds decoded float tiles, so an unbounded setting would be an
/// unbounded allocation with a friendly name.
pub const MAX_CACHE_BYTES: u64 = 4 * 1024 * 1024 * 1024;

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RenderDiagnostics {
    /// Tile edge in pixels for full-resolution renders.
    pub tile_size: u32,
    /// Upper bound on worker threads for one render.
    pub max_threads: u32,
    pub cache: CacheStats,
    /// Whether this build has the GPU feature compiled in at all.
    pub gpu_compiled: bool,
    /// Whether a usable device was found and is still healthy.
    pub gpu_available: bool,
    /// Present only when a device was found.
    pub gpu: Option<serde_json::Value>,
    /// Present only when a device was found: what it has actually done.
    pub gpu_stats: Option<serde_json::Value>,
    /// Why the GPU is not in use, when it is not. Empty when it is.
    pub gpu_note: String,
}

/// Reports what the render engine is configured to do and what it has done.
#[tauri::command]
pub async fn render_diagnostics(state: State<'_, AppState>) -> Result<RenderDiagnostics, AppError> {
    #[cfg(feature = "gpu")]
    let (gpu_compiled, gpu_available, gpu, gpu_stats) = {
        let available = crate::gpu::is_available();
        let info = crate::gpu::info();
        (
            true,
            available,
            info.as_ref().and_then(|i| serde_json::to_value(i).ok()),
            serde_json::to_value(crate::gpu::stats()).ok(),
        )
    };
    #[cfg(not(feature = "gpu"))]
    let (gpu_compiled, gpu_available, gpu, gpu_stats) = (false, false, None, None);

    let gpu_note = if !gpu_compiled {
        // Said plainly, because "GPU: unavailable" invites the user to go
        // looking for a driver problem that does not exist.
        "This build was compiled without GPU support.".to_string()
    } else if !gpu_available {
        "No usable GPU was found, or the device failed and the CPU is being used.".to_string()
    } else {
        String::new()
    };

    Ok(RenderDiagnostics {
        tile_size: crate::layers::DEFAULT_TILE_SIZE,
        max_threads: crate::layers::MAX_RENDER_THREADS as u32,
        cache: state.render_cache.stats(),
        gpu_compiled,
        gpu_available,
        gpu,
        gpu_stats,
        gpu_note,
    })
}

/// Drops every cached tile and reports the result.
///
/// Nothing is lost: a tile is a render that can be produced again. This exists
/// so a user who wants the memory back can have it immediately rather than
/// waiting for eviction.
#[tauri::command]
pub async fn clear_render_cache(state: State<'_, AppState>) -> Result<CacheStats, AppError> {
    state.render_cache.clear();
    Ok(state.render_cache.stats())
}

/// Changes the render cache budget, in bytes.
///
/// Zero is a legitimate setting and means no caching at all. Shrinking evicts
/// immediately rather than at the next render.
#[tauri::command]
pub async fn set_render_cache_budget(
    bytes: u64,
    state: State<'_, AppState>,
) -> Result<CacheStats, AppError> {
    if bytes > MAX_CACHE_BYTES {
        return Err(AppError::InvalidLayerDocument(format!(
            "the render cache is limited to {} MB",
            MAX_CACHE_BYTES / (1024 * 1024)
        )));
    }
    state.render_cache.set_capacity(bytes);
    Ok(state.render_cache.stats())
}

/// The budget a fresh session starts with, so the interface can show a
/// meaningful default next to the slider.
#[tauri::command]
pub async fn default_render_cache_budget() -> Result<u64, AppError> {
    Ok(DEFAULT_CACHE_BYTES)
}

/// Returns the runtime acceleration policy. The policy never changes the CPU
/// tiled compositor's semantics; it only controls whether supported operations
/// may use an available device before falling back to CPU.
#[cfg(feature = "gpu")]
#[tauri::command]
pub async fn get_render_backend_mode() -> Result<crate::gpu::GpuPolicy, AppError> {
    Ok(crate::gpu::policy())
}

/// Changes the runtime acceleration policy without requiring an application
/// restart. Selecting GPU is a preference, not a promise: device and operation
/// limits still fail closed to the CPU implementation.
#[cfg(feature = "gpu")]
#[tauri::command]
pub async fn set_render_backend_mode(
    mode: crate::gpu::GpuMode,
) -> Result<crate::gpu::GpuPolicy, AppError> {
    Ok(crate::gpu::set_mode(mode))
}

#[cfg(not(feature = "gpu"))]
mod cpu_only_policy {
    use serde::{Deserialize, Serialize};
    use std::sync::atomic::{AtomicU8, Ordering};

    static MODE: AtomicU8 = AtomicU8::new(0);

    #[derive(Debug, Clone, Copy, Deserialize, Serialize)]
    #[serde(rename_all = "lowercase")]
    pub enum Mode {
        Auto,
        Cpu,
        Gpu,
    }

    impl Mode {
        fn number(self) -> u8 {
            match self {
                Self::Auto => 0,
                Self::Cpu => 1,
                Self::Gpu => 2,
            }
        }

        fn current() -> Self {
            match MODE.load(Ordering::Acquire) {
                1 => Self::Cpu,
                2 => Self::Gpu,
                _ => Self::Auto,
            }
        }
    }

    #[derive(Debug, Clone, Serialize)]
    #[serde(rename_all = "camelCase")]
    pub struct Policy {
        pub mode: Mode,
        pub active: bool,
        pub effective_backend: &'static str,
        pub adapter_status: &'static str,
        pub hard_disabled: bool,
        pub fallback_reason: Option<&'static str>,
    }

    pub fn get() -> Policy {
        Policy {
            mode: Mode::current(),
            active: false,
            effective_backend: "cpu",
            adapter_status: "not_probed",
            hard_disabled: false,
            fallback_reason: Some("This build was compiled without GPU support."),
        }
    }

    pub fn set(mode: Mode) -> Policy {
        MODE.store(mode.number(), Ordering::Release);
        get()
    }
}

#[cfg(not(feature = "gpu"))]
#[tauri::command]
pub async fn get_render_backend_mode() -> Result<cpu_only_policy::Policy, AppError> {
    Ok(cpu_only_policy::get())
}

#[cfg(not(feature = "gpu"))]
#[tauri::command]
pub async fn set_render_backend_mode(
    mode: cpu_only_policy::Mode,
) -> Result<cpu_only_policy::Policy, AppError> {
    Ok(cpu_only_policy::set(mode))
}
