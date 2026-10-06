//! The memory budget, and the limits derived from it.
//!
//! # Two kinds of limit
//!
//! PhotoForge used to have one: a compile-time constant of 1 GiB per working
//! image, which set a 67.1 megapixel ceiling on every machine alike. That
//! conflated two different questions.
//!
//! * **Structural limits** say whether a file or a document is *well formed
//!   enough to do arithmetic on*. They depend on nothing but the code: a pixel
//!   index that must fit a `u32`, a coordinate that must stay exact in an `f32`.
//!   No budget, however large, lifts them. They live here as `HARD_*` constants.
//! * **The budget** says how much memory PhotoForge may *spend*. It depends on
//!   the machine and on the user, and it is the only thing that varies.
//!
//! # The limits are stable for the life of a document
//!
//! `LayerDocument::validate` runs on every command that receives a document, and
//! it consults these limits. If they followed instantaneous free memory, a
//! document that validated a moment ago could fail validation later because a
//! browser tab grew. So the limits come from a *snapshot* that is refreshed at
//! document-open boundaries and by an explicit user change, never between. Live
//! availability is used only by the admission planner, which treats it as
//! advice and labels any refusal that rests on it as momentary.
use super::memory::{MemoryProbe, SystemMemory};
use serde::{Deserialize, Serialize};
use std::sync::RwLock;

pub const MIB: u64 = 1 << 20;
pub const GIB: u64 = 1 << 30;

/// Every working pixel is straight-alpha linear RGBA `f32`.
pub const BYTES_PER_WORKING_PIXEL: u64 = 16;

/// Most pixels any canvas may hold, whatever the budget.
///
/// Render code indexes pixels with `u32` arithmetic in places. Above 2^32 pixels
/// that wraps silently in a release build, so the ceiling sits at 2^31 where `i32`
/// and `u32` index math are both exact. A budget big enough to want more is
/// refused here rather than corrupting an index.
pub const HARD_MAX_CANVAS_PIXELS: u64 = 1 << 31;

/// Longest canvas edge, whatever the budget.
///
/// Vector and text coordinates are `f32`. At 131,072 an `f32` still resolves
/// 1/64 of a pixel, which is enough for antialiasing to be right; at a million
/// it would not be.
pub const HARD_MAX_CANVAS_DIMENSION: u32 = 131_072;

/// Longest edge of a *source file* PhotoForge will even describe.
///
/// A source is not a canvas: it is only ever read through a bounded window, so
/// it may be far larger than anything that could be opened whole. Coordinates
/// are `i32` in the tiling code, so this is the largest value that stays exact.
pub const HARD_MAX_SOURCE_DIMENSION: u64 = i32::MAX as u64;

/// The least budget the policy will ever produce.
pub const MIN_BUDGET_BYTES: u64 = 512 * MIB;

/// The budget used when the machine cannot be measured, and before the
/// application has installed a measured one.
///
/// It is exactly the budget the old fixed constants implied (1 GiB working
/// image, 4 GiB job), so behaviour is unchanged where nothing is known and the
/// tests that assert those figures stay deterministic on any host.
pub const UNMEASURED_BUDGET_BYTES: u64 = 4 * GIB;

/// The most a manual budget may be when the machine cannot be measured.
const UNMEASURED_MANUAL_CEILING: u64 = 64 * GIB;

/// The smallest working image the limits will ever allow, so that a tiny budget
/// still opens a small photograph.
const MIN_WORKING_IMAGE_BYTES: u64 = 64 * MIB;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(tag = "mode", rename_all = "camelCase")]
pub enum BudgetMode {
    /// Derived from the machine.
    #[default]
    Automatic,
    /// Chosen by the user, clamped to what the machine can honestly support.
    Manual { bytes: u64 },
}

/// Where a budget figure came from, so the UI never presents a guess as a
/// measurement.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum BudgetBasis {
    /// Computed from measured physical and available memory.
    Measured,
    /// The machine could not be measured; this is the conservative fallback.
    Unmeasured,
    /// The user set it.
    Manual,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Budget {
    pub mode: BudgetMode,
    pub basis: BudgetBasis,
    /// What PhotoForge may spend.
    pub bytes: u64,
    /// Held back for everything that is not PhotoForge. Zero when unmeasured.
    pub reserve_bytes: u64,
    /// The most a manual budget could be raised to on this machine.
    pub ceiling_bytes: u64,
    /// A manual request that had to be moved into range.
    pub clamped: bool,
}

/// The reserve held back from the machine for the operating system, WebView2,
/// the GPU driver and whatever else the user is running: at least 2 GiB, or 15%
/// of installed memory if that is more.
pub fn reserve_for(total_physical: u64) -> u64 {
    (2 * GIB).max(mul_div(total_physical, 15, 100))
}

