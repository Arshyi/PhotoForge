# Phase 7.1 results

PhotoForge 0.7.1 is the completion and hardening release for Phase 7. It does not begin Phase 8. The release keeps selection and image processing deterministic and local: it adds no cloud service, telemetry, model download, neural segmentation, generative editing, OCR, account, payment, marketplace, or executable-plugin behavior.

This document separates implementation facts from measured release observations and states where hands-on coverage was incomplete.

## Completed implementation

### Geometry and stage alignment

- Crop, exact 90°/180°/270° rotation, horizontal reflection, arbitrary straighten, and perspective are represented as checked geometry chains with dimensions recorded at every stage.
- A geometry edit remaps the active mask, named masks, and persistent embedded workflow-mask snapshots in one keyed, bounded transaction. The edit pipeline and selection state are committed together only after every expected result is present and valid.
- Crop and exact rotation/reflection use discrete coverage transforms when they are an applicable suffix. General old-chain to new-chain rebases use destination-to-source bilinear coverage sampling once, clamp output coverage, and use zero outside the source domain.
- Perspective inversion is bounded and rejects non-finite, folded, near-singular, or non-convergent transforms. Stage mismatches, stale documents, cancellation, duplicate keys, missing keys, and oversized batches fail closed.
- Full-resolution masked operations require exact mask dimensions at their pipeline stage. Preview preparation tracks full and preview stages independently and creates temporary bilinear preview masks without rewriting canonical snapshots.
- Undo/Redo treats a completed geometry remap as one compound edit/selection action.

Lens correction is not included in the 0.7.1 mask-remap contract.

### Thumbnails and overlays

- Named-mask rows render actual grayscale coverage previews with area averaging and aspect-fit dimensions.
- Rendering is lazy through intersection observation and idle scheduling. Cache identity includes checksum and source/target dimensions, so mask edits and remaps invalidate only affected entries.
- The shared thumbnail LRU is bounded to 96 entries and approximately 2 MiB. It performs no disk writes or network work.
- The image stage can composite the active mask and visible named masks for review while retaining each mask independently.

### Numerical progress and cancellation

- Request-scoped progress records document ID, request ID, operation, phase, completed units, total units, and queued/running/cancelling/completed/cancelled/failed state.
- Rasterization/composition, Magic Wand, Color Range, feather/morphology/cleanup/refinement, and geometry remapping report implementation work units such as pixels, passes, or output rows.
- The frontend ignores stale progress, prevents percentage regression, bounds determinate output to 0–100%, waits 180 ms before display, and clears terminal state. Cancellation remains `cancelling` until worker acknowledgement.
- JSON and PNG mask import/export remain bounded one-shot file commands. They do not expose granular numerical file-I/O progress in 0.7.1.

### Pointer pressure

- Pen pressure is optional and disabled by default. Size response is enabled when pressure is turned on; opacity response remains off until selected. The default minimum size/opacity factors are 35%/25%.
- Only finite Pointer Events pressure from a pen is normalized and clamped. Mouse, touch, absent values, and malformed values use unchanged fixed diameter/opacity behavior.
- Coalesced pen events may be sampled, and a zero-pressure pen-up artifact is not appended after a valid stroke.
- Every recorded sample contains resolved, finite, clamped, quantized diameter and opacity values. Deterministic processing and replay do not query hardware pressure.

### Refine Selection

- A dedicated modal holds an immutable before mask and debounced preview state. Parameter changes do not mutate the active selection.
- The dialog provides split and before/after toggle comparisons, with original-image, black, white, and mask-only backgrounds.
- Smooth, feather, contrast, and shift-edge controls support Reset, Apply, Cancel, Escape, and safe Enter confirmation.
- Apply commits one logical selection history entry; Cancel discards the preview exactly; repeated previews do not create Undo entries.
- Decontaminate Colors is **not shipped**. The existing refine algorithm modifies mask coverage only. Adding an edge-color rewrite without a separately validated image-edit contract would be unsafe and misleading, so no placeholder checkbox is exposed.

### Sessions, workflows, and interchange

