//! Resource management: what the machine has, what PhotoForge may spend, and
//! whether a piece of work fits.
//!
//! * [`memory`] measures the machine and this process.
//! * [`policy`] turns a measurement and a user setting into one budget, and
//!   derives every ceiling in the application from it.
//! * The estimates here price a job in bytes, with checked arithmetic, *before*
//!   anything is allocated.
//! * [`admission`] decides what to do with a source that may not fit.
//!
//! Nothing here reads a compile-time size constant any more. The old
//! `MAX_WORKING_IMAGE_BYTES` and its relatives are gone: asking "how big may an
//! image be" is now a call that returns this machine's answer.
use crate::error::AppError;
use serde::Serialize;
use std::sync::atomic::AtomicBool;
use std::sync::{Mutex, MutexGuard, TryLockError};

pub mod admission;
pub mod memory;
pub mod policy;
pub mod settings;

pub use policy::{
    budget, configure, limits, reset, restore, snapshot, Budget, BudgetBasis, BudgetMode,
    PolicySnapshot, ResourceLimits, BYTES_PER_WORKING_PIXEL, HARD_MAX_CANVAS_DIMENSION,
    HARD_MAX_CANVAS_PIXELS, HARD_MAX_SOURCE_DIMENSION,
};

static CPU_JOB: Mutex<()> = Mutex::new(());
/// Serialize full-frame workers, including batch, with cancellable admission.
/// Call only in blocking workers, before taking any session/store lock.
pub fn acquire_job(cancel: Option<&AtomicBool>) -> Result<MutexGuard<'static, ()>, AppError> {
    loop {
        crate::image_processing::high_precision::check_cancel(cancel)?;
        match CPU_JOB.try_lock() {
            Ok(guard) => return Ok(guard),
            Err(TryLockError::Poisoned(_)) => {
                return Err(AppError::ProcessingFailure(
                    "CPU job gate is unavailable".into(),
                ))
            }
            Err(TryLockError::WouldBlock) => {
                std::thread::sleep(std::time::Duration::from_millis(20))
            }
        }
    }
}

/// Re-reads the saved choice and the machine, and installs the result.
///
/// For document-open boundaries: the open replaces the previous document, so
/// nothing that was validated against the old limits is still alive to be
/// disturbed by new ones.
pub fn refresh(probe: &dyn memory::MemoryProbe) -> Budget {
    settings::install(probe)
}

/// The budget and limits the next document open will install, computed now and
/// not installed.
///
/// The decision shown to a user before they open something has to be made
/// against the numbers the open will then use, not against whatever was
/// installed when the application started.
pub fn candidate(probe: &dyn memory::MemoryProbe) -> (Budget, ResourceLimits) {
    let budget = policy::compute_budget(settings::load_mode(), probe.system());
    (budget, ResourceLimits::from_budget(budget.bytes))
}

/// What came of asking to change the budget.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ModeChange {
    /// The budget in force now.
    pub budget: Budget,
    /// Whether the new setting is already in force. A lower budget is saved but
    /// not applied until the next document opens.
    pub applied: bool,
}

/// Changes the budget setting, saving it to `path`.
///
/// Raising the budget takes effect at once, because limits that only grow cannot
/// invalidate anything already open. Lowering it does not: a document that is
/// open was admitted under the old limits, and `LayerDocument::validate`
/// consults the current ones on every command, so cutting them mid-session would
/// make the open document start failing validation. The new choice is saved and
/// takes effect when the next document opens.
pub fn change_mode_at(
    path: &std::path::Path,
    mode: BudgetMode,
    probe: &dyn memory::MemoryProbe,
) -> Result<ModeChange, AppError> {
    settings::save_mode_to(path, mode)?;
    let candidate = policy::compute_budget(mode, probe.system());
    let current = budget();
    if candidate.bytes >= current.bytes {
        Ok(ModeChange {
            budget: configure(mode, probe),
            applied: true,
        })
    } else {
        Ok(ModeChange {
            budget: current,
            applied: false,
        })
    }
}

pub fn change_mode(
    mode: BudgetMode,
    probe: &dyn memory::MemoryProbe,
) -> Result<ModeChange, AppError> {
    change_mode_at(&settings::settings_path(), mode, probe)
}

/// The largest single float working image, in bytes, under the current budget.
pub fn max_working_image_bytes() -> u64 {
    limits().working_image_bytes
}

/// The most pixels a canvas may hold under the current budget.
pub fn max_working_pixels() -> u64 {
    limits().working_pixels()
}

/// The most any one job may peak at.
pub fn max_job_bytes() -> u64 {
    limits().job_bytes
}

/// The pixel buffers the session store may retain.
pub fn max_store_bytes() -> u64 {
    limits().store_bytes
}