/// Computes the budget for a mode on a machine.
///
/// **Automatic.** Spend at most what is left of installed memory after the
/// reserve, and at most what is available right now less half the reserve, and
/// of the smaller of those keep 85% — headroom for allocator fragmentation and
/// for the transient peak of an operation that briefly needs more than its
/// steady state. Never above 90% of installed memory, never below
/// [`MIN_BUDGET_BYTES`].
///
/// For a 128 GiB machine with 103 GiB free this gives about 79 GiB; for a 16 GiB
/// machine with 9 GiB free, about 6.6 GiB; for 8 GiB with 4 GiB free, about 2.6.
/// None of those are special-cased: they fall out of the formula.
pub fn compute_budget(mode: BudgetMode, system: Option<SystemMemory>) -> Budget {
    let Some(system) = system else {
        // Unmeasurable: a manual request is honoured up to a generous fixed
        // ceiling, an automatic one falls back to the legacy-equivalent budget.
        return match mode {
            BudgetMode::Automatic => Budget {
                mode,
                basis: BudgetBasis::Unmeasured,
                bytes: UNMEASURED_BUDGET_BYTES,
                reserve_bytes: 0,
                ceiling_bytes: UNMEASURED_MANUAL_CEILING,
                clamped: false,
            },
            BudgetMode::Manual { bytes } => {
                let clamped_bytes = bytes.clamp(MIN_BUDGET_BYTES, UNMEASURED_MANUAL_CEILING);
                Budget {
                    mode,
                    basis: BudgetBasis::Manual,
                    bytes: clamped_bytes,
                    reserve_bytes: 0,
                    ceiling_bytes: UNMEASURED_MANUAL_CEILING,
                    clamped: clamped_bytes != bytes,
                }
            }
        };
    };

    let reserve = reserve_for(system.total_physical);
    let ceiling = mul_div(system.total_physical, 90, 100).max(MIN_BUDGET_BYTES);
    match mode {
        BudgetMode::Automatic => {
            let by_total = system.total_physical.saturating_sub(reserve);
            let by_available = system.available_physical.saturating_sub(reserve / 2);
            let bytes =
                mul_div(by_total.min(by_available), 85, 100).clamp(MIN_BUDGET_BYTES, ceiling);
            Budget {
                mode,
                basis: BudgetBasis::Measured,
                bytes,
                reserve_bytes: reserve,
                ceiling_bytes: ceiling,
                clamped: false,
            }
        }
        BudgetMode::Manual { bytes } => {
            let clamped_bytes = bytes.clamp(MIN_BUDGET_BYTES, ceiling);
            Budget {
                mode,
                basis: BudgetBasis::Manual,
                bytes: clamped_bytes,
                reserve_bytes: reserve,
                ceiling_bytes: ceiling,
                clamped: clamped_bytes != bytes,
            }
        }
    }
}

/// `value * numerator / denominator` without overflowing for any `u64`.
fn mul_div(value: u64, numerator: u64, denominator: u64) -> u64 {
    ((u128::from(value) * u128::from(numerator)) / u128::from(denominator)) as u64
}

/// The ceilings that code consults, derived from one budget.
///
/// Every field is a *fraction of the same number*, which is the point: there is
/// one policy, and these are views of it. For the unmeasured 4 GiB budget they
/// reproduce the old constants exactly (1 GiB working image, 4 GiB job, 1 GiB
/// store, 1 GiB project entry).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ResourceLimits {
    /// The whole budget. The most a single job may peak at.
    pub job_bytes: u64,
    /// The largest single float working image: a quarter of the budget, because
    /// opening one costs the image itself, its canvas frame, scratch and output.
    pub working_image_bytes: u64,
    /// Pixel buffers the session store may retain, across layers and history.
    pub store_bytes: u64,
    /// The largest single decoded project entry.
    pub entry_bytes: u64,
}

impl ResourceLimits {
    pub fn from_budget(budget_bytes: u64) -> Self {
        let quarter = budget_bytes / 4;
        // Never below a small photograph, and never above the structural pixel
        // ceiling expressed in bytes.
        let working = quarter.clamp(
            MIN_WORKING_IMAGE_BYTES,
            HARD_MAX_CANVAS_PIXELS * BYTES_PER_WORKING_PIXEL,
        );
        Self {
            job_bytes: budget_bytes,
            working_image_bytes: working,
            store_bytes: quarter.max(working),
            entry_bytes: quarter.max(working),
        }
    }

    /// The limits that apply before anything has been measured.
    pub fn unmeasured() -> Self {
        Self::from_budget(UNMEASURED_BUDGET_BYTES)
    }

    /// Most pixels a canvas may hold under this budget.
    pub fn working_pixels(&self) -> u64 {
        (self.working_image_bytes / BYTES_PER_WORKING_PIXEL).min(HARD_MAX_CANVAS_PIXELS)
    }
}

