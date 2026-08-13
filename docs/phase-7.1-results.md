# Phase 7.1 results

PhotoForge 0.7.1 is the completion and hardening release for Phase 7. It does not begin Phase 8. PhotoForge application code keeps selection and image processing deterministic and local: it adds no cloud service, application telemetry, model download, neural segmentation, generative editing, OCR, account, payment, marketplace, or executable-plugin behavior. Independent WebView2 runtime diagnostics are disclosed separately below.

This document separates implementation facts from measured release observations and states where hands-on coverage was incomplete.

## Completed implementation

### Geometry and stage alignment

- Crop, exact 90°/180°/270° rotation, horizontal reflection, arbitrary straighten, and perspective are represented as checked geometry chains with dimensions recorded at every stage.
- A geometry edit remaps the active mask, named masks, and persistent embedded workflow-mask snapshots in one keyed, bounded transaction. The edit pipeline and selection state are committed together only after every expected result is present and valid.
- Crop and exact rotation/reflection use discrete coverage transforms when they are an applicable suffix. General old-chain to new-chain rebases use destination-to-source bilinear coverage sampling once, clamp output coverage, and use zero outside the source domain.
- Perspective inversion is bounded and rejects non-finite, folded, near-singular, or non-convergent transforms. Stage mismatches, stale documents, cancellation, duplicate keys, missing keys, and oversized batches fail closed.
- Full-resolution masked operations require exact mask dimensions at their pipeline stage. Preview preparation tracks full and preview stages independently and creates temporary bilinear preview masks without rewriting canonical snapshots.
- Undo/Redo treats a completed geometry remap as one compound edit/selection action.
- Lens correction participates in the same geometry contract. Scalar coverage follows bounded distortion in the validated `-0.16…1` range; vignetting and red/blue chromatic-aberration offsets are intentionally non-spatial for a one-channel mask. Identity and non-spatial-only lens changes preserve coverage exactly.
- Rapid geometry sliders retain only the final same-control intent, including a return to the committed baseline while a remap is active. Unrelated mutations are rejected until the transaction settles, and cancellation removes queued follow-up geometry.

### Thumbnails and overlays

- Named-mask rows render actual grayscale coverage previews with area averaging and aspect-fit dimensions.
- Rendering is lazy through intersection observation and idle scheduling. Cache identity includes checksum and source/target dimensions, so mask edits and remaps invalidate only affected entries.
- The shared thumbnail LRU is bounded to 96 entries and approximately 2 MiB. It performs no disk writes or network work.
- The image stage can composite the active mask and visible named masks for review while retaining each mask independently.

### Numerical progress and cancellation

- Request-scoped progress records document ID, request ID, operation, phase, completed units, total units, and queued/running/cancelling/completed/cancelled/failed state.
- Rasterization/composition, Magic Wand, Color Range, feather/morphology/cleanup/refinement, and geometry remapping report implementation work units such as pixels, passes, or output rows.
- The frontend ignores stale progress, prevents percentage regression, bounds determinate output to 0–100%, waits 180 ms before display, and clears terminal state. Cancellation remains `cancelling` until worker acknowledgement.
- JSON and PNG mask import/export use the same document/request-scoped lifecycle. They report real file-byte, output-row, decoded-coverage, checksum, and diagnostic-scan units when totals are known; parser and codec-only phases remain indeterminate. Writes use collision-resistant sibling temporary files and preserve the existing destination on cancellation or failure before replacement.

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
- Decontaminate Colors is an optional, default-off masked image operation. It deterministically replaces RGB spill only on partially selected edge pixels from bounded nearby confidently selected, non-transparent foreground samples, preserves alpha, honors inside/outside inversion, and has explicit radius/work limits.
- The dialog shows a bounded representative local preview using the same circular distance-weighted sampling semantics. Missing source imagery, malformed coverage, or a preview work-limit failure disables Apply and safe Enter rather than committing an unseen effect.

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
| Rust tests | 481 passed, 0 failed |
| Frontend Vitest | 415 passed, 0 failed across 34 files |
| `cargo fmt --all -- --check` | Passed |
| `cargo clippy --all-targets --all-features -- -D warnings` | Passed |
| `npm run check` | 0 errors, 0 warnings |
| `npm run build` | Succeeded; 167 modules transformed |
| `npm run tauri build` | Passed; generated the release executable plus NSIS and MSI bundles |
| `npm audit` / `npm audit --omit=dev` | 0 vulnerabilities in both full and production-only dependency graphs |
| `cargo audit` | 0 vulnerability findings; 17 allowed maintenance/unsoundness warnings remain in transitive Tauri platform dependencies (16 unmaintained, one `glib` unsoundness advisory) |

