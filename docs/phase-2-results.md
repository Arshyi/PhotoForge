# PhotoForge Phase 2 results

Date: July 18, 2026

Release: 0.2.0

Baseline: 0.1.1 at `cbe74bd`

PhotoForge Phase 2 uses deterministic image processing. It does not generate missing image content or reconstruct factual details that were never captured.

## Delivered scope

Phase 2 adds eight validated, serializable operations to the existing ordered edit pipeline:

| Operation | Deterministic implementation |
| --- | --- |
| Auto White Balance | Transparent-aware 5%-trimmed gray-world statistics, luminance-normalized/clamped gains, strength blending |
| Local Contrast | Clipped local-luminance normalization using a linear-time sliding box estimate; documented as a conservative CLAHE alternative |
| Denoise | Bounded 3×3/5×5 edge-aware spatial weighting |
| JPEG Cleanup | Guarded smoothing across detected 8×8 boundary discontinuities |
| Edge-Aware Sharpen | Thresholded luminance high-pass correction with bounded radius and halo cap |
| Mild Deblur | Two conservative edge-aware high-pass passes, accurately labeled mild clarity restoration rather than deconvolution |
| Uneven Lighting | Bounded additive normalization against a low-frequency luminance estimate |
| Document Enhance | Fixed white-balance, lighting, local-contrast, denoise, sharpen, and optional grayscale sequence |

All operations reject non-finite/out-of-range parameters, bound kernels/regions, preserve source alpha, treat strength zero as identity, and safely handle tiny images. Preview and export execute the same operation implementations and ordering. Preview uses the cached source copy capped at 1600 pixels; export uses the cached full-resolution source.

The sidebar now has a Restoration section with a primary strength control for each operation, expandable advanced controls, per-operation reset actions, a prominent strong-deblur warning, color/grayscale document modes, and explicit non-generative wording. Eight inspectable presets were added: Fix Indoor Lighting, Improve Old Scan, Clean Up JPEG, Mild Detail Recovery, Enhance Document — Color, Enhance Document — Grayscale, Fix Uneven Lighting, and Conservative Photo Restore.

## Local analysis

The new typed `analyze_image` command calculates average luminance, 5th–95th percentile spread, channel imbalance/color-cast direction, high-frequency noise, Laplacian-style sharpness, local contrast, edge density, white-background ratio, and likely-document status. Analysis runs on a blocking worker, is cached once per document, uses its own serialized gate and request/document generations, and cannot replace a newer document's result. The UI shows at most four cautious observations, exposes details on demand, calls every result a heuristic, and never auto-applies edits.

## Automated verification

All required commands passed against the final Phase 2 source before packaging:

- `cargo fmt --check`
- `cargo clippy --workspace --all-targets -- -D warnings`
- `cargo test --workspace`: 51 passed, 0 failed
- `npm run check`: 0 errors and 0 warnings
- `npm run test`: 21 passed, 0 failed
- `npm run build`: passed; 134 modules transformed
- `npm run tauri build`: passed; portable executable, NSIS, and MSI produced

Rust coverage includes operation behavior, deterministic synthetic noise/blur/block/gradient/document fixtures, black/flat/tiny/transparent images, NaN/infinity and boundary validation, alpha preservation, tagged serialization, pipeline ordering, history/coalescing, analysis heuristics, and preview/full-resolution operation consistency. Frontend coverage includes restoration controls, advanced expansion, warnings, analysis rendering, preset definitions/application, accessibility labels, disabled states, latest-preview coalescing, processing/error UI, and defensive disabled-button behavior.

## Packaged manual validation

The packaged portable executable was copied outside the repository and launched successfully. The following workflows were exercised against generated, license-free fixtures:

- Each of the eight restoration operations individually on a 1920×1080 photograph fixture
- All eight restoration presets; the UI showed each preset's expected operation count
- A four-operation combined Conservative Photo Restore workflow
- Undo, redo, reset, comparison, rotated side-by-side comparison, zoom in/out, and fit
- Ten rapid keyboard change groups on Auto White Balance; the latest preview became current with one operation and no error
- Opening a generated 6000×4000 image, starting maximum denoise, then opening a 4000×3000 image; the newer image won, old work did not replace it, and history reset to zero
- Sequential open of warm-cast, noisy, blurred, heavy-JPEG, uneven-document, and transparent fixtures
- Safe corrupt-input handling; the prior valid document remained active and the window stayed responsive
- Full-resolution PNG, JPEG, and WebP export from a 4000×3000 source
- Keyboard-driven restoration control, rotate, comparison, and zoom controls
- Offline behavior: zero TCP connections during sampled startup, editing, race, and idle checks