struct State {
    budget: Budget,
    limits: ResourceLimits,
}

static STATE: RwLock<Option<State>> = RwLock::new(None);

fn default_state() -> State {
    State {
        budget: compute_budget(BudgetMode::Automatic, None),
        limits: ResourceLimits::unmeasured(),
    }
}

/// The limits currently in force. A snapshot: it does not change while a
/// document is open.
pub fn limits() -> ResourceLimits {
    match STATE.read() {
        Ok(guard) => guard
            .as_ref()
            .map_or_else(ResourceLimits::unmeasured, |state| state.limits),
        // A poisoned lock means a panic while the policy was being replaced.
        // The conservative fallback is better than propagating it into every
        // validation in the process.
        Err(_) => ResourceLimits::unmeasured(),
    }
}

/// The budget currently in force.
pub fn budget() -> Budget {
    match STATE.read() {
        Ok(guard) => guard
            .as_ref()
            .map_or_else(|| default_state().budget, |state| state.budget),
        Err(_) => default_state().budget,
    }
}

/// Recomputes the budget from the machine and installs it.
///
/// Call at start-up, at document-open boundaries, and when the user changes the
/// setting. Not between: see the module note on why the limits must be stable
/// while a document is open.
pub fn configure(mode: BudgetMode, probe: &dyn MemoryProbe) -> Budget {
    let budget = compute_budget(mode, probe.system());
    let limits = ResourceLimits::from_budget(budget.bytes);
    if let Ok(mut guard) = STATE.write() {
        *guard = Some(State { budget, limits });
    }
    budget
}

/// A saved copy of the policy in force, so a failed open can put it back.
///
/// The limits are refreshed for a document that is about to open. If that open
/// then fails, the document that was already open is still there and must go on
/// being judged by the limits it was admitted under, not by new ones that may be
/// lower. `restore` is how it gets them back.
#[derive(Debug, Clone, Copy)]
pub struct PolicySnapshot {
    budget: Budget,
    limits: ResourceLimits,
    installed: bool,
}

pub fn snapshot() -> PolicySnapshot {
    match STATE.read() {
        Ok(guard) => match guard.as_ref() {
            Some(state) => PolicySnapshot {
                budget: state.budget,
                limits: state.limits,
                installed: true,
            },
            None => PolicySnapshot {
                budget: default_state().budget,
                limits: ResourceLimits::unmeasured(),
                installed: false,
            },
        },
        Err(_) => PolicySnapshot {
            budget: default_state().budget,
            limits: ResourceLimits::unmeasured(),
            installed: false,
        },
    }
}

pub fn restore(saved: PolicySnapshot) {
    if let Ok(mut guard) = STATE.write() {
        *guard = saved.installed.then_some(State {
            budget: saved.budget,
            limits: saved.limits,
        });
    }
}