/// The largest single decoded project entry.
pub fn max_entry_bytes() -> u64 {
    limits().entry_bytes
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ResourceEstimate {
    pub source_bytes: u64,
    pub scratch_bytes: u64,
    pub mask_bytes: u64,
    pub output_bytes: u64,
    pub estimated_peak_bytes: u64,
}

/// Pixels in a `width` by `height` canvas, or why that canvas is not admissible.
///
/// Two separate refusals share the one error because callers only need to know
/// the canvas cannot be had, but they mean different things: an edge beyond the
/// structural ceiling, or a pixel count beyond the current budget.
pub fn checked_pixels(width: u32, height: u32) -> Result<u64, AppError> {
    let pixels = u64::from(width)
        .checked_mul(u64::from(height))
        .ok_or(AppError::OutOfMemoryRisk)?;
    let limit = max_working_pixels();
    if width == 0
        || height == 0
        || width > HARD_MAX_CANVAS_DIMENSION
        || height > HARD_MAX_CANVAS_DIMENSION
        || pixels > limit
    {
        return Err(AppError::ImageTooLarge { pixels, limit });
    }
    Ok(pixels)
}

impl ResourceEstimate {
    pub fn pipeline(
        width: u32,
        height: u32,
        operations: &[crate::domain::EditOperation],
        resident: u64,
    ) -> Result<Self, AppError> {
        let n = checked_pixels(width, height)?;
        let frames = operations
            .iter()
            .map(crate::image_processing::high_precision::scratch_frames)
            .max()
            .unwrap_or(0);
        Self::new(
            resident,
            n.checked_mul(16)
                .and_then(|n| n.checked_mul(frames))
                .ok_or(AppError::OutOfMemoryRisk)?,
            0,
            n * 8 + 40_960_000,
        )
    }
    pub fn new(
        source_bytes: u64,
        scratch_bytes: u64,
        mask_bytes: u64,
        output_bytes: u64,
    ) -> Result<Self, AppError> {
        let total = source_bytes
            .checked_add(scratch_bytes)
            .and_then(|n| n.checked_add(mask_bytes))
            .and_then(|n| n.checked_add(output_bytes))
            .ok_or(AppError::OutOfMemoryRisk)?;
        let limit = max_job_bytes();
        if total > limit {
            return Err(AppError::ResourceBudget {
                required: total,
                limit,
            });
        }
        Ok(Self {
            source_bytes,
            scratch_bytes,
            mask_bytes,
            output_bytes,
            estimated_peak_bytes: total,
        })
    }
    /// Sensor, demosaic, camera transform, development copy and display buffers.
    pub fn decode(width: u32, height: u32, compressed_bytes: u64) -> Result<Self, AppError> {
        let n = checked_pixels(width, height)?;
        Self::new(
            compressed_bytes,
            n.checked_mul(52).ok_or(AppError::OutOfMemoryRisk)?,
            0,
            n * 4 + 40_960_000,
        )
    }
    pub fn render(
        document: &crate::layers::LayerDocument,
        scale: f64,
        resident: u64,
        promotion_bytes: u64,
    ) -> Result<Self, AppError> {
        let w = ((f64::from(document.canvas_width) * scale).round() as u32).max(1);
        let h = ((f64::from(document.canvas_height) * scale).round() as u32).max(1);
        let n = checked_pixels(w, h)?;
        let mut max_depth = 0_u64;
        let mut adjustment_frames = 0_u64;
        let mut masks = 0_u64;
        let mut stack: Vec<_> = document.layers.iter().map(|l| (l, 0_u64)).collect();
        while let Some((layer, depth)) = stack.pop() {
            if !layer.visible || layer.opacity <= 0.0 {
                continue;
            }
            if let Some(mask) = &layer.mask {
                masks = masks
                    .checked_add(u64::from(mask.snapshot.width) * u64::from(mask.snapshot.height))
                    .ok_or(AppError::OutOfMemoryRisk)?;
            }
            match &layer.content {
                crate::layers::LayerContent::Group { children, .. } => {
                    max_depth = max_depth.max(depth + 1);
                    stack.extend(children.iter().map(|l| (l, depth + 1)));
                }
                crate::layers::LayerContent::Adjustment { operation } => {
                    let frames = crate::image_processing::high_precision::scratch_frames(operation);
                    adjustment_frames = adjustment_frames.max(frames);
                }
                _ => {}
            }
        }
        // Canvas + one promoted encoded source + the deepest live groups and
        // operation scratch. Reserve PNG16-sized output even for preview.
        let scratch = n
            .checked_mul(16)
            .and_then(|b| b.checked_mul(1 + max_depth + adjustment_frames))
            .and_then(|b| b.checked_add(promotion_bytes))
            .ok_or(AppError::OutOfMemoryRisk)?;
        Self::new(resident, scratch, masks, n * 8 + 40_960_000)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn overflow_and_decompression_bombs_fail_before_allocation() {
        assert!(checked_pixels(u32::MAX, u32::MAX).is_err());
        assert!(ResourceEstimate::new(u64::MAX, 1, 0, 0).is_err());
        assert!(checked_pixels(20000, 20000).is_err());
    }
    #[test]
    fn legitimate_45_and_61_mp_dimensions_have_explicit_byte_costs() {
        for (w, h) in [(8256, 5504), (9504, 6336)] {
            let estimate = ResourceEstimate::decode(w, h, 100_000_000).unwrap();
            assert!(estimate.estimated_peak_bytes < max_job_bytes());
            assert!(estimate.scratch_bytes > 2_000_000_000);
        }
    }
    #[test]
    fn excessive_live_buffers_are_rejected_even_when_dimensions_are_legal() {
        assert!(matches!(
            ResourceEstimate::new(3_000_000_000, 2_000_000_000, 0, 0),
            Err(AppError::ResourceBudget { .. })
        ));
    }
}
