# PhotoForge

PhotoForge is a lightweight, privacy-first desktop photo restoration and enhancement tool. It processes PNG, JPEG, and WebP images locally with a typed, non-destructive edit pipeline and never uploads photos.

This repository contains the Phase 0 foundation through Phase 13 semantic layer
editing and optional local inference. The target version is **0.13.0**. See
[Phase 13 results](docs/phase-13-results.md) for current verification and
release limits; Phase 10 remains the precision and colour baseline and Phase 11
the tiled-render baseline.

## Restoration (Phase 12)

Every restoration tool is deterministic and runs locally with no model
installed. Measured against known-clean fixtures, against Phase 11: colour
noise +7.9 dB, sensor defects +15.1 dB, impulse noise +5.8 dB, motion blur
+4.4 dB, with edge retention restored from 0.64-0.85 to about 1.00. Denoise
separates luminance from colour; dust and hot pixels get a dedicated repairer
because a bilateral filter cannot fix an impulse; deconvolution inverts a blur
you name rather than guessing one. See [Restoration](docs/restoration.md).

PhotoForge can also run a neural model **you install yourself**. It ships none
and downloads none, and the editor is complete without one. See
[Local inference](docs/local-inference.md) and
[Model format](docs/model-format.md).

## Bounded rendering performance (Phase 11)

- The linear-float compositor renders bounded tiles across a capped CPU worker
  pool and falls back to its full-frame reference when an operation cannot be
  evaluated correctly from a tile plus halo.
- A bounded in-memory LRU reuses exact rendered tiles in interactive previews.
  Diagnostics expose real hits, misses, evictions, refusals, entries and bytes;
  the user can shrink or clear the cache explicitly.
- GPU support is an optional build feature. When included and available, it can
  accelerate only sufficiently large, wide Gaussian blurs whose measured cost
  beats the CPU. CPU remains the correctness reference and automatic fallback.
- Diagnostics distinguish the selected policy, effective path, adapter state,
  and actual dispatch counters. They never label the whole compositor GPU.

See [GPU and tiled rendering](docs/gpu-rendering.md).

## High-precision editing (Phase 10)

- Linear f32 layer/group/mask/adjustment/transform rendering with all 16 blend modes
- Genuine layered PNG16 export, native PNG16 import, and typed project/recovery data
- Embedded RGB ICC input and actual sRGB / Display P3 / Adobe RGB output transforms
- Source-backed DNG placement and undoable full-resolution RAW re-development
- Sequential RAW batch export with profile/bit-depth controls and source protection
- Checked memory admission, bounded immutable caches, and measured 45/60 MP synthetic cases
- Explicit legacy rendering for older projects; no silent appearance migration

The CPU renderer is authoritative. Preview remains 8-bit sRGB; Windows/WebView2
control monitor presentation. [Color pipeline](docs/color-pipeline.md).

## Camera RAW development (0.9.0)

PhotoForge decodes camera RAW files locally and develops them without ever
writing to the original.

- **DNG decoding written against the published specification** — uncompressed
  and lossless-JPEG, at 8, 10, 12, 14, and 16 bits, with no third-party decoder
  linked and no native library to package
- **Real sensor handling** — CFA pattern, per-position black levels, white
  level, active area, default crop, orientation, as-shot neutral, and the
  camera colour matrix, all read from the file rather than assumed
- **Two demosaic algorithms** — bilinear for previews, Malvar-He-Cutler for
  final renders, the latter measured against ground truth rather than merely
  labelled higher quality
- **Highlight headroom** — nothing clips before the display transform, so
  reducing exposure genuinely recovers detail an 8-bit path would have lost
- **Source-backed projects** — a RAW layer stores the file it came from, its
  SHA-256, and its development parameters alongside the exact developed pixels;
  explicit re-development returns to the verified original
- **Missing and changed sources** — detected and reported; a relink verifies the
  hash and refuses a different photograph
- **True 16-bit PNG export** at full sensor resolution, quantised only at the
  file boundary

**Only DNG is decoded.** CR2, CR3, NEF, ARW, RAF, ORF, and RW2 are recognised
so the interface can explain itself, but no decoder for them is bundled.
The scoped dependency assessment is documented below. Supported DNG variants
can be used after external conversion; not every DNG encoding is supported.
The float source ceiling is 1 GiB (67,108,864 pixels), with separate cache and
job estimates; legal dimensions can still be rejected when history or scratch
would exceed those budgets. See
[docs/raw-development.md](docs/raw-development.md) and
[docs/color-pipeline.md](docs/color-pipeline.md).

