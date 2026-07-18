# PhotoForge Phase 2 implementation plan

Baseline: version 0.1.1 at commit `cbe74bd` on July 18, 2026. The working tree was clean and synchronized with `origin/main` when this plan was created.

Status: implemented as PhotoForge 0.2.0. The operation set, parameter bounds, architecture, and deferrals below were retained. The local-contrast implementation is the documented conservative luminance-normalization alternative rather than complete CLAHE, and mild deblur is explicitly presented as clarity restoration rather than deconvolution. Measured release results are recorded in `phase-2-results.md` and `performance.md`.

## Architecture

Phase 2 extends the existing tagged `EditOperation` enum with restoration variants. It does not create a second pipeline or change history/export semantics. Algorithms live in a focused `image_processing::restoration` module; lightweight heuristics live in `image_processing::analysis`. The existing preview and export commands continue to call one validated ordered pipeline.

Image analysis is exposed as one typed Tauri command. It runs against the cached preview on a blocking worker, is serialized by a dedicated gate, is cached per document, and is protected by document/request generations so stale observations cannot replace a newer image.

No networking, model runtime, Python, GPU requirement, arbitrary filesystem capability, or runtime download is added.

## Proposed operations

| Operation | Approach | Validated parameters | Alpha |
| --- | --- | --- | --- |
| Auto white balance | Transparent-aware trimmed gray-world channel statistics; normalized and clamped gains blended with the source | strength `0…1` | Preserved |
| Local contrast | Fast bounded local-luminance normalization using a box background estimate; conservative clipped luminance adjustment | strength `0…1`, tile size `8…128`, clip limit `0.5…4` | Preserved |
| Denoise | Small edge-aware spatial filter with range weighting; 3×3 or 5×5 bounded neighborhood | strength `0…1`, edge preservation `0…1` | Preserved |
| JPEG cleanup | Conservative smoothing only across detected 8×8 boundaries with discontinuity guards | strength `0…1` | Preserved |
| Edge-aware sharpen | Luminance high-pass sharpening with radius, threshold, and correction/halo limits | strength `0…2`, radius `0.5…4`, threshold `0…0.25` | Preserved |
| Mild deblur | Two conservative thresholded high-pass passes; accurately described as mild clarity restoration rather than deconvolution | strength `0…1`, radius `0.5…3` | Preserved |
| Uneven lighting | Fast low-frequency luminance estimate, bounded additive normalization toward the global mean | strength `0…1`, radius `4…96` | Preserved |
| Document enhance | Explicit typed, documented sequence of white balance, lighting correction, local contrast, denoise, text sharpening, and optional grayscale | strength `0…1`, grayscale boolean | Preserved |

All floating-point parameters reject NaN and infinity through range validation. Neutral strength is exact identity. Kernel, radius, and tile values are strictly bounded. Tiny and transparent images are explicit test cases.

## Analysis heuristics

The deterministic analysis result contains average luminance, percentile luminance spread, channel imbalance/color-cast estimate, Laplacian-variance sharpness, high-frequency noise, local contrast, edge density, white-background ratio, and a likely-document boolean. These are observations, not diagnoses, and never auto-apply edits.

## Preview and export behavior

Preview and export use the same operation code and ordered semantics. Preview operates on the existing source copy capped at 1600 pixels; export processes the cached full-resolution image. Radii and tile sizes are pixel-domain values, so their physical scale follows the image resolution even though the algorithm is identical. The frontend retains the 120 ms debounce, one in-flight invoke, latest-state queue, history coalescing, and request generations. Rust retains serialized preview/export gates and stale-result checks.

Performance risks are bilateral-neighborhood denoising and large-radius lighting/local-contrast estimates. Denoise is limited to a 5×5 neighborhood. Background estimates use linear-time sliding box filters rather than an area-proportional kernel. Expensive work remains on blocking workers and is never polled continuously.

## Presets

Eight Phase 2 presets will be ordinary visible arrays of typed restoration operations: Fix Indoor Lighting, Improve Old Scan, Clean Up JPEG, Mild Detail Recovery, Enhance Document — Color, Enhance Document — Grayscale, Fix Uneven Lighting, and Conservative Photo Restore. Applying a preset is one undoable history commit while its individual operations remain inspectable in the pipeline data.

## Testing strategy

- Rust unit tests use generated deterministic pixels for identity, validation, NaN/infinity, alpha, tiny images, warm/cool casts, contrast, synthetic noise, 8×8 boundaries, edges, gradients, documents, pipeline ordering, history, and analysis heuristics.
- Command/state tests cover analysis caching and generation behavior where practical through extracted pure helpers and state invariants.
- Frontend tests cover restoration definitions, presets, coalescing, analysis-observation rendering, advanced controls, labels, disabled/warning states, and processing UI.
- Release validation uses generated license-free fixtures only, exercises representative packaged workflows, records performance/resources, verifies export formats, checks TCP connections, and tests installer lifecycle.

## Deferred functionality

Perspective correction, auto-crop, OCR, batch processing, natural-language editing, Ollama/LLMs, ONNX, neural restoration/deblurring, super-resolution, face restoration, inpainting, generative enhancement, plugins, telemetry, cloud APIs, accounts, auto-update, and non-Windows packaging remain out of scope.

Phase 2 does not claim to recreate missing content or factual detail. Code signing, color-managed/linear-light processing, ICC/EXIF preservation, tiled full-resolution processing, and atomic destination replacement also remain later work.
