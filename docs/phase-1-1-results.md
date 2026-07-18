# PhotoForge Phase 1.1 audit results

Date: July 18, 2026

Repository: `E:\PhotoForge`

Baseline: `ccb8e12`, version 0.1.0

Audited release: version 0.1.1

## Scope and method

This pass audited the existing Phase 1 editor; it did not add Phase 2 restoration, natural-language editing, an LLM, telemetry, cloud services, or a new dependency on an external runtime. Evidence combines source review, focused unit/component tests, generated license-free fixtures, baseline and final packaged-app workflows, installer lifecycle testing, resource observations, and release hashing.

The working tree was clean at the start. It is intentionally left uncommitted and unpushed for review.

## Defects found and fixed

1. **Competing image opens could publish in the wrong order.** Open requests now carry generations, stale results cannot replace the current document, and a new open invalidates preview work immediately.
2. **Rapid preview changes could queue unbounded blocking jobs.** The frontend now permits one preview invoke in flight and keeps only the newest queued state. Rust serializes preview work and drops superseded queued requests before processing.
3. **A result could become stale during the 120 ms debounce window.** The request generation now advances when the UI state changes, not only when rendering starts.
4. **A 90-degree rotated comparison reused the original geometry.** Transformed comparisons use independent, responsive side-by-side figures; same-geometry comparisons retain the swipe divider.
5. **Slider input events produced noisy history.** Events from one rapid same-slider gesture coalesce within 500 ms. Both frontend and Rust histories are capped at 200 snapshots.
6. **Settings and unavailable editor controls needed stronger keyboard behavior.** The dialog traps focus, supports Escape, returns focus to Settings, and makes the background inert. Document-only controls are inert while unavailable.
7. **JPEG alpha and quality were implicit library behavior.** JPEG is explicitly quality 90 with transparency composited onto white. PNG and WebP explicitly preserve RGBA; WebP is lossless.
8. **The 120 MP ceiling allowed impractical memory peaks.** Input is now limited to 40 MP, 20,000 pixels per dimension, 256 MiB decoder allocation, and 750 MiB encoded size.
9. **Exports and empty pipelines performed avoidable full-image work.** Export is serialized; an empty pipeline reuses the shared source rather than cloning it.
10. **Each operation reconverted a dynamic image to RGBA.** Non-empty pipelines now convert once before applying ordered operations.

## Automated verification

All required checks passed against the final 0.1.1 source in `E:\PhotoForge`:

| Command | Outcome |
| --- | --- |
| `cargo fmt --manifest-path src-tauri/Cargo.toml --check` | Passed |
| `cargo clippy --manifest-path src-tauri/Cargo.toml --workspace --all-targets -- -D warnings` | Passed; the USB filesystem caused a non-code incremental-cache hard-link fallback warning |
| `cargo test --manifest-path src-tauri/Cargo.toml --workspace` | 29 passed, 0 failed |
| `npm run check` | 0 errors, 0 warnings |
| `npm run test` | 10 passed, 0 failed |
| `npm run build` | Passed; 129 modules, 88.11 kB production JS (29.76 kB gzip) |
| `npm run tauri build` | Passed; portable EXE, NSIS EXE, and MSI produced |
| `git diff --check` | Passed; only expected Windows line-ending notices were printed |

Because Cargo's incremental cache on the removable filesystem repeatedly exceeded the command time limit and could not hard-link, the final Rust test and release compile used `CARGO_TARGET_DIR` in the local audit workspace while reading the exact manifests and source from `E:\PhotoForge`. Release artifacts were copied back to `E:\PhotoForge\release`.

Focused coverage includes parameter ranges and non-finite values; pixel clamping and operation order; neutral exactness; alpha preservation; grayscale, sepia, saturation, sharpen, reflection, rotation, and transform ordering; undo/redo/reset and history bounds/coalescing; rotated comparison layout; PNG/JPEG/WebP/grayscale loading; Unicode and space paths; read-only input; empty/truncated/renamed-invalid input; dimension and preview bounds; explicit export policy; export dimensions; original overwrite rejection; and unchanged source bytes.

## Fixture and packaged-app workflows

License-free fixtures were generated locally and kept outside the repository at `E:\PhotoForge-audit-fixtures`. They cover PNG, JPEG, WebP, alpha, grayscale, portrait, landscape, 1×1, 24 MP, corrupt/truncated/empty/renamed-invalid, unsupported BMP, dimension-bomb header, spaces, Unicode, long names, and read-only media.