## Layers and non-destructive editing (0.13.0; 0.8.x baseline retained)

PhotoForge is now a layer-based editor. Documents hold a real layer tree — pixel
layers, nestable groups, parametric adjustments, editable shapes and text, and
shared smart-object instances — composited by a
deterministic renderer with sixteen blend modes, correct straight-alpha
compositing, per-layer non-destructive transforms, and layer masks that reuse
the existing Phase 7 mask engine unchanged.

- **Pixel, group, and adjustment layers** with stable identifiers, visibility,
  lock, opacity, blend mode, transform, mask, and metadata
- **Non-destructive adjustment layers** that store parameters, never baked
  pixels, and recompute whenever they change
- **Layer masks** built from selections, invertible, disableable, applicable,
  and loadable back as selections
- **An editable project format**, `.photoforge`, that stores the tree rather
  than a flattened image
- **Full undo and redo** for every layer operation, with slider drags and
  drag-and-drop reorders each collapsing into one logical step
- **An explicit editing target** so painting into a mask can never be mistaken
  for painting into pixels
- **Current-layer sampling** for histogram, pixel inspection, and selection
  tools, with stale-document guards
- **Mask-target shape and brush gestures** mapped through a pixel layer's
  transform and committed as one history entry
- **Fail-closed merge safety** for contiguous sibling ranges whose omitted
  backdrop cannot affect the result
- **Editable curves and selective-colour adjustment layers**, including a
  keyboard-operable curve editor
- **Bounded local recovery snapshots** for unsaved work, with a startup recovery
  choice and no cloud storage
- **Workflow schema 2 layer steps** with fail-closed selectors and planner-safe
  restrictions; schema 1 workflows still load unchanged
- **Batch rendering of `.photoforge` projects** without modifying the project
  file
- **Sibling multi-selection and grouping** in the Layers panel
- **Editable vector shapes and text** with explicit rasterization when pixels
  are wanted
- **Smart objects** with shared native-size source stacks, nested validation,
  independent copies, explicit local link checks, and relinking

Opening an ordinary photo produces a single background layer in the linear
document renderer. Older byte projects keep their compatibility path.
See [docs/layers.md](docs/layers.md),
[docs/compositing.md](docs/compositing.md), and
[docs/project-format.md](docs/project-format.md). The smart-object boundary is
documented in [docs/smart-objects.md](docs/smart-objects.md).

PhotoForge does not support PSD, general GPU compositing, general TIFF import, CMYK,
printer proofing or proprietary RAW formats. Earlier phase reports remain
historical evidence; [Phase 13 results](docs/phase-13-results.md) records current
verification, the packaged 0.13.0 artifacts and their checksums, and what is
still not claimed.

## What works

