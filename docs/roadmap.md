# Roadmap

## Now: after 0.15.0

The sections below this one are the history of what each phase delivered, in the order they
were written; this one is where the project stands and where it is going.

Where the project is, what is left, and the decisions that have been made. It is a plan, not a
promise: nothing here is claimed until a results page says it was run.

### Where it is

**0.15.0 (Phase 15, interoperability) is released**: the tag `v0.15.0` and a GitHub pre-release
with unsigned installers. [Phase 15 results](phase-15-results.md) is the ledger of what it holds and
what is not claimed.

### The sequence

| Version | Theme | Notes |
| --- | --- | --- |
| 0.15 | Interoperability: PSD / PSB, TIFF, OpenEXR, SVG, metadata, clipboard, DNG picture handling | **done** |
| 0.16 | Local semantic selection, object understanding, inpainting and generative restoration | Models are packages the user supplies through the model manager, never in the installer or the repository |
| 0.17 | GPU renderer parity and CPU/GPU scheduling | Only where measured to help (Phase 11 is the precedent) |
| 0.18 | Colour, print, proofing and HDR, and the remaining professional gaps | Includes what Phase 15 recorded as not fixed (below) |
| 0.19 | Interface and product integration: workspace, shortcuts, panels, history | Includes the display-scaling, second-monitor and screen-reader matrix |
| 1.0 | Stabilisation: fuzzing, huge-document torture tests, crash recovery, accessibility, signing, native Windows validation, performance gates | Signing needs a real certificate |

Principles, in order when they conflict: correctness, image fidelity, non-destructive editing,
security and privacy, capability, performance, interface, storage. No claim without a run.

### Open work, from the 0.15.0 ledger

Each of these is written up where it was found; they are small and independent.

* **High-dynamic-range values are clipped by most blend modes** (everything but Normal, Multiply,
  Screen, Darken, Lighten, Difference, Exclusion). A design decision about what Overlay and its
  relatives mean above white. ([layers.md](layers.md), [openexr.md](openexr.md))
* **PSD memory**: import decodes every channel (up to about 14 times the per-layer estimate with extra
  channels); export's peak is about three times what is guarded.
  ([phase-15-5-review-results.md](phase-15-5-review-results.md))
* **SVG round trip**: isolated groups written as plain `<g>`; colour names outside 40, `hsl()` and
  percentage lengths read wrongly; `<title>` not read back. ([svg.md](svg.md))
* **RAW**: DefaultScale and non-square pixels are not applied; the masked photosites are not used to
  measure the black level; only DNG is read. ([raw-development.md](raw-development.md))
* **Packaged window**: the Layers panel's controls use a different type scale from the cards; the
  clipboard has not been driven in the packaged window; other widths and display scales are
  unchecked. ([phase-15-9-1-results.md](phase-15-9-1-results.md))
* **A window that did not close** was seen once and not explained; since 0.15.0 a local log
  (`close.log`, [phase-15-9-3-results.md](phase-15-9-3-results.md)) records what happens if it recurs.
* **Installers**: the MSI is registered per machine but installs per user; upgrade over an older
  version, repair and a machine without WebView2 have not been tried.
  ([phase-15-results.md](phase-15-results.md))

### Decisions made

* **Licence: none selected.** All rights reserved; the repository is public, as a personal project.
  Nothing here is open-source licensed.
* **Binaries are unsigned** and released as a pre-release; SmartScreen warns. A certificate is a
  decision for 1.0.
* **Windows only.** The platform layers (WebView2, the clipboard) are not abstracted.
* **No telemetry, no cloud, no model in the installer.** The optional local planner (Ollama) is
  text-only and loopback-only.

### Working here

[`development.md`](development.md) is the entry point for building, verifying and releasing.

## History

## Phase 6 — Professional editing and workflow system (complete)

Version 0.6.0 adds deterministic curves, levels, point sampling, crop, straighten, perspective, lens correction, HSL, temperature/tint, selective color, live scopes, professional inspection, expanded comparison, versioned workflows, bounded batch processing, export profiles, workspace layouts, configurable shortcuts, and accessibility improvements.

## Phase 7 — Professional selections and masking (complete)

- Deterministic 8-bit mask engine with rectangle, ellipse, polygon, freehand, brush, eraser, Magic Wand, and Color Range tools
- Replace, add, subtract, intersect, invert, feather, expand, contract, smooth, cleanup, border, and classical edge-refinement operations
- Active and named masks with visibility, locking, ordering, duplication, import/export, bounded history, and per-document local sessions
- Marching-ants, color, grayscale, black/white, and mask-only overlays with image-space pointer mapping across zoom and high-DPI displays
- Immutable mask snapshots in validated typed edit operations and workflows, including inside/outside adjustment scopes
- Versioned `.photoforge-mask.json` plus grayscale PNG interchange, checksums, decompression bounds, cancellation, stale-result protection, and local-path enforcement
- No AI, semantic selection, OCR, cloud service, marketplace, account, or payment dependency