Baseline 0.1.0 observations established the release starting point:

- One responsive window and no release console window.
- Native `Ctrl+O` picker opened and a 640×360 landscape PNG loaded with correct metadata and current preview.
- Visible keyboard focus; brightness maximum; one-step undo/redo; and redo-branch clearing worked.
- At 15 seconds idle: 0.000 CPU seconds, approximately 30.2 MB working set, approximately 5.4 MB private memory, and zero TCP connections.

Final 0.1.1 portable observations were made after copying the EXE outside the source tree:

- File/product version 0.1.1; one responsive `PhotoForge` window; no console window.
- At five seconds: 24.7 MB working set, 4.8 MB private memory, zero TCP connections.
- Over a subsequent 15 idle seconds: 0.000 CPU seconds, stable 24.7 MB working set, stable 4.8 MB private memory, zero TCP connections.
- A generated 6000×4000 JPEG loaded successfully from the removable drive. The accessibility tree reported `6000 × 4000 · JPEG`, 780.5 KB, 0 operations, `Preview current`, and 289 ms. The process remained responsive; after full decode it used approximately 147.7 MB working set and 89.0 MB private memory.
- Keyboard focus on Brightness plus `End` produced one operation and a current preview, confirming final packaged keyboard editing at the control maximum.

## Installer lifecycle

The final NSIS installer was installed silently into a clean per-user state:

- installer exit code 0;
- registered display version 0.1.1;
- install location `%LOCALAPPDATA%\PhotoForge`;
- Start menu shortcut created;
- shortcut launched a responsive 0.1.1 executable with zero observed TCP connections;
- uninstaller exit code 0;
- install directory, Start menu shortcut, and uninstall registry entry all removed.

The MSI was successfully built and hashed but was not separately installed after the equivalent NSIS lifecycle passed, avoiding a second product registration on the audit machine.

## Privacy and security review

- Only `open_image`, `render_preview`, and `export_image` cross the Tauri command boundary.
- The webview has no shell command, unrestricted filesystem API, arbitrary URL loader, telemetry, analytics, remote logging, or model runtime.
- CSP permits local content/data/blob images and Tauri IPC; it does not permit remote script, style, or image origins.
- Paths are canonicalized, input formats are detected from content, decoder limits apply before full decode, operations are typed and validated, and export cannot target the canonical source path.
- Previews remain in decoded memory/data URLs and are not written to temporary files.
- Zero TCP connections were observed in baseline idle, final portable idle/24 MP use, and installed-app startup. This supports the source review but is not a guarantee about unrelated OS/WebView2 behavior.

## Deferred items and known limitations

- Drag-and-drop was source-reviewed but not replayed with global pointer automation; native picker behavior was exercised in both baseline and final packages.
- Exact 125% and 150% display-scale matrix testing, screen-reader testing, and every visual extreme remain useful human QA. The baseline was visually inspected in a large high-DPI window and keyboard focus was visible.
- MSI installation was not duplicated after a complete NSIS install/launch/uninstall lifecycle.
- Existing destination files are written directly after the native save dialog confirms replacement. A disk-full or abrupt power-loss event could leave a partial destination file; atomic temporary-file replacement is deferred.
- ICC profiles, EXIF metadata, animation, HDR, linear-light/color-managed processing, and metadata preservation are outside Phase 1.1. JPEG is lossy; PNG/WebP are the alpha-preserving choices.
- Full-resolution processing is buffer-based rather than tiled and intentionally rejects images above 40 MP. Export near that boundary can be slow and is serialized.
- The application is unsigned; Windows may display publisher/reputation warnings until a future code-signing workflow is added.

## Release artifacts

`E:\PhotoForge\release\SHA256SUMS.txt` is the authoritative manifest.

| Artifact | Bytes | SHA-256 |
| --- | ---: | --- |
| `PhotoForge-portable.exe` | 10,235,904 | `82AED50D038469F12D08A80018E578C55F011C69A996E54407E24CD86BB25113` |
| `PhotoForge_0.1.1_x64-setup.exe` | 2,332,692 | `FA54DA05C9F1CA0BEB469C891C5D6E9D68EAAA49FC559160318612BC92CE3ADC` |
| `PhotoForge_0.1.1_x64_en-US.msi` | 3,436,544 | `3FF0CC55B6D768AE86E7BCAF2E5E587F919C4A895D46F2E46354560C72042F1E` |

Legacy 0.1.0 installer files were retained in `release/` for baseline comparison and are intentionally omitted from the 0.1.1 checksum manifest.
