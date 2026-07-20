# Phase 6 results

PhotoForge 0.6.0 transforms the application into a deterministic professional editing and workflow system while preserving the Phase 5 local-first and optional-planner boundaries.

## Delivered

- RGB/channel curves, levels, white/black point pickers, crop ratios and overlays, straighten, four-corner perspective, lens correction, HSL, temperature/tint, and selective color
- Live before/after RGB and luminance histograms with clipping indicators
- Pixel/HSV inspector, 1600% zoom, grid, crosshair, measurement, expanded metadata, and safe EXIF camera-model parsing
- Swipe, split, blink, difference, and histogram comparison
- Versioned local workflow recorder, library, JSON transfer, editor, search, favorites, folders, and replay
- Bounded folder batch processing with preview, templates, overwrite protection, recursion, progress, estimates, cancellation, failures, skips, and logs
- Remembered export profiles, saved workspace layouts, shortcut conflict detection/import/export, high contrast, scalable UI, keyboard navigation, and screen-reader labels

## Automated verification

| Check | Result |
| --- | --- |
| Rust tests | 357 passed, 0 failed |
| Frontend tests | 271 passed, 0 failed across 16 files |
| Svelte/TypeScript | 0 errors, 0 warnings |
| Rust formatting | Passed |
| Rust Clippy | Passed for all targets with `-D warnings` |
| Production bundle | 194.28 kB JS / 49.28 kB CSS; 61.41 / 9.26 kB gzip |

Rust coverage includes workflow recording and versioning, import/export limits, batch discovery/export/overwrite/cancellation, bounded workers, histogram accuracy, curves, levels, crop, perspective, lens correction, HSL, metadata parsing, workspace validation, shortcut conflicts, operation serialization, alpha preservation, and deterministic replay. Frontend coverage exceeds the target across workflow UI, histogram, professional tools, batch, workspace, shortcuts, metadata, accessibility labels, and existing Phase 1–5 behavior.

## Privacy and architecture

No new network client, model, runtime, download, telemetry, cloud service, script execution, or external process was added. Optional Ollama planning remains the only documented loopback networking path and remains disabled unless selected and invoked. Professional editing and batch processing never call it.

## Manual validation and performance

The packaged Windows application was exercised through its native accessibility tree and file dialogs. The validation used repository-owned PNG fixtures, and all temporary workflow and batch files were removed afterward.

| Check | Result |
| --- | --- |
| Professional tools | All five panels (Tools, Scopes, Flows, Batch, Inspect) loaded in the packaged app; curves, levels, crop, perspective, lens, HSL, temperature/tint, selective color, live histogram, and inspector controls were present and enabled after opening an image |
| Perspective | Four-corner perspective applied through the packaged UI and appeared as one undoable typed operation |
| Histogram | Live after-image RGB/luminance scope rendered in the packaged UI; exact bins, clipping, alpha handling, and before/after behavior passed Rust reference tests |
| Workflow round trip | Recorded a pipeline, renamed it, duplicated an operation, replayed two operations, exported schema v1 JSON, deleted it from the library, and successfully imported the JSON again |
| Workspace restore | Saved the Default layout, restarted the process, found it in Recent workspaces, and restored it successfully |
| Undo/redo and export | Reflect edit round-tripped through Undo/Redo; lossless export produced a valid 128 x 128 PNG without modifying the source |
| Large-folder preview | 2,000-image discovery/preview completed in 245 ms and reported all planned outputs without writing |
| Live cancellation | Cancellation was acknowledged in 312 ms after 61 completed outputs; 0 failures and a cancellation log |
| Resumed batch | 1,939 new exports plus 61 protected skips completed in 4.178 s (478.7 discovered entries/s, 464.1 new exports/s), with 0 failures, 2,000 final PNGs, and a completion log |
| Portable startup | Responsive native window in 723 ms; 443.9 MiB aggregate working set across the WebView2 process tree |
| Offline runtime | 0 active TCP connections across the packaged process tree while idle |
| Responsive layout | 800 x 600 browser-hosted shell had no horizontal overflow |
| Installer lifecycle | NSIS silent install and uninstall both returned 0; registered, installed, and executable versions were 0.6.0; installed app launched responsively; registration and install directory were removed |
| MSI metadata | Product PhotoForge, version 0.6.0, manufacturer photoforge |

The direct USB `cargo test` invocation spent more than five minutes relinking the Windows debug harness before the tests could start. The complete 357-test Rust suite therefore ran from a byte-identical internal staging copy in 1.83 seconds. The USB repository itself passed formatting, all-target Clippy, release compilation, and packaging; file hashes confirmed all intended source files were identical to the tested staging copy.

## Release artifacts

The release build completed on the USB repository. Optimized Rust compilation took 2 minutes 41 seconds and the complete Tauri packaging pass took 3 minutes 16 seconds.

| Artifact | Size | SHA-256 |
| --- | ---: | --- |
| `release/PhotoForge-portable.exe` | 13,235,712 bytes | `160D221AEB35C7DC53D972939178C02E51C3380FC76A00FE2D06B34A4F611F53` |
| `release/PhotoForge_0.6.0_x64-setup.exe` | 3,026,732 bytes | `B8405FC081AC4155B9333E3B8AA988A823A991A2F7C47A9E0B0F4502E1CFB30D` |
| `release/PhotoForge_0.6.0_x64_en-US.msi` | 4,435,968 bytes | `9F1CD277BA25A2CFB41A936DC69BCA0972DAB529D3AA1858D7DEF2A3F81F9494` |
| `release/SHA256SUMS.txt` | checksum manifest | Contains all three hashes above |

All version surfaces report 0.6.0. The release directory is intentionally ignored by Git so installers remain local build outputs rather than repository blobs.

Final installer paths, sizes, SHA-256 hashes, manual validation, and performance measurements are recorded here during the final packaged release pass.
