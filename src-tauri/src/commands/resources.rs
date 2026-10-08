//! Resource commands: what the machine has, what PhotoForge may spend, and the
//! user's one setting that changes it.
//!
//! Nothing here is image content. The numbers are about the machine and this
//! process, and they stay on it.
use serde::Serialize;
use tauri::State;

use crate::application::AppState;
use crate::error::AppError;
use crate::layers::CacheStats;
use crate::resources::{
    self,
    memory::{free_disk_bytes, MemoryProbe, OsProbe, ProcessMemory, SystemMemory},
    Budget, BudgetMode, ModeChange, ResourceLimits,
};

/// Why GPU memory is not reported.
///
/// wgpu exposes limits such as the largest buffer, not how much video memory is
/// free. Reading that reliably means going through DXGI, which this application
/// does not otherwise use. Reporting a limit as if it were free memory would be
/// worse than reporting nothing, so nothing is reported and the reason is given.
const GPU_MEMORY_REASON: &str = "Video memory is not reliably measurable through wgpu, which \
     reports limits rather than free memory. GPU work is bounded by tile size instead.";

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct GpuMemory {
    pub measurable: bool,
    pub reason: &'static str,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ResourceStatus {
    /// `None` if the machine could not be measured.
    pub system: Option<SystemMemory>,
    pub process: Option<ProcessMemory>,
    pub budget: Budget,
    /// The least a manual budget may be, so the interface can offer exactly the
    /// range the policy will accept rather than a range it will quietly move.
    pub min_budget_bytes: u64,
    pub limits: ResourceLimits,
    /// Most pixels a canvas may hold under the budget in force.
    pub max_working_pixels: u64,
    /// Pixel buffers the session store currently retains.
    pub resident_pixel_bytes: u64,
    pub render_cache: CacheStats,
    /// Free space on the volume the disk cache would use. `None` is "unknown",
    /// not "plenty".
    pub disk_free_bytes: Option<u64>,
    pub gpu_memory: GpuMemory,
    /// Whether a lower budget has been saved but not yet applied.
    pub reduction_pending: bool,
}

/// Whether a lower budget has been *chosen* and is waiting for the next document.
///
/// Only a saved setting that differs from the one in force counts. In Automatic mode
/// the budget follows free memory, which changes all the time; a budget that is lower
/// than the one fixed at start-up only because other programs have since taken memory
/// is the policy working as designed, and reporting it as "a lower budget is saved"
/// would be false, and constant. (Found by running the packaged application.)
fn reduction_pending(saved: BudgetMode, current: &Budget, wanted_bytes: u64) -> bool {
    saved != current.mode && wanted_bytes < current.bytes
}

fn status(state: &AppState) -> Result<ResourceStatus, AppError> {
    let resident = state
        .layers
        .lock()
        .map_err(|_| AppError::ProcessingFailure("layer store is unavailable".into()))?
        .total_bytes();
    let saved = resources::settings::load_mode();
    let wanted = resources::policy::compute_budget(saved, OsProbe.system());
    let current = resources::budget();
    Ok(ResourceStatus {
        system: OsProbe.system(),
        process: OsProbe.process(),
        budget: current,
        min_budget_bytes: resources::policy::MIN_BUDGET_BYTES,
        limits: resources::limits(),
        max_working_pixels: resources::max_working_pixels(),
        resident_pixel_bytes: resident,
        render_cache: state.render_cache.stats(),
        disk_free_bytes: free_disk_bytes(&resources::settings::settings_directory()),
        gpu_memory: GpuMemory {
            measurable: false,
            reason: GPU_MEMORY_REASON,
        },
        reduction_pending: reduction_pending(saved, &current, wanted.bytes),
    })
}

/// Reports the machine, the budget in force and what is currently held.
#[tauri::command]
pub async fn resource_status(state: State<'_, AppState>) -> Result<ResourceStatus, AppError> {
    status(&state)
}

/// Changes the memory budget.
///
/// A higher budget applies at once. A lower one is saved and applies when the
/// next document opens, so that the document being edited is not suddenly judged
/// against limits it was not admitted under. The returned status says which
/// happened.
#[tauri::command]
pub async fn set_memory_budget(
    mode: BudgetMode,
    state: State<'_, AppState>,
) -> Result<(ModeChange, ResourceStatus), AppError> {
    let change = resources::change_mode(mode, &OsProbe)?;
    Ok((change, status(&state)?))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::resources::policy::{Budget, BudgetBasis};

    fn in_force(mode: BudgetMode, bytes: u64) -> Budget {
        Budget {
            mode,
            basis: BudgetBasis::Measured,
            bytes,
            reserve_bytes: 0,
            ceiling_bytes: bytes * 2,
            clamped: false,
        }
    }

    #[test]
    fn automatic_following_free_memory_down_is_not_a_saved_reduction() {
        // Nothing was chosen; other programs simply took memory since start-up.
        let current = in_force(BudgetMode::Automatic, 80 << 30);
        assert!(!reduction_pending(
            BudgetMode::Automatic,
            &current,
            70 << 30
        ));
    }

    #[test]
    fn a_lower_manual_choice_is_pending_until_the_next_document() {
        let current = in_force(BudgetMode::Automatic, 80 << 30);
        let manual = BudgetMode::Manual { bytes: 4 << 30 };
        assert!(reduction_pending(manual, &current, 4 << 30));
        // A manual figure replaced by a lower one.
        let manual_now = in_force(BudgetMode::Manual { bytes: 8 << 30 }, 8 << 30);
        assert!(reduction_pending(manual, &manual_now, 4 << 30));
    }

    #[test]
    fn a_higher_or_unchanged_choice_is_not_pending() {
        let current = in_force(BudgetMode::Manual { bytes: 4 << 30 }, 4 << 30);
        // Raising applies at once, so there is nothing waiting.
        assert!(!reduction_pending(
            BudgetMode::Manual { bytes: 8 << 30 },
            &current,
            8 << 30
        ));
        // Saving what is already in force.
        assert!(!reduction_pending(current.mode, &current, 4 << 30));
    }
}