- Selection-session schema 2 stores current-stage dimensions, canonical geometry operations, and a geometry fingerprint. It validates active/named masks against the stage and caps named masks at 100.
- The current session key hashes the normalized source path together with the original image dimensions. It stores dimensions and the hash, never the plaintext path, and distinguishes same-name, same-size files in different folders. The older filename-and-dimensions key is read only as a compatibility fallback and is rekeyed in memory without an automatic write.
- A valid Phase 7 schema-1 selection session is migrated in memory with empty geometry and conservative pressure defaults. Only schema 2 is written. Unsupported or incoherent future state fails closed.
- Workflow envelope schema remains version 1. Phase 6 global workflows and Phase 7 masked workflows require no file rewrite. Persistent embedded snapshots are remapped when their pipeline stage changes.
- Standalone JSON and grayscale PNG mask formats remain version 1 and load without rewrite. A newly exported remapped mask records its current dimensions in the same format.

### UX hardening

- Selection controls that mutate mask state are disabled while an operation is busy; cancelling state remains visible until acknowledged.
- Hidden-overlay and global-scope states expose warnings rather than silently suggesting a selective edit is active.
- Selection diagnostics are guarded by mask checksum, semantic no-op history commits are suppressed, and restored sessions establish a clean Undo baseline.
- Selection coordinates use the current transformed canvas dimensions, and selection input is disabled only while a comparison is actually active.

## Automated verification

| Check | Final result |
| --- | --- |
| Rust tests | 444 passed, 0 failed |
| Frontend Vitest | 369 passed, 0 failed across 31 files |
| `cargo fmt --check` | Passed |
| `cargo clippy --all-targets --all-features -- -D warnings` | Passed |
| `npm run check` | 0 errors, 0 warnings |
| `npm run build` | Succeeded; 165 modules transformed |
| `npm run tauri build` | Passed; generated the release executable plus NSIS and MSI bundles |

The final report must record the complete-suite totals, not only focused tests. Relevant coverage includes exact and interpolated geometry remaps, invalid perspective rejection, pressure resolution/replay, thumbnail cache invalidation, progress monotonicity/cancellation, refine preview isolation/apply/cancel/reset, strict preview-stage masks, session migration, embedded workflow masks, and stress/pathological input handling.

## Release-mode performance

Measured on 2026-08-10 with generated fixtures on Windows 11 Pro build 26200, Intel Core i7-12850HX (16 cores/24 logical processors), and 127.7 GiB physical RAM. Command: `cargo run --release --example mask_benchmark`.

Each timing is a one-machine regression observation, not a cross-machine guarantee. Values must include all three required sizes and may not be cherry-picked.

| Operation | 1920×1080 | 4000×3000 | 6000×4000 |
| --- | ---: | ---: | ---: |
| Rectangle rasterization | 3.289 ms | 19.360 ms | 37.598 ms |
| Freehand, 180 points | 2.623 ms | 13.908 ms | 27.129 ms |
| Polygon, 500 points | 3.201 ms | 15.191 ms | 28.873 ms |
| Full-image Magic Wand | 124.477 ms | 812.401 ms | 1,690.994 ms |
| Feather radius 5 | 12.763 ms | 71.993 ms | 155.671 ms |
| Feather radius 25 | 12.521 ms | 72.645 ms | 157.867 ms |
| Expand radius 24 | 17.924 ms | 120.891 ms | 265.158 ms |
| Contract radius 24 | 17.992 ms | 127.622 ms | 258.331 ms |
| Refine Selection | 51.543 ms | 321.048 ms | 695.423 ms |
| Thumbnail generation | 2.859 ms | 17.004 ms | 32.906 ms |
| Crop remap | 0.642 ms | 3.833 ms | 7.550 ms |
| 90° rotation remap | 2.613 ms | 19.699 ms | 49.808 ms |
| Straighten remap | 44.776 ms | 258.530 ms | 514.178 ms |
| Perspective remap | 41.315 ms | 236.461 ms | 471.538 ms |