- Native image picker and desktop drag-and-drop
- Cached, downscaled previews for responsive editing
- Brightness, contrast, gamma, saturation, grayscale, sepia, blur, unsharp masking, rotation, and horizontal reflection
- Ordinary typed pipelines for five presets
- Undo, redo, reset, zoom, fit view, and before/after comparison
- PNG, JPEG, and WebP full-resolution export
- Protection against overwriting the original image
- Document and request-generation checks that prevent stale opens or previews from replacing newer results
- Bounded preview/export work and a 200-entry history ceiling
- Auto white balance, local contrast, edge-preserving denoise, JPEG cleanup, edge-aware sharpening, mild deblur, uneven-lighting correction, and document enhancement
- Eight conservative restoration presets made from inspectable typed operations
- Local heuristic image analysis that observes luminance, color cast, noise, sharpness, contrast, edges, and document-like structure without auto-applying edits
- A deterministic `RuleBasedPlanner` that maps supported plain-English requests to reviewable typed edit plans
- A keyboard-accessible plan inspector with summary, heuristic confidence, warnings, explanations, deletion, reordering, strength adjustment, validation, Apply, and Cancel
- Ten suggested guided prompts, up to 25 optional locally stored recent requests, and four local planner-display/history preferences
- Typed `EditPlanner` and `RestorationEngine` interfaces with registries and factories; Phase 3 behavior remains the default
- An optional proxy-disabled, redirect-disabled, loopback-only Ollama client with explicit Test Connection, model refresh, plan generation, timeout, response ceiling, and cancellation
- Strict deny-unknown-fields Ollama JSON parsing followed by the existing `EditPlan` validator; operation explanations are created locally
- Rule/Ollama planner selection, explicit fallback, read-only raw JSON, validation reports, and side-by-side comparison with no automatic winner
- Provider-tagged prompt history plus local Ollama status, latency, validation, rejection, success, and cancellation diagnostics
- A dedicated Local AI Privacy page documenting the exact text-only boundary
- A Components settings page with provider, version, memory estimate, status, capabilities, local paths, and safely disabled future adapters
- A Diagnostics page for registered, loaded, unavailable, failed, and invalid components plus explicit local overhead measurement
- Local-only model metadata discovery and bounded plugin-manifest validation; no model is loaded and no plugin code is executed
- No telemetry, analytics, remote logging, Python runtime, cloud service, or mandatory AI model
- Image-space rectangle, ellipse, freehand-lasso, polygon-lasso, brush, eraser, magic-wand, and color-range selections
- Replace, Add, Subtract, and Intersect coverage composition with feathered 8-bit masks
- Invert, feather, expand, contract, smooth, fill-holes, island cleanup, border, and classical edge-aware refinement operations
- Global, inside-mask, and outside-mask application for deterministic pixel adjustments without changing existing global behavior
- Named masks with stable identifiers, lock/visibility/reorder/load/replace/combine actions, bounded undo/redo, and local session restoration
- Integrity-checked `.photoforge-mask.json` and grayscale PNG mask import/export with bounded raw/run-length encoding, request-scoped numerical progress, cooperative cancellation, and atomic destination replacement
- Transactional active, named, and embedded workflow-mask remapping through crop, quarter-turn rotation, horizontal reflection, straighten, perspective, and bounded lens-distortion operations
- Real lazy grayscale mask thumbnails, visible named-mask overlays, delayed numerical progress, optional pen pressure, and a dedicated Refine Selection comparison dialog with opt-in deterministic edge-color decontamination
- Phase 9 precision foundation: bounded linear-sRGB f32 development buffers,
  stop-based exposure/WB/tone controls, clipping-aware histograms, true 16-bit
  PNG encoding, and safe metadata-only RAW inspection with source hashing

## Requirements

- Windows 10 or 11 with a current Evergreen Microsoft Edge WebView2 runtime that supports `ICoreWebView2Settings8`; startup fails closed on an older runtime that cannot apply and verify the documented reputation-checking policy
- Node.js 20 or newer and npm
- Rust stable with the `x86_64-pc-windows-msvc` toolchain
- Microsoft C++ Build Tools with the Desktop development with C++ workload