## Phase 7.1 — Phase 7 completion and hardening (complete)

- Transactional active, named, and embedded workflow-mask remapping for crop, exact rotation, horizontal reflection, straighten, perspective, and bounded lens distortion
- Lazy real-coverage grayscale thumbnails, visible named-mask overlays, bounded caches, and stricter full-resolution stage alignment
- Work-unit numerical progress with delayed display, stale-request filtering, cooperative cancelling state, terminal cleanup, and asynchronous JSON/PNG mask interchange
- Optional pen pressure resolved into deterministic brush samples, disabled by default with unchanged mouse behavior
- Dedicated Refine Selection comparison dialog with split/toggle, four background modes, and optional deterministic masked edge-color decontamination
- Versioned selection-session schema 2 with read-only Phase 7 migration; standalone mask and workflow file schemas remain compatible
- Masked-only Decontaminate Colors with safe-off defaults, bounded foreground sampling, alpha preservation, representative dialog preview, and compound selection/edit history

## Phase 0 — Foundation (complete)

- Tauri 2, Rust, Svelte 5, TypeScript, and Vite project
- Layered domain/application/infrastructure/processing/presentation design
- Typed errors and Tauri command boundary
- Documentation and test foundations

## Phase 1 — Minimum viable editor (complete)

- Native open and save dialogs plus drag-and-drop
- PNG, JPEG, and WebP decode/encode
- Metadata, cached responsive preview, full-resolution export
- Ten deterministic edit operations and five typed presets
- Undo, redo, reset, comparison, zoom, progress, and current-preview status
- Path, format, parameter, image-size, and stale-result safeguards

## Phase 1.1 — Audit and release hardening (complete)

- Document-generation protection for competing opens and previews
- Bounded/coalesced preview work and serialized export work
- Explicit PNG, JPEG, and WebP export policies
- 40 MP memory ceiling and decoder allocation limits
- Slider history coalescing and 200-entry history cap
- Correct rotated comparison layout and modal/disabled-control keyboard hardening
- Expanded format, corruption, alpha, transform, validation, and packaging verification

## Phase 2 — Deterministic restoration tools (complete)

- Automatic white balance
- Conservative local-luminance contrast normalization
- Denoising and JPEG artifact reduction
- Edge-aware sharpening and conservative mild-clarity restoration
- Color/grayscale document enhancement and uneven-lighting correction
- Lightweight cached image-quality heuristics with no automatic edits
- Eight inspectable restoration presets
- Clear language distinguishing restoration from reconstructed content

## Phase 3 — Guided local editing (complete)

- Deterministic `RuleBasedPlanner` informed by cached image-analysis heuristics
- Strict typed edit-plan schema, safe ordering/conflict validation, and proposal review UI
- Human explanations, heuristic confidence, warnings, operation removal/reordering/strength editing, and approval before applying
- Suggested requests, optional 25-entry local prompt history, and local display/history settings
- `EditPlanner` trait isolates any future optional local planner adapter without coupling the restoration engine to Ollama
- Perspective correction, auto-crop, and deterministic batch workflows remain later candidates

## Phase 4 — Extensible AI and restoration platform (complete)

- `EditPlanner` and `RestorationEngine` interfaces used by the real guided and image-processing paths
- Typed capabilities, providers, registrations, local configuration, registry, and factories
- Built-in Rule Planner and Deterministic Engine remain active by default with Phase 3 behavior
- Compiling Ollama, OpenAI, ONNX, Real-ESRGAN, and future placeholders with safe not-installed failures
- Lazy optional initialization, bounded timeout, failure diagnostics, and inactive-component unload bookkeeping
- Components and Diagnostics settings pages with unavailable providers visible but disabled
- Local model metadata discovery with no download, content loading, or inference
- Versioned plugin manifest validation with no arbitrary plugin execution

## Phase 5 — Optional Ollama local planner (complete)

- Rule Planner remains the startup default and offline fallback
- Explicit loopback-only Ollama connection testing and installed-model discovery
- Proxy-disabled, redirect-disabled, non-retrying HTTP client with timeout, cancellation, UTF-8/status handling, and bounded streamed responses
- Deterministic text-only prompt containing approved analysis, supported operations, ranges, and strict JSON schema
- Deny-unknown-fields wire parsing, rejected-field reports, locally generated explanations, and reuse of existing plan validation
- Planner selector, cancellation on prompt edits, raw/validated JSON inspection, explicit Rule fallback, and no-winner Rule/Ollama comparison
- Persisted endpoint, timeout, response ceiling, model, and operation-limit settings with visible defaults reset
- Local AI Privacy page plus connection, timing, validation, rejection, success, cancellation, and memory diagnostics
- Deterministic mock Ollama server and expanded 277-Rust/146-frontend automated coverage without a real Ollama installation

## Phase 8 — Layers and non-destructive editing (complete)

