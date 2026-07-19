# Performance

## Phase 1 design targets

- Near-zero idle CPU: no polling loop, animation is limited to an active processing indicator, and no model loads at startup.
- Fast startup: the application loads a small Svelte bundle and native Rust commands; all image decode work begins only after open.
- Responsive adjustment: controls debounce for 120 ms and use a decoded preview capped at 1600 pixels.
- Responsive UI: decode and pipeline execution run on Tauri blocking workers rather than the webview thread.
- Bounded input: encoded images above 750 MiB, dimensions above 20,000 pixels, decoder allocation above 256 MiB, or images above 40 megapixels are rejected before full processing.
- Reduced copying: full and preview images are retained behind `Arc`; full resolution is processed only for export.
- Bounded work: one preview and one export may consume CPU at a time; superseded preview states are coalesced before Rust processing.

## Measurement

The status bar reports end-to-end processing time for each preview and export using Rust's monotonic `Instant`. This is the useful measurement for a user's actual image, hardware, pipeline order, and output format.

Phase 1.1 records packaged-app observations and a 24 MP fixture workflow in `phase-1-1-results.md`. These are practical release observations on one Windows machine, not cross-machine performance guarantees.

The packaged Windows release completed a startup smoke test on July 18, 2026: the process remained alive and Windows reported it as responsive after five seconds. This verifies startup viability, not a formal startup-duration measurement.

## Phase 2 measurements

These observations were taken from the packaged 0.2.0 Windows application on one development machine on July 18, 2026. They are practical regression checks rather than cross-machine benchmarks. The operation samples used a generated 1920×1080 RGBA fixture, each tool's maximum main strength, and default advanced settings. The status-bar duration is the Rust command's end-to-end processing time.

| Restoration preview | Time |
| --- | ---: |
| Auto White Balance | 49 ms |
| Local Contrast | 92 ms |
| Denoise | 518 ms |
| JPEG Cleanup | 18 ms |
| Edge-Aware Sharpen | 85 ms |
| Mild Deblur | 156 ms |
| Uneven Lighting | 91 ms |
| Document Enhance | 484 ms |
| Conservative Photo Restore preset (4 operations) | 394 ms |

Ten rapid keyboard change groups on Auto White Balance coalesced to one current preview in 61 ms once processing settled. The UI remained responsive and reported no error.

Full-resolution export of a generated 4000×3000 JPEG with the four-operation Conservative Photo Restore preset took 3,448 ms for lossless WebP processing/encoding. PNG, JPEG, and WebP exports all retained 4000×3000 dimensions. The WebP output was 732,914 bytes. Immediately after this workflow, Windows reported 118.8 MiB working set and 59.9 MiB private memory. A generated 6000×4000 image also opened successfully and participated in a new-image-during-processing race check below the unchanged 40 MP ceiling.

The portable application produced its first responsive window in approximately 690 ms. A clean NSIS installation produced a responsive 0.2.0 window in 803 ms. After 15 seconds idle, the installed process accumulated 78.1 ms of CPU time (about 0.52% of one logical core on average), used 26.5 MiB working set and 5.2 MiB private memory, and had zero TCP connections.

### Size and dependency impact

| Artifact | 0.1.1 | 0.2.0 | Change |
| --- | ---: | ---: | ---: |
| Portable executable | 10,235,904 bytes | 10,442,752 bytes | +206,848 bytes (+2.02%) |
| NSIS setup | 2,332,692 bytes | 2,370,608 bytes | +37,916 bytes (+1.63%) |
| MSI | 3,436,544 bytes | 3,502,080 bytes | +65,536 bytes (+1.91%) |

No npm or Cargo dependency was added for Phase 2. `package-lock.json` and `Cargo.lock` change only the PhotoForge package version. The production frontend changed from 88.11 kB JavaScript / 17.46 kB CSS in the Phase 1.1 build to 102.78 kB JavaScript / 20.39 kB CSS; gzip sizes are 33.52 kB and 4.88 kB respectively. The release adds no model, runtime download, Python component, GPU requirement, or networking library.

## Phase 3 measurements

These observations were taken from the packaged 0.3.0 Windows application on the same development machine on July 18, 2026. They are release regression checks, not cross-machine guarantees.

The rule-based planner was measured after image analysis was cached. Across 100 sequential typed-plan requests, Rust-reported planning time was 0.0093–0.0206 ms, with a 0.0117 ms median and 0.0145 ms p95. Every sample was below the 50 ms target. Eleven additional packaged cases covered dark and bright photos, a receipt, handwritten notes, a screenshot, an old scan, damaged JPEG input, a portrait, a transparent PNG, and 12/24 MP JPEGs; their planner times were 0.0105–0.0161 ms. These measurements cover deterministic rule matching, plan construction, explanation construction, ordering, and validation. They do not include image decode or initial analysis.