| Additional operation | Final result |
| --- | ---: |
| JSON mask save/load | 26.811 / 50.325 ms at 6000×4000 |
| PNG mask save/load | 6.241 / 22.378 ms at 6000×4000 |
| Masked export | Masked brightness 172.342 ms and PNG encode 18.663 ms at 4000×3000 |
| Cancellation acknowledgement | 0.936 ms; operation cancelled |
| Owned-buffer/memory observations | 24,000,000-byte raw mask plus 96,000,000-byte RGBA fixture at 6000×4000; no measured peak-process-memory claim |
| Equivalent Phase 7 comparison | 24 MP rectangle 45.367 vs 46.234 ms (-1.9%); 1080p polygon legacy sample 3.502 vs 3.405 ms (+2.8%); 12 MP freehand 13.199 vs 13.154 ms (+0.3%); expand/contract 122.886/122.726 vs 115.462/116.348 ms (+6.4%/+5.5%); Magic Wand 1,680.355 vs 1,633.557 ms (+2.9%); JSON save/load 26.811/50.325 vs 26.917/51.647 ms (-0.4%/-2.6%); masked brightness 172.342 vs 167.739 ms (+2.7%); cancellation 0.936 vs 0.994 ms (-5.8%) |

## Packaged manual validation

The following groups cover the requested 73-item matrix. Automated/backend evidence and hands-on packaged actions are reported separately; a unit test is not treated as a GUI pass.

| Matrix group | Items | Final evidence/status |
| --- | --- | --- |
| Selection creation/composition | Rectangle, ellipse, freehand, polygon, brush add/erase, pen pressure, Magic Wand, Color Range, Replace/Add/Subtract/Intersect | Automated Rust/frontend coverage passed for the listed tools, modes, pressure fallback, and bounded malformed inputs. The packaged flow exercised Select All and Magic Wand on a 24 MP image; it did not individually re-run every listed creation tool and composition mode by hand. |
| Mask operations/refinement | Feather, expand, contract, invert, smooth, fill holes, islands, border, Refine, before/after comparison | Automated coverage passed for all listed mask operations. The packaged flow opened Refine, changed comparison/background views, applied the preview, and retained one active mask; the other operations were not each re-run by hand. |
| Named masks/interchange | Create, rename, duplicate, delete, visibility, lock, reorder, load, save active, JSON/PNG import/export, thumbnails | Automated coverage passed for bounded state, all named-mask actions, lazy thumbnails, and JSON/PNG round trips. The packaged flow saved the active selection as `Mask 1` and rendered its named-mask row; GUI file-dialog automation for packaged import/export was not completed. |
| Selective edits | Exposure, curves, HSL, temperature/tint, denoise, sharpen, local contrast, outside-mask | Automated pipeline and workflow coverage passed. The packaged flow applied Brightness at 14% inside a transformed mask; the remaining edit kinds and outside-mask scope were not each re-run by hand. |
| History/workflows/sessions | Undo, redo, record/replay, missing-mask fail closed, session save/restore, Phase 1–6 regression | Automated migration, workflow validation/replay, fail-closed, history-retention, and compound geometry Undo/Redo coverage passed. Packaged Undo and Redo were exercised after rotation; workflow/session cases were not each re-run by hand. |
| Fixtures/output integrity | Transparent PNG, single-color, noisy photo, document-like image, source unchanged, output decodes | Automated decode, bounds, round-trip, and export coverage passed. Packaged runs used a local 6000×4000 JPEG and a 6000×4000 single-color PNG and verified the input hash was unchanged. Transparent/noisy/document-like fixtures and packaged GUI export were not each completed by hand. |
| Geometry | Crop, rotate, straighten, perspective, mask alignment after transform | Automated exact/interpolated remap, invalid-perspective, stage, and compound-history coverage passed. The packaged flow exercised 90° rotation, mask remap, masked adjustment, Undo, and Redo; crop, straighten, and perspective were not each re-run by hand. |
| Async behavior | Large-image cancellation and numerical progress | A packaged 24 MP Magic Wand run displayed real numerical progress and a Cancel control; cancellation returned the UI to idle in 20.354 ms. The release benchmark measured backend cancellation acknowledgement at 0.936 ms. |
| Display/window | High-DPI mapping; 100%, 125%, 150%, 200%; narrow and maximized | Coordinate and transformed-canvas tests passed. Hands-on package checks used the current 100% Windows scale and a standard window only; 125%, 150%, 200%, narrow, and maximized manual passes were not completed. |
| Package forms | Portable, NSIS-installed, MSI-installed | The exact portable started responsively. The exact NSIS package completed install, launch, packaged selection/refine/geometry work, and uninstall. The MSI completed metadata audit, administrative extraction, and extracted-executable launch; an actual all-users MSI install was not completed because elevation was unavailable. |

