# Phase 1.1 audit checklist

This checklist tracks the release-hardening audit that began from commit `ccb8e12` and version `0.1.0` on July 18, 2026. It records evidence, not aspirations. Items move between sections as the audit proceeds.

## Status definitions

- **Confirmed working** — exercised successfully in this audit or proven by a focused automated test.
- **Needs manual validation** — promised Phase 1 behavior that still needs real-application evidence.
- **Confirmed defect** — promised Phase 1 behavior that demonstrably fails or is unsafe.
- **Architectural concern** — implementation risk that requires analysis or hardening even if no user-visible failure has occurred.
- **Deferred enhancement** — useful behavior not promised by Phase 1.
- **Not reproducible** — investigated report or suspected failure that could not be reproduced, with conditions recorded.

## Baseline

- Repository: `E:\PhotoForge`
- Branch: `main`, tracking `origin/main`
- Baseline commit: `ccb8e12` (`Build PhotoForge Phase 1`)
- Baseline application version: `0.1.0`
- Baseline release artifacts: portable EXE, NSIS setup EXE, and MSI under `release/`
- Working tree at audit start: clean

## Confirmed working

- [x] Repository history, remote tracking, package manifests, and documentation are internally consistent at baseline.
- [x] Release-mode executable starts and remains responsive in the existing startup smoke test.
- [x] The edit history stores operation snapshots, not image buffers.
- [x] Source images are decoded once per open and previews are encoded as local in-memory PNG data URLs.
- [x] Tauri registers only `open_image`, `render_preview`, and `export_image`; no shell command is registered.
- [x] CSP contains no remote script, style, image, or general network source.
- [x] Original-path overwrite protection and unsupported export extensions have focused Rust tests.
- [x] Pixel clamping, brightness, grayscale, reflection, rotation, ordering, reset, undo, and redo have baseline tests.
- [x] Final source passes formatting, warnings-as-errors Clippy, 29 Rust tests, Svelte diagnostics, 10 frontend tests, and production/release builds.
- [x] PNG/JPEG/WebP/grayscale, Unicode/space paths, read-only input, invalid/corrupt inputs, dimension bounds, alpha, export dimensions, and original immutability have focused tests.
- [x] Final portable 0.1.1 starts outside the source tree with one responsive window, no console, stable idle memory/CPU, and no observed TCP connections.
- [x] A 6000×4000 JPEG loads from removable storage with correct metadata and a current preview in the final package.
- [x] The final NSIS installer installs, creates a Start menu shortcut, launches version 0.1.1, and uninstalls cleanly.

## Needs manual validation after this release audit

### Startup and lifecycle

- [x] One expected window; close and reopen; no release console window.
- [x] No observed startup connection; idle CPU and baseline/final memory observations.
- [x] Portable executable launched outside the source tree.
- [x] NSIS install, Start menu launch, and uninstall.
- [ ] Separately install the MSI on a disposable Windows VM.

### Opening files

- [x] Native picker in baseline and final package.
- [ ] Drag-and-drop and replacement-open history clearing by human pointer workflow (the code paths and generation protocol were reviewed/tested).
- [x] Supported formats, alpha, grayscale, tiny/high-resolution, unusual paths, read-only, and invalid/corrupt behavior covered by focused tests; representative landscape and 24 MP inputs exercised in packaged apps.

### Editing and history

- [x] Neutral exactness, extrema validation, deterministic operations, operation ordering, transforms, alpha/channel behavior, history cap/coalescing, undo/redo/reset, and branch semantics have focused tests.
- [x] Packaged keyboard Brightness maximum and current-preview state; baseline packaged undo/redo and branch clearing.
- [ ] Human visual review of every control/preset extreme on a calibrated representative photo set.

### Preview, comparison, and viewport

- [x] Same-geometry swipe and rotated independent-geometry layouts have component tests.
- [ ] Human visual matrix for every transform, zoom/fit/resize/maximize combination and exact 125%/150% scaling.

### Export

- [x] PNG/JPEG/WebP explicit encoders, Unicode/space loading, full-resolution dimensions, original overwrite rejection, alpha/JPEG policy, transform orientation, and unchanged source bytes have focused tests.
- [ ] Human save-dialog matrix for overwrite confirmation, inaccessible destination, and cancel on a disposable account/volume.

## Confirmed defects fixed in 0.1.1

- [x] Competing opens lacked a document generation.
- [x] Preview blocking work was not bounded/coalesced.
- [x] The debounce interval allowed a stale preview to remain current temporarily.
- [x] Rotated comparison shared incompatible geometry.
- [x] Slider input created noisy undo entries and history had no cap.
- [x] Settings/background focus and unavailable-control keyboard behavior needed hardening.
- [x] JPEG alpha/quality behavior was implicit.
- [x] The 120 MP memory ceiling was not defensible for buffer-based pipelines.
- [x] Export concurrency and empty-pipeline copies were avoidable.
- [x] Pipelines repeated dynamic-to-RGBA conversion for every operation.

## Architectural concerns

- [x] Preview work is bounded/coalesced on both sides of IPC.
- [x] Open and preview work is scoped to a document generation.
- [x] Memory ceiling reduced/documented with decoder limits.
- [x] Rotated comparison repaired with independent geometry.
- [x] Export policies explicit and tested.
- [ ] Existing-image export still writes directly to the confirmed destination; atomic replacement is deferred.
- [x] Status uses polite atomic live semantics; dialog focus/background inertness and disabled controls were hardened.

## Deferred enhancements

- [ ] A close-document command is not part of Phase 1; process exit and opening a replacement image are the applicable invalidation paths.
- [ ] ICC profile, EXIF metadata, HDR, animation, tiled processing, and color-managed linear-light operations remain outside Phase 1.
- [ ] Neural restoration, OCR, natural-language editing, cloud services, telemetry, and plugin systems remain explicitly out of scope.

## Not reproducible

No suspected defect has been moved here yet.

## Required final verification

- [x] `cargo fmt --manifest-path src-tauri/Cargo.toml --check`
- [x] `cargo clippy --manifest-path src-tauri/Cargo.toml --workspace --all-targets -- -D warnings`
- [x] `cargo test --manifest-path src-tauri/Cargo.toml --workspace` — 29 passed
- [x] `npm run check` — 0 errors, 0 warnings
- [x] `npm run test` — 10 passed
- [x] `npm run build`
- [x] `npm run tauri build`
- [x] Final 0.1.1 external-location portable and installed-app startup smoke tests.
- [x] Final SHA-256 manifest under `release/SHA256SUMS.txt`.