The 12 MP and 24 MP fixtures opened in 185 ms and 265 ms respectively. Analysis uses the capped working preview and took 67 ms and 61 ms. A reviewed two-operation guided pipeline exported the 6000×4000 fixture at full resolution in 2,191 ms to PNG, 2,877 ms to JPEG, and 2,197 ms to WebP. All three outputs retained 6000×4000 dimensions.

The portable release produced its first responsive window in approximately 671 ms. The installed NSIS release produced a responsive window in 755 ms. After the full fixture/planner/export workflow, Windows reported 146.5 MiB working set and 92.6 MiB private memory; this is a post-workflow observation, not a clean-start idle baseline. The packaged process had zero external TCP connections.

### Phase 3 size and dependency impact

| Artifact | 0.2.0 | 0.3.0 | Change |
| --- | ---: | ---: | ---: |
| Portable executable | 10,442,752 bytes | 10,548,224 bytes | +105,472 bytes (+1.01%) |
| NSIS setup | 2,370,608 bytes | 2,407,396 bytes | +36,788 bytes (+1.55%) |
| MSI | 3,502,080 bytes | 3,547,136 bytes | +45,056 bytes (+1.29%) |

No npm or Cargo dependency was added for Phase 3. The production frontend is 116.39 kB JavaScript / 26.00 kB CSS, with gzip sizes of 37.94 kB and 5.79 kB. The release adds no model weights, model server, Python component, GPU requirement, or networking library.

## Known tradeoffs

- The source converts to RGBA once per non-empty pipeline, but each operation still allocates an output buffer for clarity and correctness.
- Preview transport PNG-encodes the result, reducing IPC size at the cost of encoding time.
- Unsharp masking creates one temporary blurred image.
- ICC/EXIF preservation and tiled processing are not in Phase 1.
- Full-resolution export is deliberately serialized and may take time near the 40 MP ceiling.
- Edge-preserving denoise and document enhancement are the most expensive Phase 2 previews because they sample bounded neighborhoods or run several passes.
- Pixel-domain restoration radii retain identical preview/export semantics, but cover a different physical portion of downscaled previews and full-resolution images.
- Planner timings are extremely small because Phase 3 deliberately performs bounded string matching against cached scalar analysis; initial image decode and analysis remain separate work.

## Phase 4 measurements

These observations were taken from the final packaged 0.4.0 application on the same Windows machine on July 18, 2026. The untouched Phase 3 portable executable was verified by its published SHA-256 before comparison.

The explicit local component diagnostic used 250 samples. Registry lookup averaged below the timer's per-sample display resolution (`<1 ns`), Rule Planner trait dispatch including plan construction and validation averaged 4.25 µs, and built-in factory/loading bookkeeping averaged 3 ns. These are same-machine diagnostic observations, not portable microbenchmark guarantees. No optional component, model, plugin entry, or network connection was loaded.

Three alternating warm startup samples produced these medians:

| Version | Responsive-window time | Main working set | Private memory |
| --- | ---: | ---: | ---: |
| 0.3.0 | 76 ms | 22.2 MiB | 3.5 MiB |
| 0.4.0 | 66 ms | 22.1 MiB | 3.5 MiB |

Separate clean-session observations were 422 ms for 0.3.0 and 484 ms for 0.4.0; WebView2 process reuse makes individual startup observations variable. After ten seconds idle, 0.4.0's main process used 26.5 MiB working set / 4.9 MiB private memory and accumulated 78.1 ms CPU. The seven-process application/WebView tree used 410.5 MiB working set / 251.7 MiB private memory and held zero TCP connections. The main-process comparison and identical production dependency set show no mandatory-model memory penalty.

### Phase 4 size and dependency impact

| Artifact | 0.3.0 | 0.4.0 | Change |
| --- | ---: | ---: | ---: |
| Portable executable | 10,548,224 bytes | 10,895,872 bytes | +347,648 bytes (+3.30%) |
| NSIS setup | 2,407,396 bytes | 2,482,549 bytes | +75,153 bytes (+3.12%) |
| MSI | 3,547,136 bytes | 3,641,344 bytes | +94,208 bytes (+2.66%) |

The production frontend is 131.04 kB JavaScript / 32.20 kB CSS, with gzip sizes of 42.15 kB and 6.74 kB. No npm package or Cargo package was added; only the existing Tokio dependency's lightweight `time` feature was enabled. The release includes no model weights, inference runtime, server, Python component, GPU requirement, or networking library.