The complete-suite totals above include exact and interpolated geometry remaps, invalid perspective rejection, pressure resolution/replay, thumbnail cache invalidation, progress monotonicity/cancellation, refine preview isolation/apply/cancel/reset, strict preview-stage masks, session migration, embedded workflow masks, and stress/pathological input handling.

## Release-mode performance

Measured on 2026-08-11 with generated fixtures on Windows 11 Pro build 26200, Intel Core i7-12850HX (16 cores/24 logical processors), and 127.7 GiB physical RAM. Root-safe command: `cargo run --manifest-path src-tauri/Cargo.toml --release --example mask_benchmark`.

Each timing is a one-machine regression observation, not a cross-machine guarantee. Values must include all three required sizes and may not be cherry-picked.

| Operation | 1920×1080 | 4000×3000 | 6000×4000 |
| --- | ---: | ---: | ---: |
| Rectangle rasterization | 3.395 ms | 18.861 ms | 38.269 ms |
| Freehand, 180 points | 2.577 ms | 13.581 ms | 26.251 ms |
| Polygon, 500 points | 3.157 ms | 15.687 ms | 28.468 ms |
| Full-image Magic Wand | 118.337 ms | 790.365 ms | 1,638.854 ms |
| Feather radius 5 | 11.670 ms | 70.856 ms | 149.233 ms |
| Feather radius 25 | 11.707 ms | 70.526 ms | 148.394 ms |
| Expand radius 24 | 18.191 ms | 121.997 ms | 270.805 ms |
| Contract radius 24 | 17.876 ms | 120.246 ms | 253.437 ms |
| Refine Selection | 50.539 ms | 316.687 ms | 661.937 ms |
| Thumbnail generation | 2.886 ms | 17.992 ms | 33.522 ms |
| Crop remap | 0.604 ms | 4.400 ms | 8.105 ms |
| 90° rotation remap | 2.550 ms | 19.486 ms | 51.324 ms |
| Straighten remap | 43.677 ms | 269.532 ms | 521.603 ms |
| Perspective remap | 41.422 ms | 237.950 ms | 472.749 ms |
| Lens-distortion remap | 40.862 ms | 239.418 ms | 471.302 ms |

| Additional operation | Final result |
| --- | ---: |
| JSON mask save/load | 27.319 / 45.245 ms at 6000×4000 |
| PNG mask save/load | 33.681 / 20.476 ms at 6000×4000 |
| Masked image processing/export | Masked brightness 170.227 ms, Decontaminate Colors 54.372 ms, and PNG encode 18.805 ms at 4000×3000 |
| Cancellation acknowledgement | 0.930 ms; operation cancelled |
| Owned-buffer/memory observations | 24,000,000-byte raw mask plus 96,000,000-byte RGBA fixture at 6000×4000; no measured peak-process-memory claim |
| Equivalent Phase 7 comparison | 24 MP rectangle 45.627 vs 46.234 ms (-1.3%); 1080p polygon legacy sample 3.146 vs 3.405 ms (-7.6%); 12 MP freehand 13.400 vs 13.154 ms (+1.9%); expand/contract 120.783/121.391 vs 115.462/116.348 ms (+4.6%/+4.3%); Magic Wand 1,631.142 vs 1,633.557 ms (-0.1%); JSON save/load 27.319/45.245 vs 26.917/51.647 ms (+1.5%/-12.4%); masked brightness 170.227 vs 167.739 ms (+1.5%); cancellation 0.930 vs 0.994 ms (-6.4%) |