The tested window's effective DPI was 192 (200% scaling) and remained accessible and responsive. Separate 125% and 150% sessions were not performed. Drag-and-drop was not re-run in this Phase 2 pass because Phase 2 did not change its path. MSI installation was not performed; the MSI build completed, while the NSIS lifecycle received the install/start/uninstall test.

### Export verification

All three outputs decoded successfully at the source's 4000×3000 dimensions:

| Format | Bytes | SHA-256 |
| --- | ---: | --- |
| PNG | 1,055,297 | `A00D010ECA7423B893264FE894DD1DFD7A19A70A4A8F805450EC51CA14AA0B93` |
| JPEG | 536,885 | `56851984A355DB16B3110BE8CB6C07DEAFE71A427E898EC6E6BB6AFF675B3144` |
| WebP | 732,914 | `CB8BE284411C9032A40538F89C1941C392A0B1FEDE57B01D1811051A619CDB8C` |

Alpha preservation is covered by Rust operation tests and the deterministic preview/export pipeline fixture, plus the packaged transparent-PNG open workflow. JPEG continues to composite transparency onto white by policy.

## Performance and resources

On the tested Windows machine, the 1920×1080 maximum-main-strength preview times were: Auto White Balance 49 ms, Local Contrast 92 ms, Denoise 518 ms, JPEG Cleanup 18 ms, Edge-Aware Sharpen 85 ms, Mild Deblur 156 ms, Uneven Lighting 91 ms, and Document Enhance 484 ms. Conservative Photo Restore took 394 ms. A 4000×3000 lossless WebP export using that preset reported 3,448 ms.

The portable app's first responsive window appeared in about 690 ms. A clean installed start appeared in 803 ms. The installed idle process accumulated 78.1 ms CPU over 15 seconds, used 26.5 MiB working set / 5.2 MiB private memory, and held zero TCP connections. Immediately after the 12 MP restoration/export workflow, the portable process used 118.8 MiB working set / 59.9 MiB private memory. See `performance.md` for artifact and frontend bundle comparisons.

## Privacy and security review

Phase 2 adds no network dependency, telemetry, remote resource, model runtime/download, Python runtime, GPU requirement, arbitrary filesystem command, or new Tauri capability. Analysis and restoration accept only the active decoded image plus validated typed parameters. The existing native open/save dialog allowlist, extension checks, source-overwrite protection, image-size/decode allocation ceilings, serialized work gates, and stale-result checks remain in force. Sampled application processes had zero TCP connections.

## Installer and release artifacts

`PhotoForge_0.2.0_x64-setup.exe` installed silently with exit code 0, registered version 0.2.0, launched responsively, and uninstalled with exit code 0. The uninstall registry entry and install directory were removed. The MSI and portable builds were also produced. Release binaries and their checksum manifest live in `E:\PhotoForge\release` and remain ignored by Git.

## Known limitations and deferred work

- Gray-world balance can misread scenes dominated by one legitimate color and is not calibrated color management.
- Local contrast is a conservative luminance-normalization equivalent, not complete interpolated CLAHE.
- Denoise can soften fine texture; sharpening/clarity restoration can amplify noise or halos at strong values.
- JPEG cleanup reduces selected decoded-pixel artifacts but cannot recover discarded coefficients.
- Mild Deblur is not neural or iterative deconvolution and cannot recover missing detail or substantial motion blur.
- Document Enhance is not OCR, semantic understanding, perspective correction, auto-crop, or background removal.
- Restoration operates on 8-bit encoded channels; ICC/EXIF preservation, linear-light/color-managed processing, tiled full-resolution processing, and atomic destination replacement remain future work.
- Perspective correction, batch processing, guided editing, Ollama/LLM support, ONNX/neural restoration, super-resolution, face restoration, inpainting, generative enhancement, telemetry, cloud services, accounts, updates, plugins, and non-Windows packaging remain deferred.
