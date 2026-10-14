# PhotoForge 0.15.1 — hardening and integration

This integrates the clipboard validation, HDR/PSD/SVG fixes, and window layout work documented
in [15.10](phase-15-10-results.md), [15.11](phase-15-11-results.md), and
[15.12](phase-15-12-results.md). Those reports retain the measurements made when each change was
developed. This page records the integration and release checks separately.

Status: stable Phase 15 hardening checkpoint, with the retained limits below. Phase 16
experiments are not included. Downloads, once published, are on the
[v0.15.1 release page](https://github.com/Arshyi/PhotoForge/releases/tag/v0.15.1).

The final Tauri build used merged source `9a75fce521dfdf83bdfd7bbde95066c187b1f9ee`.
The release-evidence commit adds documentation and checksums only; it does not change
the application code, dependency locks, or build configuration.

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
  The hosted run below was inspected after completion, including its test summaries.

## Source gates (local and hosted)

- Frontend: 1,393 tests in 94 files, zero failures and zero unhandled errors.
- TypeScript and Svelte: zero errors; Svelte reports zero warnings.
- Frontend production build and all four gate-runner regressions pass.
- PSD estimate regressions: seven passed.
- Control-character scan: zero affected files.
- Full Rust matrix, with real DNG fixtures and 60 test binaries in each configuration:

  | Configuration | Passed | Failed | Clippy diagnostics |
  | --- | ---: | ---: | ---: |
  | All features | 2,019 | 0 | 0 |
  | Without GPU | 2,012 | 0 | 0 |
  | Without inference | 2,011 | 0 | 0 |
  | Without plugins | 1,980 | 0 | 0 |

- [Hosted Windows run 38534824358](https://github.com/Arshyi/PhotoForge/actions/runs/38534824358)
  passed all five jobs with the same counts. Its head was `6c1d1edc777df1e804dda91c333628880a692485`;
  the merged build-source tree was verified identical to that head.
- All 18 format/HDR mutants and five layout mutants were caught by regressions. The
  sources were restored and the pristine all-feature library passed 1,644 tests afterwards.

## Final binary and installer checks

- Final Tauri-built executable: all 32 layout cells clean (eight viewport sizes, four
  emulated device pixel ratios). This is page-layout evidence, not native Windows DPI
  validation or the complete hands-on GUI matrix.
- Final clipboard recheck: **not executed**. Whole-clipboard backup could not open the
  clipboard, so the harness refused before changing it or launching the app. The earlier
  direct-Cargo candidate passed all 14 checks, including exact restoration; that result
  is not presented as a final-binary pass.
- Final EXR/SVG/PSD window driver completed with only the expected luminance/chroma EXR
  refusal. All 16 exported files are hash-identical to the earlier candidate's outputs.
  Independent readers open the files; the three EXRs have zero pixel difference, the
  three SVGs render identically to their selected source fixtures, and all seven PSDs
  pass the strict independent structural checker. PSD structure is not visual fidelity:
  omitted shapes/styles remain disclosed, and one optional source-composite comparison
  could not run because the external oracle lacked `aggdraw`.
- Per-user NSIS: isolated install, version registration, image open, zero IPC errors,
  normal app close, and uninstall passed. No pre-existing installation, app process,
  shortcut, or normal per-user install path was replaced.
- MSI: administrative extraction passed and did not register an installed product.
  This is **not** an elevated all-users install/launch/uninstall.
- Installer payloads match the final executable except for the documented three-byte
  Tauri bundle-kind marker. The portable ZIP contains the unmodified final executable,
  the existing third-party notices, and a README.
- Final dependency inspection found neither `VCRUNTIME140.dll` nor `VCRUNTIME140_1.dll`:
  Tauri's build uses the static Visual C++ runtime. The earlier direct-Cargo candidate
  did depend on those DLLs and is not the distributed portable executable. WebView2
  remains required.
- SHA-256 of the final executable:
  `cdc42a1c24cd66fb1841df47531a82b6211a9464fb7fa85beddc48ecfb0862b6`.
  Download hashes are in [SHA256SUMS.txt](../SHA256SUMS.txt).

## Real-file corpora

- 315 PSD/PSB files read: 289 supported imports, 26 documented refusals, no unexpected
  failure. The 289 supported round trips gave 273 exact composites and 16 with less than
  one level mean difference. Five PSD golden exports are byte-identical to the baseline.
- EXR: 62 cases, 52 import/export round trips and ten documented refusals. Independent
  OpenEXR reading reported zero problems; f32 round trips had zero difference and the
  largest f16 relative difference was `4.86e-4`.
- TIFF: all 70 input cases passed their criteria; seven exported files passed the
  independent Pillow/tifffile checks with zero reported problems.
- SVG: 230 icon imports passed. The resvg comparison remained at median mean difference
  `0.219`, p90 `0.863`, worst `2.199` on a 0–255 scale, unchanged from the baseline.

## Retained limits

The installers are unsigned. Native Windows display scaling, mixed-monitor behavior, and
cross-application testing in Photoshop, Lightroom, Nuke, Blender, or Inkscape remain unverified.
DevTools device-pixel-ratio checks exercise page layout only. The MSI's stock template can reuse
a per-user NSIS install directory while registering the product per machine; the per-user NSIS
installer remains the recommended installer. DNG is the supported camera RAW format. The other
format and rendering limits in the Phase 15 ledger still apply unless one of the linked reports
explicitly closes them.

No signing certificate or signing service was supplied: the final executable and both
installers report `NotSigned`. No production neural model or model-download feature ships.
Local image processing is not a zero-network claim for the WebView2 process tree. The
existing third-party notices are included; no project license or exhaustive transitive-license
audit has been invented for this release.