- Real layer tree with pixel layers, nestable groups, and parametric adjustment layers
- Deterministic compositor with sixteen blend modes and correct straight-alpha compositing
- Layer masks reusing the Phase 7 mask engine unchanged, with no second mask representation
- Non-destructive per-layer transforms kept separate from document geometry
- `.photoforge` project format storing the tree rather than a flattened image
- Full undo and redo for every layer operation, with gesture coalescing and no pixel snapshots
- Copy-on-write pixel buffers shared between layers, bounded and released with history
- Explicit editing target so painting into a mask cannot be mistaken for painting into pixels
- Existing Phase 1–7.1 behavior preserved: a plain single-layer document keeps the original render and export path
- Editable curves and selective-colour adjustment layers, including a keyboard-operable curve editor
- Bounded periodic local recovery snapshots with startup recovery and explicit discard
- Workflow schema 2 layer steps, deterministic fail-closed selectors, and planner-safe restrictions
- Batch rendering of `.photoforge` projects without modifying their source files
- Sibling multi-selection and grouping in the Layers panel
- Deterministic row-band compositor parallelism across at most eight CPU threads
- Not implemented: pass-through groups, GPU acceleration, colour management, PSD support, procedural/neural layers

## Phase 9 — RAW development and high-precision colour (0.9.0, delivered)

- Camera DNG decoding written against the published specification, with no
  third-party decoder linked and no native library packaged
- Bounded TIFF reader, lossless-JPEG (SOF3) decoder, and DNG sensor semantics
- Black-level normalisation per CFA position, and white balance applied on the
  CFA before interpolation
- Bilinear and Malvar-He-Cutler demosaicing, the latter measured against ground
  truth rather than asserted
- Camera colour matrix to linear sRGB, with an absent matrix reported as such
- Scene-linear development with no clipping before the display transform, so
  highlight recovery is real
- Source-backed RAW projects: the file, its hash, and its development
  parameters, with missing and changed sources detected and relink verified
- Preview decimation that preserves CFA phase, so editing stays interactive on
  a 24-megapixel file
- True 16-bit PNG export at full sensor resolution, proven to carry precision
  8 bits cannot represent
- Not implemented: formats other than DNG (a licensing constraint), sensors
  above 40 megapixels, float compositing, ICC profiles, Display P3, Adobe RGB,
  and batch RAW development

## Phase 13 — Semantic layers and smart objects (0.13.0, source-complete)

- Editable text and vector shape layers share one float-coverage rasteriser and
  remain parametric until an explicit rasterize action
- Smart-object instances reference bounded, editable source layer stacks held
  in a document registry; shared instances update together and Independent copy
  creates a separate source dependency graph
- Native-size source compositing supports nested sources, masks, transforms,
  tiled output, streamed output, deterministic cache invalidation, and bounded
  full-frame CPU fallback for each required source
- Project save/load, recovery, undo/redo, thumbnails, merge, flatten, and export
  preserve or explicitly bake semantic content with fail-closed validation
- Linked sources record a local path, size, and SHA-256 digest; links are never
  followed on open and are checked/relinked only by explicit user actions
- Blank high-precision documents and semantic authoring controls are available
  from the desktop UI

The Rust and frontend source gates pass, including svelte-check, Vitest and the
Vite production build. Native packaged GUI/DPI/UAC/signing acceptance is not
claimed; see [Phase 13 results](phase-13-results.md).

## Later, optional AI work

- Lazy-loaded quality assessment, blur/noise estimation, OCR cleanup, super-resolution, and old-photo restoration
- Explicit model downloads with source, size, memory disclosure, and user approval
- Visible labels for any generated or reconstructed detail

As of Phase 6, no OpenAI/cloud planning, neural restoration, ONNX inference, Real-ESRGAN, OCR, chatbot, Python runtime, model download, or executable plugin integration was implemented; later phases changed some of that (see Phases 12 to 14), and none of the cloud, Python or model-download items. Perspective correction and batch processing are deterministic local Phase 6 features. Ollama remains a text-to-validated-plan adapter only; it receives no image, mask, or pixel/tool authority.

## Phase 14 — Extensibility, resources and oversized images (0.14.0)

- An operation registry (22 operations) and a transaction engine in Rust that every
  layer-document change goes through; macros with conditions and a recorder; batch and
  the planner on the same evaluator
- A sandboxed WebAssembly plugin runtime (Wasmtime, no WASI, default authority none),
  a `.photoforge-plugin` package read as hostile input, a plugin manager, filters that
  tile with a declared and verified reach, commands, panels and tools, and documents
  that open and report when a plugin is missing
- A memory budget derived from the machine and settable by the person, admission
  planning that offers a region or a reduced copy of an oversized source, region
  decoding by format with its real costs stated, and documents that remember where a
  region came from
- Not implemented, and said so in the product: out-of-core editing, a plugin marketplace
  or any plugin download, native plugin libraries, scripting, signing, generative fill,
  cloud AI. See [phase-14-results.md](phase-14-results.md).