/// Returns to the unmeasured default. Used by tests that install a policy.
pub fn reset() {
    if let Ok(mut guard) = STATE.write() {
        *guard = None;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn machine(total_gib: u64, available_gib: u64) -> Option<SystemMemory> {
        Some(SystemMemory {
            total_physical: total_gib * GIB,
            available_physical: available_gib * GIB,
        })
    }

    fn gib(bytes: u64) -> f64 {
        bytes as f64 / GIB as f64
    }

    /// The three machines the spec talks about, derived rather than special-cased.
    #[test]
    fn the_automatic_budget_scales_with_the_machine() {
        let big = compute_budget(BudgetMode::Automatic, machine(128, 103));
        let mid = compute_budget(BudgetMode::Automatic, machine(16, 9));
        let small = compute_budget(BudgetMode::Automatic, machine(8, 4));

        assert!(
            (78.0..81.0).contains(&gib(big.bytes)),
            "128 GiB machine: {}",
            gib(big.bytes)
        );
        assert!(
            (6.0..7.2).contains(&gib(mid.bytes)),
            "16 GiB machine: {}",
            gib(mid.bytes)
        );
        assert!(
            (2.2..3.0).contains(&gib(small.bytes)),
            "8 GiB machine: {}",
            gib(small.bytes)
        );
        assert!(big.bytes > mid.bytes && mid.bytes > small.bytes);
        assert_eq!(big.basis, BudgetBasis::Measured);
    }

    /// Available memory must matter, or a machine that is already full is
    /// budgeted as if it were idle.
    #[test]
    fn a_busy_machine_gets_a_smaller_budget_than_an_idle_one() {
        let idle = compute_budget(BudgetMode::Automatic, machine(32, 28));
        let busy = compute_budget(BudgetMode::Automatic, machine(32, 8));
        assert!(
            busy.bytes < idle.bytes / 2,
            "idle {}, busy {}",
            gib(idle.bytes),
            gib(busy.bytes)
        );
    }

    /// Installed memory must matter too, or a machine with lots free but little
    /// installed spends past what the OS can give.
    #[test]
    fn the_budget_never_exceeds_ninety_percent_of_installed_memory() {
        for total in [4u64, 8, 16, 32, 64, 128, 512, 2048] {
            let budget = compute_budget(BudgetMode::Automatic, machine(total, total));
            assert!(
                budget.bytes <= total * GIB * 90 / 100,
                "{total} GiB machine budgeted {}",
                gib(budget.bytes)
            );
            assert!(budget.bytes >= MIN_BUDGET_BYTES);
        }
    }

    #[test]
    fn a_machine_too_small_for_its_own_reserve_still_gets_the_floor() {
        let tiny = compute_budget(BudgetMode::Automatic, machine(1, 1));
        assert_eq!(tiny.bytes, MIN_BUDGET_BYTES);
    }

    #[test]
    fn an_unmeasurable_machine_gets_the_legacy_equivalent_budget() {
        let budget = compute_budget(BudgetMode::Automatic, None);
        assert_eq!(budget.bytes, UNMEASURED_BUDGET_BYTES);
        assert_eq!(budget.basis, BudgetBasis::Unmeasured);
        assert_eq!(budget.reserve_bytes, 0);
    }

    #[test]
    fn a_manual_budget_is_honoured_inside_the_range_and_clamped_outside_it() {
        let system = machine(64, 50);
        let fine = compute_budget(BudgetMode::Manual { bytes: 20 * GIB }, system);
        assert_eq!(fine.bytes, 20 * GIB);
        assert!(!fine.clamped);
        assert_eq!(fine.basis, BudgetBasis::Manual);

        // More than the machine has is refused, and says so.
        let greedy = compute_budget(BudgetMode::Manual { bytes: 500 * GIB }, system);
        assert!(greedy.bytes <= 64 * GIB * 90 / 100);
        assert!(greedy.clamped);

        let starved = compute_budget(BudgetMode::Manual { bytes: 1 }, system);
        assert_eq!(starved.bytes, MIN_BUDGET_BYTES);
        assert!(starved.clamped);
    }

    #[test]
    fn the_budget_arithmetic_cannot_overflow() {
        // A hostile or buggy probe reporting u64::MAX must not wrap.
        let absurd = Some(SystemMemory {
            total_physical: u64::MAX,
            available_physical: u64::MAX,
        });
        let budget = compute_budget(BudgetMode::Automatic, absurd);
        assert!(budget.bytes <= u64::MAX / 10 * 9 + 1);
        let limits = ResourceLimits::from_budget(budget.bytes);
        assert!(limits.working_pixels() <= HARD_MAX_CANVAS_PIXELS);
    }

    /// The old constants, reproduced exactly, so the unmeasured default changes
    /// nothing and every test that asserts them is deterministic on any host.
    #[test]
    fn the_unmeasured_limits_equal_the_old_fixed_constants() {
        let limits = ResourceLimits::unmeasured();
        assert_eq!(limits.working_image_bytes, 1_073_741_824);
        assert_eq!(limits.job_bytes, 4_294_967_296);
        assert_eq!(limits.store_bytes, 1_073_741_824);
        assert_eq!(limits.entry_bytes, 1_073_741_824);
        assert_eq!(limits.working_pixels(), 67_108_864);
    }

    /// The structural ceiling holds however large the budget gets.
    #[test]
    fn no_budget_can_lift_the_structural_pixel_ceiling() {
        let limits = ResourceLimits::from_budget(u64::MAX / 2);
        assert_eq!(limits.working_pixels(), HARD_MAX_CANVAS_PIXELS);
        // 2^31 pixels is the point where u32 index arithmetic is still exact.
        assert!(limits.working_pixels() <= u64::from(u32::MAX));
    }

    #[test]
    fn a_larger_budget_allows_proportionally_larger_images() {
        let small = ResourceLimits::from_budget(4 * GIB);
        let large = ResourceLimits::from_budget(64 * GIB);
        assert_eq!(large.working_pixels(), small.working_pixels() * 16);
        assert!(large.working_pixels() > 1_000_000_000);
    }

    #[test]
    fn a_tiny_budget_still_opens_a_small_photograph() {
        let limits = ResourceLimits::from_budget(MIN_BUDGET_BYTES);
        // 64 MiB of floats is a 4-megapixel image.
        assert!(limits.working_pixels() >= 4_000_000);
    }

    #[test]
    fn the_reserve_is_at_least_two_gib_and_grows_with_the_machine() {
        assert_eq!(reserve_for(4 * GIB), 2 * GIB);
        assert_eq!(reserve_for(128 * GIB), mul_div(128 * GIB, 15, 100));
    }
}