Deliberately retained evidence is under the ignored `release/validation` directory: `nsis-startup.png`, `nsis-landscape-open.png`, `nsis-selection-refine-rotate.png`, `packaged-wand-progress.png`, the generated 24 MP solid fixture, the packaged-flow/network scripts, and installer logs. These files are local release evidence and are not committed.

## Installer lifecycle

### MSI

| Step | Final observation |
| --- | --- |
| Product/version metadata | `PhotoForge` 0.7.1; `ProductCode={46AE88A5-3A91-40B2-AE20-851A8AE42355}`; stable `UpgradeCode={DA34C5F7-E5BB-583B-93F8-1F4E4065DC14}`; `ALLUSERS=1`; manufacturer `photoforge`. |
| Install and installed files | A direct non-elevated install of the exact final MSI failed with Windows Installer error 1925/status 1603 and rolled back cleanly. An elevation request could not be completed in the unattended environment. A non-installing administrative extraction of the final MSI succeeded with exit 0 and produced the expected 0.7.1 executable. |
| Launch and basic image operation | The administratively extracted executable reached a responsive `PhotoForge` window and closed cleanly. A basic operation through an actually installed MSI was not completed because the all-users install requires elevation. |
| Start Menu/shortcut behavior | Not tested: the all-users MSI was not installed. Administrative extraction created no install registration. |
| Uninstall and residue check | Not applicable to an installed product because installation never completed. The failed direct attempt rolled back with no PhotoForge install record/files, and administrative extraction did not register a product. An elevated MSI uninstall was not tested. |

### NSIS

| Step | Final observation |
| --- | --- |
| Install and registration | Silent per-user installation returned 0; installed `photoforge.exe` and `uninstall.exe`, created the user Start Menu shortcut and HKCU uninstall record, and reported version 0.7.1. |
| Launch and basic image operation | The exact final installed executable reached a responsive `PhotoForge` window. The packaged flow opened a local image, created/saved/refined a selection, rotated it, used Undo/Redo, and applied Brightness inside the remapped mask while leaving the source hash unchanged. |
| Uninstall and residue check | Silent uninstall returned 0 and removed the executable, uninstaller, Start Menu shortcut, and HKCU uninstall record. The pre-existing WebView2 cache remained and was not misclassified as installer residue. |

Pre-existing WebView2 user data must be distinguished from files introduced by the lifecycle under test; it should not be described as installer residue without evidence.

## WebView2 network observation

