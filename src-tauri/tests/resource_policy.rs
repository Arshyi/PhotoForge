//! Process-wide resource policy, tested in its own process.
//!
//! The limits are global state, so a test that installs a policy would race with
//! every other test in the same binary. Integration tests each get their own
//! process, and everything here runs inside a single #[test] so ordering is fixed.
use photoforge_lib::resources::{self, memory::FixedProbe, BudgetBasis, BudgetMode, ModeChange};

const GIB: u64 = 1 << 30;

#[test]
fn the_policy_follows_the_machine_and_defers_only_reductions() {
    // Before anything is installed the limits are the legacy-equivalent ones, so
    // every test that predates the resource manager is deterministic.
    resources::reset();
    assert_eq!(resources::max_working_pixels(), 67_108_864);
    assert_eq!(resources::budget().basis, BudgetBasis::Unmeasured);

    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("resources.json");

    // A large machine raises the ceiling well past the old constant.
    let large = FixedProbe::machine(128 * GIB, 103 * GIB);
    let installed = resources::configure(BudgetMode::Automatic, &large);
    assert_eq!(installed.basis, BudgetBasis::Measured);
    assert!(
        resources::max_working_pixels() > 1_000_000_000,
        "a 128 GiB machine is still limited to {} pixels",
        resources::max_working_pixels()
    );
    // ...and an image the old constant refused now passes.
    assert!(resources::checked_pixels(30_000, 30_000).is_ok());

    // The structural ceilings hold regardless of how much memory there is.
    assert!(resources::checked_pixels(200_000, 1).is_err());
    assert!(resources::checked_pixels(0, 10).is_err());
    assert!(resources::checked_pixels(u32::MAX, u32::MAX).is_err());

    // Lowering the budget while a document may be open is saved but deferred.
    let small = FixedProbe::machine(8 * GIB, 4 * GIB);
    let ModeChange { budget, applied } =
        resources::change_mode_at(&path, BudgetMode::Automatic, &small).unwrap();
    assert!(!applied, "a reduction was applied mid-session");
    assert_eq!(budget.bytes, resources::budget().bytes);
    assert!(
        resources::checked_pixels(30_000, 30_000).is_ok(),
        "the open document would now fail"
    );

    // The saved choice is what the next open installs, on the small machine.
    resources::settings::save_mode_to(&path, BudgetMode::Automatic).unwrap();
    let mode = resources::settings::load_mode_from(&path);
    let refreshed = resources::configure(mode, &small);
    assert!(refreshed.bytes < 4 * GIB);
    assert!(resources::checked_pixels(30_000, 30_000).is_err());

    // Raising applies immediately.
    let raised =
        resources::change_mode_at(&path, BudgetMode::Manual { bytes: 6 * GIB }, &small).unwrap();
    assert!(raised.applied);
    assert_eq!(raised.budget.bytes, 6 * GIB.min(small_ceiling(&small)));

    // An unmeasurable machine falls back rather than failing.
    resources::configure(BudgetMode::Automatic, &FixedProbe::unmeasurable());
    assert_eq!(resources::budget().basis, BudgetBasis::Unmeasured);
    assert_eq!(resources::max_working_pixels(), 67_108_864);

    resources::reset();
}

fn small_ceiling(probe: &FixedProbe) -> u64 {
    // 90% of installed memory, as the policy defines the manual ceiling.
    probe.system.unwrap().total_physical / 100 * 90
}