## Packaged manual validation

The requested 73-item matrix is retained as a local validation plan. It was not completed against the final binaries, and automated/backend evidence is not relabeled as a GUI pass. Two supported Computer Use checks reported the integration as `not_installed`; the current bundled-runtime import also failed with a protected-path `EPERM`, so no supported PhotoForge window existed. An earlier temporary copy of the official runtime package could import, but app approval remained unavailable and it still exposed no targetable window. The older PowerShell/UI Automation matrix runs were debugging runs against interim binaries and are deliberately excluded from final acceptance.

| Matrix group | Items | Final evidence/status |
| --- | --- | --- |
| Selection creation/composition | Rectangle, ellipse, freehand, polygon, brush add/erase, pen pressure, Magic Wand, Color Range, Replace/Add/Subtract/Intersect | Automated Rust/frontend coverage passed for the listed tools, modes, pressure fallback, cancellation, and bounded malformed inputs. These individual interactions were not all repeated through the final packaged GUI. |
| Mask operations/refinement | Feather, expand, contract, invert, smooth, fill holes, islands, border, Refine, before/after comparison | Automated coverage passed for the listed operations, preview isolation, representative Decontaminate Colors preview, work limits, and one-entry Apply/zero-entry Cancel. The final packaged GUI matrix was not completed. |
| Named masks/interchange | Create, rename, duplicate, delete, visibility, lock, reorder, load, save active, JSON/PNG import/export, thumbnails | Automated coverage passed for bounded state, named-mask actions, lazy thumbnails, secure JSON/PNG round trips, numerical progress, and cancellation. Native packaged file-dialog round trips were not accepted as final evidence. |
| Selective edits | Exposure, curves, HSL, temperature/tint, denoise, sharpen, local contrast, outside-mask | Automated processor/workflow coverage passed, including strict stage-aligned masks and masked-only Decontaminate Colors. The edit kinds were not each repeated through the final packaged GUI. |
| History/workflows/sessions | Undo, redo, record/replay, missing-mask fail closed, session save/restore, Phase 1–6 regression | Automated migration, deep workflow validation/replay, fail-closed behavior, history retention, rapid-slider coalescing, cancellation, and compound geometry Undo/Redo coverage passed. The final packaged workflow/session matrix was not completed. |
| Fixtures/output integrity | Transparent PNG, single-color, noisy photo, document-like image, source unchanged, output decodes | Automated decode, bounds, round-trip, source-preservation, and export coverage passed. The fixture set was generated locally, but the final GUI fixture/export sequence was not completed. |
| Geometry | Crop, rotate, straighten, perspective, reflection, lens distortion, mask alignment after transform | Automated exact/interpolated remap, invalid-transform rejection, stage, embedded-mask, cancellation, coalescing, and compound-history coverage passed. The exact final packaged GUI did not repeat every geometry case. |
| Async behavior | Large-image cancellation and numerical progress | Automated request/currentness, monotonic-progress, JSON/PNG I/O, operation cancellation, and stale-result coverage passed. The release benchmark measured cancellation acknowledgement at 0.930 ms. The final packaged 24 MP progress/cancel sequence was not completed. |
| Display/window | High-DPI mapping; 100%, 125%, 150%, 200%; narrow and maximized | Coordinate and transformed-canvas tests passed. The exact portable and installed NSIS executable opened responsive standard windows at the current 100% Windows scale; 125%, 150%, 200%, narrow, and maximized passes were not completed. |
| Package forms | Portable, NSIS-installed, MSI-installed | The exact portable started responsively. The exact NSIS completed install, responsive launch, and clean uninstall. The MSI completed metadata audit, administrative extraction, and responsive extracted-executable launch; the requested elevated all-users installation was cancelled at the UAC boundary, so an installed-MSI lifecycle was not completed. |