Configured launch arguments and supported-control review: the packaged window retains only `--disable-background-networking` in `additionalBrowserArgs`. Undocumented internal feature switches for browser UI or SmartScreen are not used. Microsoft documents `AdditionalBrowserArguments` as a browser-argument pass-through whose important switches may be ignored, and documents `IsReputationCheckingRequired` as the supported WebView2 SmartScreen control; PhotoForge leaves reputation checking at the runtime default and makes no firewall or hosts-file change. See [AdditionalBrowserArguments](https://learn.microsoft.com/en-us/dotnet/api/microsoft.web.webview2.core.corewebview2environmentoptions.additionalbrowserarguments?view=webview2-dotnet-1.0.3800.47) and [IsReputationCheckingRequired](https://learn.microsoft.com/en-us/microsoft-edge/webview2/reference/win32/icorewebview2settings8?view=webview2-1.0.3650.58).

Observation window, method, and operation: the exact final portable executable was launched idle at 2026-08-10T15:18:18.4764531Z for 15.286 seconds. A local PowerShell observer sampled non-loopback `Get-NetTCPConnection` ownership for the root PID and discovered descendants at 100 ms intervals; 27 connection samples were retained. No debug browser argument was present, and the WebView2 root command line contained `--disable-background-networking`.

| Process class | Source/destination/timing | Classification |
| --- | --- | --- |
| PhotoForge Rust application process | PID 8760 owned no sampled non-loopback TCP socket. | No PhotoForge application socket was observed. |
| Microsoft Edge WebView2 subprocesses | WebView2 root PID 28900 held two established IPv6 TLS connections from ephemeral ports 49832/49833 to `[2603:1046:c0b:819::2]:443`, first seen at 1,090.9/1,084.3 ms and last seen at 15,177.7/15,177.6 ms, with 27 samples each. The local public address is intentionally omitted from the committed record. | Runtime traffic owned by WebView2; it is not attributed to PhotoForge mask code. |
| Windows/system processes | The scoped observer captured only PhotoForge and discovered descendants; unrelated system sockets were excluded. | No Windows-wide traffic claim is made. |

Conclusion: no non-loopback socket was owned by the PhotoForge Rust process, but the embedded WebView2 runtime did establish two TLS connections during idle startup despite `--disable-background-networking`. Therefore 0.7.1 makes no zero-network claim for the complete WebView2 process tree.

PhotoForge's selection, thumbnail, remap, progress, pressure, and refinement code contains no networking path. This is not a zero-connection guarantee for the complete process tree: WebView2 required diagnostics/configuration traffic is governed partly by Windows settings and is not fully controlled by the embedding application. Unsupported flags, firewall changes, and hosts-file workarounds are not used.

## Release artifacts

Release files belong in the repository's ignored `release` directory and are not committed.

| Artifact | Size | SHA-256 |
| --- | ---: | --- |
| `PhotoForge-portable.exe` | 14,309,888 bytes | `11afe3108efd26e599bd0c24f539c5ef03bcedf7431fad22d8e7a57da5575923` |
| `PhotoForge_0.7.1_x64-setup.exe` | 3,260,739 bytes | `a23f887d37e9138ca5dc226a1fbec74ee2f2c019b635e02e189956c51e580987` |
| `PhotoForge_0.7.1_x64_en-US.msi` | 4,759,552 bytes | `23b9c953a11ea33280a555d525037d548f0bc97f3d299dbb857acbab2e7b5233` |
| `SHA256SUMS.txt` | 284 bytes | `89f313699184d02a4e9edf37bbb92fd0ed8472c6dce2c0a9cfcf79aded1d55a9` |

Artifact audit: the three release binaries match their Tauri build outputs byte for byte, and `certutil` independently reproduced every manifest hash. The manifest contains exactly the three expected entries with no malformed, duplicate, missing, or extra entry. Portable and NSIS resources report `PhotoForge` 0.7.1; MSI properties match the metadata above. All three binaries are Authenticode `NotSigned`; no signing certificate or secret was supplied.

## Remaining limitations

- Decontaminate Colors is not shipped; refinement changes mask coverage only.
- Lens-correction masks are not geometrically remapped in 0.7.1.
- JSON/PNG mask file I/O has bounded validation and atomic JSON save behavior but no granular numerical progress.
- Selection sessions are bounded WebView local state, not a general project-file system. Document association uses a hash of the normalized source path plus original image dimensions; no plaintext path or source-pixel/content hash is stored. Moving or renaming a file changes its current identity, while replacing it in place with different pixels at the same dimensions does not.
- Manual gaps remain explicit above: the full 73-item matrix was not repeated hands-on across every tool, scale, window mode, fixture, and package form; actual elevated MSI install/uninstall and packaged GUI export were not completed.

## Git publication

- Implementation commit: `0b98a53510853523be5e84525b0df32dd26d31da` (`Complete and harden Phase 7 selections`).
- Publication record: this correction is committed with the requested release title `Complete and harden Phase 7 selections and masking`; its immutable hash is reported in the post-push release handoff rather than embedded self-referentially here.
- Publication target: `main` on `origin`. The follow-up preserves the already-published implementation commit instead of rewriting shared history; final local/remote hash equality is recorded in the release handoff.
