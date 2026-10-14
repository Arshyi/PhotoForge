# PhotoForge 0.15.1 — hardening and integration

This integrates the clipboard validation, HDR/PSD/SVG fixes, and window layout work documented
in [15.10](phase-15-10-results.md), [15.11](phase-15-11-results.md), and
[15.12](phase-15-12-results.md). Those reports retain the measurements made when each change was
developed. This page records the integration and release checks separately.

Status: validation in progress; no 0.15.1 release is claimed yet.

## Integration repairs

- Command-palette commands execute inside the promise handler, so both synchronous throws and
  asynchronous rejections are handled after the palette closes. The previous unhandled `boom`
  was a defect, not an acceptable test warning.
- The Rust gate runner finishes every feature configuration and returns failure if any command
  fails, any configuration reports failed tests, or a successful command produces no test
  summary. Four regression cases substitute the cargo boundary and exercise the real runner.
- PSD export admission follows the tree the writer emits: transformed pixel layers and semantic
  layers count as canvas-sized rasters, transformed groups count once, and masks are included.
  Three regressions cover these cases. The memory figure remains an estimate, not a measured
  process peak.
- The control-character scanner now returns failure when it finds a bad file and scans scripts
  and workflows too. A vertical tab in the layout-harness documentation was removed.
- Windows CI checks the frontend and all four Rust feature configurations, using pinned action
  commits, read-only repository permissions, a fixed Rust toolchain, and the real DNG fixtures.
  The fixture downloader verifies both size and SHA-256, including files already present.
  Remote results must be inspected before this is called verified.

## Current local checks

- Frontend: 1,393 tests in 94 files, zero failures and zero unhandled errors.
- TypeScript and Svelte: zero errors; Svelte reports zero warnings.
- Frontend production build and all four gate-runner regressions pass.
- PSD estimate regressions: seven passed.
- Control-character scan: zero affected files.
- Packaged candidate layout: all 32 cells clean (eight viewport sizes, four emulated device
  pixel ratios). This is not native Windows DPI validation.
- Packaged candidate clipboard: all 14 checks passed, including exact clipboard restoration.
- Full four-feature Rust matrix and format corpora: in progress.

## Retained limits

The installers are unsigned. Native Windows display scaling, mixed-monitor behavior, and
cross-application testing in Photoshop, Lightroom, Nuke, Blender, or Inkscape remain unverified.
DevTools device-pixel-ratio checks exercise page layout only. The MSI's stock template can reuse
a per-user NSIS install directory while registering the product per machine; the per-user NSIS
installer remains the recommended installer. DNG is the supported camera RAW format. The other
format and rendering limits in the Phase 15 ledger still apply unless one of the linked reports
explicitly closes them.
