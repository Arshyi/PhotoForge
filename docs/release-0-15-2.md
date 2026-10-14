# PhotoForge 0.15.2 development — storage and installer hardening

Status: implementation/validation in progress on `release-0.15.2-hardening`.
This is not a released version. The published 0.15.1 artifacts remain unchanged.

## Build storage

- The Cargo helper defaults to compact validation symbols and no incremental cache;
  full debugging is an explicit opt-in. Feature selection, optimisation, assertions,
  overflow checks and release settings are unchanged.
- The full eight-step/four-feature gate matrix checks free space and retained target
  size before and after each command. Refusals fail the gate and never delete anything.
- Hosted CI uses the same compact helper and storage checks, and verifies a tiny actual
  Rust fixture for debug assertions and overflow checks. PR pushes do not also launch a
  duplicate branch-push workflow; main and PR checks remain enabled.
- 29 tool regressions passed locally. Two actual compact-profile Rust probe tests
  passed, confirming assertions and overflow checks. The unchanged frontend passed
  1,393 tests, TypeScript, Svelte (zero errors/warnings), and production build before
  the version bump; final-head hosted and product-matrix checks are still pending.
  No cache-size reduction or runtime performance figure is claimed before measurement.

The disk audit and approved cleanup receipt are private local work products, not public
repository documents. The cleanup was not a source-code change and is not repeated here.

## Installer and release acceptance

- A pinned Tauri WiX template now reads INSTALLDIR only from the machine registry,
  writes product/shortcut key paths only to the native machine view, preserves the
  existing upgrade code, and leaves NSIS explicitly per-user.
- MSI versions through 0.15.1 are detected and refused for automatic upgrade: the old
  uninstaller could remove overlapping per-user NSIS files. Migration requires explicit
  removal of the old MSI, not silent reuse/removal of a person's existing directory.
- Future packages refuse downgrades. The upstream template's MIT notice is retained
  and bundled separately; it is not an overall PhotoForge license.
- Seven source-policy regressions catch hive/view/scope/upgrade-identity, shared-Desktop
  removal, and unsafe
  migration changes. This is source lint, not built-MSI or native lifecycle proof.

Built MSI table inspection, installer lifecycle, native display-scaling and
cross-application verification are pending. The upstream WebView2 prerequisite handling
is retained and is not proof of runtime availability for every Windows account.

The user chose to leave the project license unset for now and confirmed that no trusted
code-signing certificate/service is available. No project LICENSE file or signing identity
will be invented. Third-party license obligations/notices remain separate from that choice.