Deliberately retained evidence is under the ignored `release/validation` directory: `artifact-origin-bootstrap-final-20260813.txt`, `nsis-origin-bootstrap-final-20260813.txt`, `network-origin-bootstrap-final-20260813/attributed-network.txt`, `msi-origin-bootstrap-final-admin-20260813/`, `phase71-origin-bootstrap-final-cua-blocker-20260813.txt`, the benchmark output, generated fixtures, observer/matrix scripts, and interim debugging runs. The interim GUI matrix directories are treated here as debugging evidence and are not final acceptance. These local files are not committed.

## Installer lifecycle

### MSI

| Step | Final observation |
| --- | --- |
| Product/version metadata | `PhotoForge` 0.7.1; `ProductCode={22FFC7C3-874B-4FBF-AE8D-C3B9F6C4E0A5}`; stable `UpgradeCode={DA34C5F7-E5BB-583B-93F8-1F4E4065DC14}`; `ALLUSERS=1`; `MSIINSTALLPERUSER` absent; manufacturer `photoforge`. |
| Install and installed files | The origin-corrected final MSI's elevation request was explicitly presented but cancelled at the UAC consent boundary. An earlier post-hardening MSI request had also been cancelled; UAC was not bypassed or automated, so no product installation or registration occurred. A non-installing administrative extraction of the final MSI succeeded with exit 0 and produced the expected 0.7.1 executable. |
| Launch and basic image operation | The administratively extracted executable reached a responsive `PhotoForge` window and closed cleanly. A basic operation through an actually installed MSI was not completed because the all-users install requires elevation. |
| Start Menu/shortcut behavior | Not tested: the all-users MSI was not installed. Administrative extraction created no install registration. |
| Uninstall and residue check | Not applicable to an installed product because installation never completed. Administrative extraction did not register a product, create installed-product shortcuts, or require uninstall. An elevated MSI uninstall was not tested. |

### NSIS

| Step | Final observation |
| --- | --- |
| Install and registration | Silent per-user installation returned 0; installed `photoforge.exe` and `uninstall.exe`, created the user Start Menu shortcut and HKCU uninstall record, and reported version 0.7.1. |
| Launch and basic image operation | The exact final installed executable reached a responsive `PhotoForge` window with a nonzero window handle and closed cleanly. A final image-editing matrix could not be driven through the packaged GUI, so no packaged operation pass is claimed. |
| Uninstall and residue check | Silent uninstall returned 0 and removed the executable, uninstaller, Start Menu shortcut, and HKCU uninstall record. The pre-existing WebView2 cache remained and was not misclassified as installer residue. |

Pre-existing WebView2 user data must be distinguished from files introduced by the lifecycle under test; it should not be described as installer residue without evidence.

## WebView2 network observation

