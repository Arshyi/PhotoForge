# Phase 7 results

PhotoForge 0.7.0 adds a reusable, deterministic selection and masking subsystem across Rust processing, typed Tauri commands, Svelte image-space tools, bounded history/session state, workflows, and local mask interchange. It adds no cloud service, telemetry, model download, generative editing, semantic recognition, or mandatory neural inference.

## Delivered

- Rectangle, ellipse, freehand lasso, polygon lasso, selection brush/eraser, color range, and contiguous/non-contiguous magic wand.
- Replace, Add, Subtract, and Intersect composition with keyboard modifiers.
- Select All, Deselect, Invert, Feather, Expand, Contract, Smooth, Fill Holes, Remove Small Islands, Border, and gradient-aware Refine Selection.
- Marching-ants, color, grayscale, black, white, and mask-only overlays.
- Explicit Global, Inside selection, and Outside selection pipeline behavior. Supported pixel adjustments use coverage blending and preserve source alpha; old operations remain global.
- Stable named masks with create/rename/duplicate/delete/visibility/lock/reorder/load/replace/combine/import/export actions.
- Bounded logical selection undo/redo, completed-stroke grouping, local session restoration, immutable workflow snapshots, and fail-closed mask validation.
- Versioned JSON mask files with raw/RLE coverage and integrity checks, plus grayscale PNG interchange.
- Planner/engine capability negotiation reports current planner selection planning as unavailable and deterministic masked adjustments as available.

## Automated verification

The verified checks at the implementation checkpoint were:

| Check | Result |
| --- | --- |
| Rust library tests | 392 passed, 0 failed |
| Frontend Vitest | 290 passed, 0 failed across 20 files |
| `cargo clippy --all-targets --all-features -- -D warnings` | Passed |
| `npm run check` | 0 errors, 0 warnings |

Coverage includes geometry, self-intersections, brush interpolation, coverage composition, flood-fill connectivity/transparency/pathological input, color range, feather/morphology/refinement, cancellation, raw/RLE/PNG persistence, corruption and size rejection, masked blending, workflow round-trip/fail-closed behavior, high-DPI coordinate mapping, state history, named masks, overlays, and session validation.

## Release-mode performance

Measured on 2026-08-05 with generated fixtures on a Dell Precision 7770, Windows 11 Pro build 26200, Intel Core i7-12850HX (16 cores/24 logical processors), and 127.7 GiB physical RAM. Command: `cargo run --manifest-path src-tauri/Cargo.toml --release --example mask_benchmark`.

| Operation | Time |
| --- | ---: |
| Rectangle, 6000×4000 | 46.234 ms |
| Freehand lasso, 180 points, 4000×3000 | 13.154 ms |
| Polygon, 500 points, 1920×1080 | 3.405 ms |
| Brush, 500 points, 4000×3000 | 50.095 ms |
| Feather radius 4/16/64, 4000×3000 | 68.496 / 69.503 / 69.875 ms |
| Expand / contract radius 24, 4000×3000 | 115.462 / 116.348 ms |
| Full single-color magic wand, 6000×4000 | 1633.557 ms |
| Mask save / load, 6000×4000 | 26.917 / 51.647 ms |
| Masked brightness, 4000×3000 | 167.739 ms |
| Cancellation acknowledgement after signal | 0.994 ms |

A 6000×4000 coverage plane is 24,000,000 bytes and the benchmark RGBA fixture is 96,000,000 bytes. Traversal queues and visited arrays are bounded by checked pixel counts. The benchmark does not claim GPU acceleration or a measured process peak; algorithms are CPU-only.

## Packaged startup and network observation

The final portable executable produced a responsive window in 1,075 ms and remained alive and responsive during the smoke test. Its seven-process application/WebView2 tree carried the requested `--disable-background-networking` argument. The PhotoForge process opened no TCP connection, but the Microsoft WebView2 151 browser-host child opened two TLS connections to Microsoft IPv6 endpoints. This is not attributed to PhotoForge code, but it means the release cannot honestly claim a zero-connection process tree on this Windows runtime. [Microsoft documents required WebView2 diagnostics](https://learn.microsoft.com/en-us/microsoft-edge/webview2/concepts/data-privacy) as governed by Windows settings rather than wholly controlled by an embedding app.

The NSIS installer exited successfully and registered PhotoForge 0.7.0 for the current user. The clean installed executable produced a responsive window in 977 ms. Its silent uninstaller then exited successfully and removed the uninstall registration, executable, and installation directory. MSI construction completed successfully but its install lifecycle was not separately exercised.

## Release artifacts and dependency impact

| Artifact | 0.6.0 | 0.7.0 | Change | SHA-256 (0.7.0) |
| --- | ---: | ---: | ---: | --- |
| Portable executable | 13,235,712 bytes | 14,028,800 bytes | +793,088 (+5.99%) | `bae1c9a72d48a0f4f704de91675cc53c85b5566d0132fc5d283a110592743037` |
| NSIS setup | 3,026,732 bytes | 3,179,582 bytes | +152,850 (+5.05%) | `4b4e9d6fbd7e8d6471076ff2c1ad7c4f3ee17de47e8dd0ce2fb9f5f503685aef` |
| MSI | 4,435,968 bytes | 4,661,248 bytes | +225,280 (+5.08%) | `2f5d3c369fe89403d40aeef50d9d8e5db7a0258d35e0275ebb97f3baa3fee7b2` |

The production frontend contains 153 modules and bundles to 237.83 kB JavaScript and 55.72 kB CSS (74.09 kB and 10.24 kB gzip). Phase 7 adds no npm or Cargo dependency; the lockfiles change only the PhotoForge package version. It adds no model weights, inference runtime, server, Python component, GPU requirement, or cloud client.

## Compatibility and safety

Workflow schema remains version 1. Old workflows contain ordinary operations and replay globally without migration. A masked workflow operation embeds an immutable, integrity-checked snapshot. Corrupt, unsupported, dimension-incompatible, or absent snapshot data fails validation before the pipeline changes, so replay cannot silently fall back to global.

Mask operations accept finite coordinates/settings, cap dimensions at 100 million pixels, use checked allocation arithmetic, reject malformed Base64/RLE/checksums, validate local paths, use stale document/request generations, and check cancellation in expensive loops. No mask command contains networking code. Ollama still receives no pixels or mask coverage.

## Known limitations and deferred work

- The general PhotoForge project-file system requested by the Phase 7 brief did not exist in Phase 6. Version 0.7.0 provides bounded per-document WebView session restoration; noisy sessions over 3.5 million characters must be exported explicitly.
- Named-mask visibility is persisted, but the stage renders the active selection rather than compositing all visible named masks. Mask thumbnails are restrained glyphs, not lazily generated bitmap previews.
- Masks are not geometrically remapped through crop, perspective, straighten, lens correction, reflect, or 90-degree rotation. Masked adjustments after a changed aspect ratio fail closed; apply them before geometry or rebuild the mask.
- Refine Selection uses overlay backgrounds and undo for comparison; there is no dedicated before/after dialog. Decontaminate Colors is omitted.
- Color-range multi-sampling is expressed as sequential Add/Subtract clicks rather than a separate editable sample list.
- There is a busy/cancel state but no numerical progress percentage. Pointer pressure is not used.
- Automated packaged-UI interaction could not be completed because the Codex Windows-control runtime could not load its prescribed module from the desktop installation path. Selection components are covered by frontend tests and both portable/installed windows were observed responsive, but the full hands-on image/tool matrix is not claimed.