See the official [Tauri prerequisites](https://v2.tauri.app/start/prerequisites/) if the Rust linker is unavailable.

## Develop

```powershell
npm install
npm run tauri dev
```

The Vite-only UI can be opened with `npm run dev`, but native image open, processing, and export commands require the Tauri runtime.

## Verify

```powershell
cargo fmt --manifest-path src-tauri/Cargo.toml --check
cargo clippy --manifest-path src-tauri/Cargo.toml --all-targets --all-features -- -D warnings
cargo test --manifest-path src-tauri/Cargo.toml
npm run check
npm run test
npm run build
```

## Package

```powershell
npm run tauri build
```

Windows installers are written under `src-tauri/target/release/bundle/`.

## Use

1. Select **Open**, press `Ctrl+O`, or drop a supported image into the workspace.
2. Adjust controls or choose a preset. Slider gestures are coalesced into useful undo steps, and previews are generated from a cached copy capped at 1600 pixels.
3. Use **Guided Edit** to describe a supported result in ordinary language. Review, reorder, remove, or adjust every proposed operation before applying it.
4. Use the **Restoration** section for direct control over captured color, lighting, noise, compression, softness, or document-readability problems. Advanced controls are optional.
5. Review **Image Analysis** as a cautious heuristic summary; it never applies edits automatically.
6. Use **Compare** to drag between images. Rotated comparisons switch to side-by-side views so neither image is distorted.
7. Undo with `Ctrl+Z`, redo with `Ctrl+Y` or `Ctrl+Shift+Z`, or reset the pipeline.
8. Select **Export** or press `Ctrl+S`. PhotoForge processes the original at full resolution and requires a destination different from the source file.
9. Open **Settings → Components** to inspect or configure optional providers. Rule Planner and the Deterministic Engine remain the defaults. Ollama requires an already installed local model; PhotoForge never downloads one.
10. Use **Test Connection** or **Refresh Models** explicitly before selecting an Ollama Planner Model. Generate Plan and Compare Planners are the only other actions that contact the configured loopback endpoint.
11. Open **Settings → Local AI Privacy** to inspect the exact information boundary before enabling Ollama.
12. Use **Selections & Masks** to choose a tool, compose an active selection, save named masks, and set new adjustments to Global, Inside selection, or Outside selection. Shift adds, Alt subtracts, and Shift+Alt intersects.

## Architecture and project notes

- [Architecture](docs/architecture.md)
- [Image processing](docs/image-processing.md)
- [Restoration](docs/restoration.md)
- [Local inference](docs/local-inference.md)
- [Model format](docs/model-format.md)
- [Privacy](docs/privacy.md)
- [WebView2 network boundary](docs/webview-network-boundary.md)
- [Performance](docs/performance.md)
- [Roadmap](docs/roadmap.md)
- [Phase checklist](docs/checklist.md)
- [Phase 1.1 audit](docs/phase-1-1-audit.md)
- [Phase 1.1 results](docs/phase-1-1-results.md)
- [Phase 2 plan](docs/phase-2-plan.md)
- [Phase 2 results](docs/phase-2-results.md)
- [Phase 3 results](docs/phase-3-results.md)
- [Component architecture](docs/component-architecture.md)
- [Plugin manifest specification](docs/plugin-specification.md)
- [Phase 4 results](docs/phase-4-results.md)
- [Ollama provider](docs/ollama-provider.md)
- [Local AI privacy](docs/local-ai-privacy.md)
- [Phase 5 results](docs/phase-5-results.md)
- [Professional tools](docs/professional-tools.md)
- [Workflows](docs/workflows.md)
- [Batch processing](docs/batch-processing.md)
- [Phase 6 results](docs/phase-6-results.md)
- [Selections and masks](docs/selections-and-masks.md)
- [Mask file format](docs/mask-file-format.md)
- [Phase 7 results](docs/phase-7-results.md)
- [Phase 7.1 results](docs/phase-7.1-results.md)
- [Layers](docs/layers.md)
- [Vector shape layers](docs/vector-layers.md)
- [Text layers](docs/text-layers.md)
- [Smart objects](docs/smart-objects.md)
- [Compositing](docs/compositing.md)
- [Project format](docs/project-format.md)
- [Phase 8 results](docs/phase-8-results.md)
- [Colour pipeline](docs/color-pipeline.md)
- [RAW development](docs/raw-development.md)
- [Phase 9 results (historical)](docs/phase-9-results.md)
- [High-precision architecture](docs/high-precision-rendering.md)
- [Phase 10 results](docs/phase-10-results.md)
- [GPU and tiled rendering](docs/gpu-rendering.md)
- [Phase 11 results](docs/phase-11-results.md)
- [Phase 12 results](docs/phase-12-results.md)
- [Phase 13 results](docs/phase-13-results.md)

## Honest scope

PhotoForge 0.13.0 adds deterministic local layers, editable projects, bounded local
recovery, layer-aware workflows, semantic text/vector layers, bounded smart
objects, and project batch rendering while preserving the Phase 7.1 mask
boundary. Rule Planner remains the default; optional Ollama remains
a text-only local planning adapter and receives no image, mask, layer tree, or
path. The Deterministic Engine remains the only component that changes pixels.
PhotoForge does not install or download models, execute model-supplied code, call
cloud providers, execute plugins, generate missing content, or reconstruct
factual detail that was never captured. RAW decoding, demosaicing, and
source-backed RAW projects are implemented for the documented DNG subset.
Phase 10 adds float compositing, RGB ICC input, sRGB/P3/Adobe RGB output and
batch RAW. Phase 11 adds bounded CPU tiles, an in-memory render cache, and
optional Vulkan acceleration for eligible wide Gaussian blur only. Arbitrary
output ICC, printer proofing, PSD, general GPU compositing,
semantic selection, OCR, neural restoration, super-resolution, inpainting,
generative editing, procedural/neural layer kinds, PSD, and general GPU
compositing remain outside this release. Native/package validation limits are
listed separately in the Phase 13 report.

## License

No license has been selected yet. All rights are reserved until the repository owner adds one.