Supported-control review: the packaged window starts on a script-free inert bundled document at the normal Tauri origin, preserving Tauri's per-webview origin metadata. It applies and reads back WebView2's documented `IsReputationCheckingRequired=false` and only then navigates to the application document. Top-level external navigation, popups, browser downloads, frames, objects, form submissions, and ordinary renderer fetch/XHR/WebSocket/EventSource destinations outside Tauri IPC are denied. PhotoForge supplies no custom browser argument, firewall rule, hosts-file entry, or undocumented feature switch. These controls are not a process firewall. See [the application/runtime boundary](webview-network-boundary.md), [Microsoft's WebView2 privacy documentation](https://learn.microsoft.com/en-us/microsoft-edge/webview2/concepts/data-privacy), and [`ICoreWebView2Settings8`](https://learn.microsoft.com/en-us/microsoft-edge/webview2/reference/win32/icorewebview2settings8).

Observation window, method, and operation: the origin-corrected final portable executable was launched idle at 2026-08-13T14:25:57.0619208Z for 25.187 seconds. A local PowerShell observer sampled non-loopback `Get-NetTCPConnection` ownership for root PID 23468, profile-bound WebView2 descendants, and system processes separately. No profile-bound WebView2 process existed before launch, and no debug/custom browser argument was present.

| Process class | Source/destination/timing | Classification |
| --- | --- | --- |
| PhotoForge Rust application process | Root PID 23468 owned no sampled non-loopback TCP socket. | No PhotoForge application socket was observed. |
| Microsoft Edge WebView2 subprocesses | Profile-bound WebView2 browser-host PID 21696 held two established IPv6 TLS connections from ephemeral ports 55101/55102 to `[2603:1046:c0b:4d::2]:443`, first seen at 1,899.6/1,899.4 ms and last seen at 25,062.6/25,062.5 ms, with 55 samples each. The local public address is intentionally omitted from the committed record. | Runtime traffic owned by WebView2; it is not attributed to PhotoForge application code. |
| Windows/system processes | The observer recorded 19 system/unattributed connections separately; these were not owned by the PhotoForge/WebView2 process tree. | No Windows-wide traffic is attributed to PhotoForge. |

Conclusion: no non-loopback socket was owned by the PhotoForge Rust process, but the embedded WebView2 runtime established two TLS connections during idle startup despite the supported application boundary. Microsoft documents that required WebView2 diagnostic collection is not fully controlled by the embedding application. Therefore 0.7.1 makes no zero-network claim for the complete WebView2 process tree.

PhotoForge's selection, thumbnail, remap, progress, pressure, and refinement code contains no networking path. This is not a zero-connection guarantee for the complete process tree: WebView2 required diagnostics/configuration traffic is governed partly by Windows settings and is not fully controlled by the embedding application. Unsupported flags, firewall changes, and hosts-file workarounds are not used.

## Release artifacts

Release files belong in the repository's ignored `release` directory and are not committed.

| Artifact | Size | SHA-256 |
| --- | ---: | --- |
| `PhotoForge-portable.exe` | 14,744,064 bytes | `bd88cde252a1277baca7d8283af6b7ae937a002725c199a2d62936ee7076b0d2` |
| `PhotoForge_0.7.1_x64-setup.exe` | 3,349,896 bytes | `0fe662b6cafcc4409b9cb11b15244cabf80593df9c8f8871024be957198f8c5e` |
| `PhotoForge_0.7.1_x64_en-US.msi` | 4,956,160 bytes | `0aeb1d923516cc1b8e2ea96271893635b5471c6d92199c3b074d7468fc2d48ca` |
| `SHA256SUMS.txt` | 284 bytes | `51e91aa17e934f5ad35ff5a7998334f9999306a358610f080e6d7e9d832d4a7f` |

Artifact audit: the three release binaries match their Tauri build outputs byte for byte, and independent hashing reproduced every manifest hash. The manifest contains exactly the three expected current-0.7.1 entries with no malformed, duplicate, missing, or extra entry. Portable and NSIS resources report `PhotoForge` 0.7.1; MSI properties match the metadata above. `Get-AuthenticodeSignature` and Windows SDK SignTool independently report no signature on all three binaries; no trusted code-signing identity or service was supplied or discovered, and no self-signed substitute was used.

## Remaining limitations

- Selection sessions are bounded WebView local state, not a general project-file system. Document association uses a hash of the normalized source path plus original image dimensions; no plaintext path or source-pixel/content hash is stored. Moving or renaming a file changes its current identity, while replacing it in place with different pixels at the same dimensions does not.
- Manual gaps remain explicit above: the final 73-item GUI matrix was not completed; native 125%, 150%, and 200% display-scale passes were not run; and actual elevated MSI install/launch/uninstall was not completed because UAC consent was cancelled.
- The release binaries are unsigned because no CA-issued Code Signing identity with its private key, hardware-backed identity, or configured trusted signing service was supplied or discovered. A self-signed certificate was not substituted for production signing.

## Git publication

- Integrated Phase 7.1 implementation commit: `e8f5d7469cbc3e904c9e07d3c117cdd3fcca6c7f` (`Complete and harden Phase 7 selections`).
- Publication target: `main` on `origin`. This release-record correction preserves published history; its immutable hash and final local/origin/live-remote equality are reported in the post-push handoff rather than embedded self-referentially here.
