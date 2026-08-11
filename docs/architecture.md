# Architecture

## Phase 7 and 7.1 selections and masks boundary

PhotoForge 0.7.0 added selections as a separate, deterministic domain under `src-tauri/src/mask`; 0.7.1 completes its geometry, progress, preview, persistence, and session boundaries. A mask is an 8-bit coverage bitmap with checked dimensions, bounded geometry, and explicit composition semantics. Rectangle, ellipse, polygon, freehand, brush, magic-wand, and color-range tools all produce the same representation. Feathering, morphology, cleanup, border creation, and classical edge refinement transform only coverage values; none reconstruct image content or invoke a model. Opt-in color decontamination is deliberately separate: it is a deterministic, masked-only image operation that changes RGB on partial-coverage edge pixels while preserving alpha.

`EditOperation::Masked` embeds an immutable mask snapshot around one mask-capable edit. Ordinary adjustments render through the existing deterministic path and coverage-blend with the unmodified input while preserving alpha. `decontaminate_colors` is rejected unless it is inside this wrapper; when enabled, its bounded neighborhood sampler replaces RGB spill only at partially covered edges and leaves all alpha values unchanged. Geometry-changing operations cannot be wrapped and nested wrappers are rejected. Every embedded snapshot must exactly match its full-resolution pipeline stage. Preview preparation separately tracks full and bounded-preview stage dimensions and creates an ephemeral bilinear preview mask; the canonical workflow snapshot is never resized or rewritten by preview. Workflow replay therefore does not depend on mutable UI selection state.

Tauri mask commands validate all dimensions, payloads, local paths, checksums, operation parameters, document identifiers, and request generations before work begins. Long operations use cooperative cancellation, stale-result checks, and request-scoped work-unit progress. JSON and PNG mask import/export run on blocking workers behind the same request boundary; byte, row, and pixel phases report real work, while parser/codec phases without granular callbacks are labeled without inventing local work units. Exports write a securely created sibling temporary file, flush it, and atomically replace the chosen destination only after successful completion. Mask files and grayscale PNGs use user-selected absolute local paths; the webview receives no general filesystem capability.

The Svelte presentation owns interactive tool state, named masks, the active mask, bounded selection history, per-document local session persistence, modifier keys, lazy thumbnails, progress presentation, and overlay rendering. `ImageStage` maps pointer coordinates through the displayed image bounds into current image-stage space, so zoom and high-DPI display scaling do not change selection coordinates. It resolves optional pen pressure into explicit diameter/opacity samples; Rust receives deterministic resolved values rather than live hardware state. The Refine dialog may generate a bounded representative comparison canvas in the presentation, but canonical pipeline pixel generation and trust-boundary validation remain native Rust responsibilities.

### Geometry transaction boundary

`mask::transform` models crop, exact quarter-turn rotation, horizontal reflection, straighten, perspective, and lens correction as a checked `GeometryChain`. The chain records dimensions for every stage. Exact crop/quarter-turn/reflection suffixes use discrete coverage mapping; general rebases map a destination pixel through the new chain to original image space, through the old chain to the source stage, then bilinearly sample once. Pixels outside the valid source domain become zero. Perspective and lens inversions are iteration-bounded and reject non-finite, folded, near-singular, or non-convergent mappings. Lens coverage follows the image operation's normalized center/green-and-alpha radial distortion map. Vignetting is intensity-only and chromatic aberration uses channel-specific offsets, so neither changes the coordinate map of a single-channel mask. Distortion is restricted to the invertible `-0.16…1` range; vignetting and chromatic aberration remain `-1…1`.

The `remap_selection_masks` command accepts a bounded batch of keyed snapshots with explicit old/new stages. It validates all geometry, dimensions, key uniqueness, item count, aggregate allocation, cancellation, and work units before returning. The frontend reconciles every result key and performs one compound edit-history/selection-history commit. Active, named, and persistent embedded workflow masks therefore move together, and any missing item or stale document leaves both states unchanged.

### Presentation caches and session migration

Named-mask thumbnails are area-averaged grayscale coverage generated only when visible or idle. Their content key is checksum plus source/target dimensions. A shared LRU retains at most 96 entries and approximately 2 MiB; overlay coverage uses a separate bounded cache. Neither cache writes to disk or contacts a service.

Selection-session schema 2 stores current-stage dimensions, the canonical geometry-operation list, and its fingerprint in addition to Phase 7 state. Loading validates every mask against that stage. The loader reads schema-2 first and can migrate a valid schema-1 Phase 7 record in memory with empty geometry and new settings at conservative defaults. It writes only schema 2 and rejects incompatible future or incoherent records rather than guessing.

## Phase 6 professional workflow boundary

PhotoForge 0.6.0 extends `EditOperation` with validated curves, levels, point balance, crop, straighten, perspective, lens, HSL, temperature/tint, and selective-color variants. The deterministic processor remains the single pixel boundary. Interactive preview, undo/redo, workflow replay, export profiles, and batch processing all consume the same ordered operation vector.

`domain::professional` owns the versioned workflow, histogram, pixel, batch, workspace, shortcut, and export-profile contracts. `image_processing::professional` owns deterministic pixel transforms; `image_processing::inspection` owns histogram and pixel sampling. `infrastructure::workflow_io` and `infrastructure::metadata` handle bounded JSON/EXIF parsing. `application::batch` discovers local inputs and schedules bounded workers. `commands::professional` is the Tauri validation and cancellation boundary.

Batch state and cancellation are process-local atomics/mutexes. No worker holds the editor session lock while decoding or processing. Histogram work uses the bounded 1600-pixel preview and a stale-request gate. Full-resolution source images remain immutable `Arc<DynamicImage>` values and are decoded per batch item only while that item is active.

## Goals

PhotoForge is local-first, non-destructive, modular, and conservative with memory. The desktop boundary exposes a small set of typed Tauri commands; the webview never receives unrestricted filesystem or shell access.

## Layers

```text
Svelte presentation
  └─ typed Tauri commands: editor, rule/Ollama planning, component registry, diagnostics
       └─ application state and use-case orchestration
            ├─ components: registries, factories, planners, Ollama HTTP/validation, restoration engines, timeout
            ├─ domain: operations, plans, component capabilities, manifests, validation
            ├─ mask: coverage bitmaps, rasterization, transforms, persistence, diagnostics
            ├─ image_processing: deterministic pixel algorithms and mask-aware composition
            └─ infrastructure: decoding, export safety, manifest/model metadata discovery
```

### Domain

`src-tauri/src/domain` owns `EditOperation`, `EditPlan`, `EditPlanner`, `RestorationEngine`, typed component capabilities/providers/configuration/diagnostics, plugin manifests, plan validation, `EditPipeline`, image metadata, analysis heuristics, and command result types. Operations use Serde's tagged representation, so TypeScript and Rust exchange JSON such as `{ "type": "auto_white_balance", "strength": 0.7 }`. Ordinary and restoration operations share one ordered pipeline and validate every parameter before processing.

`EditPlan` contains a summary, bounded heuristic confidence, concise warnings, `Vec<EditOperation>`, and one human explanation per operation. Plan validation rejects empty/oversized plans, non-finite confidence or operation values, unknown/unsupported operations, duplicates, grayscale/saturation conflicts, and unsupported cleanup/detail ordering. The plan schema contains no code, command, path, plugin, model prompt, or pixel payload.

`EditPipeline` maintains the current ordered operations plus undo and redo snapshots. Both Rust and TypeScript cap history at 200 snapshots. The presentation coalesces rapid events from one slider gesture into a single undo step; Rust still validates every pipeline at the trust boundary.

### Application

`src-tauri/src/application` holds one open editor session. A session keeps:

- canonical original path;
- decoded full-resolution image behind `Arc`;
- decoded preview capped at 1600 pixels;
- immutable image metadata.
- a monotonically increasing document identifier.
- an optional cached quality analysis for that document.
- the component registry and its persisted local configuration.

Preview, analysis, planning, and export use independent bounded gates and request generations. Planning reads only the cached analysis and user request; it never receives decoded pixels or paths.

The source is decoded once. Tauri commands clone only reference-counted handles before moving CPU work to a blocking worker, keeping the UI and async runtime responsive.

### Image processing

`src-tauri/src/image_processing` is independent of Tauri and filesystem code. `processor` applies ordered operations, `restoration` contains deterministic restoration algorithms, `decontaminate` contains the bounded partial-edge RGB correction, and `analysis` calculates lightweight heuristics. This keeps algorithms unit-testable without changing the application boundary.

### Infrastructure

`src-tauri/src/infrastructure` handles canonical paths, format detection, decoding, PNG preview encoding, safe full-resolution export, bounded plugin-manifest scans, and local engine-model metadata discovery. PhotoForge bundles no Ollama, ONNX, or model runtime. The optional Ollama adapter communicates with a separately managed loopback service through a bounded HTTP client in `components`.

### Presentation

Svelte components under `src/lib/components` implement the toolbar, Guided Edit panel, ordinary controls, Restoration panel, Analysis panel, image stage, status bar, Components settings, Diagnostics settings, and Local AI Privacy page. TypeScript types mirror the Rust operation, plan, registration, Ollama model/validation/comparison, configuration, plugin, and diagnostic schemas. The guided inspector exposes provider selection, summary, confidence, warnings, operations, explanations, removal, ordering, strength controls, raw/validated JSON inspection, Apply, and Cancel. Applying a validated plan is one undoable history commit and replaces the visible pipeline with exactly the reviewed operations.

### Optional components

`src-tauri/src/components` contains the registry, factories, timeout helper, performance diagnostic, planner implementations, Ollama HTTP/schema boundary, and restoration-engine implementations. `RulePlanner` and `DeterministicEngine` preserve the Phase 3 paths. Ollama is an installed optional adapter whose network work is exposed only through cancellable async commands; OpenAI and ONNX/Real-ESRGAN/future types remain safe placeholders. See [component-architecture.md](component-architecture.md) and [ollama-provider.md](ollama-provider.md).

## Preview transport and stale-result protection

The Rust session retains decoded pixels. Interactive operations run against the cached downscaled image, which is PNG-encoded and returned as a data URL. This avoids sending uncompressed RGBA buffers over IPC while keeping the preview entirely local.

Open and preview requests record separate monotonically increasing generations. A new open invalidates pending preview work immediately. The frontend keeps at most one preview IPC request in flight and retains only the newest queued state. Rust independently serializes preview CPU work, discards superseded queued requests before processing, and rechecks both document and request identifiers before encoding. This two-sided protocol prevents a stale open, the debounce window, or a slow old preview from replacing newer state.

Export uses a separate single-job gate and always rebuilds the pipeline from the cached full-resolution source. An empty pipeline exports the shared source without an unnecessary full-image clone.

## Analysis lifecycle

Analysis runs once per open document against the cached preview on a blocking worker. An analysis gate bounds CPU work independently from preview/export, and request/document generations prevent stale observations from appearing after a new open. A completed result is cached in the editor session so UI rerenders do not recalculate it. Analysis never changes the edit pipeline.

The metrics are deterministic heuristics—not definitive diagnoses—and the frontend uses cautious language such as “appears” and “possible.”

## Guided planning lifecycle

`generate_edit_plan` accepts a bounded text request plus document/request identifiers. It reads the already cached analysis from the active session and delegates to the pure `RuleBasedPlanner` through the `EditPlanner` trait. Rules recognize conservative lighting, color-cast, contrast, noise, JPEG, softness, document, scan, and uneven-lighting phrases. Analysis suppresses contradictory recommendations—for example, an already bright image does not receive a brightness increase.

The result is a proposal only. The frontend ignores stale generations, optionally stores at most 25 provider-tagged requests in local WebView storage, and presents the typed plan. After the user edits the plan, `validate_guided_plan` revalidates the complete proposal at the Rust trust boundary. Only the returned operations can be committed to existing history and sent through the unchanged preview/export processor. No planner invokes image-processing functions.

### Optional Ollama lifecycle

`test_ollama_connection`, `refresh_ollama_models`, `generate_ollama_plan`, and `compare_planners` are the only commands that can contact Ollama, and each requires a direct user action. The endpoint validator accepts HTTP loopback hosts only. The HTTP client disables proxies, redirects, and retries, applies connection/total timeouts, bounds streamed responses, and validates UTF-8.

Generation races the request against document/request cancellation. The deterministic prompt contains the user request, approved scalar analysis, supported operations, parameter ranges, and JSON schema only. A dedicated deny-unknown-fields wire type converts accepted fields into existing operations, generates explanations locally, and reuses `validate_edit_plan`. Rejected responses remain inspectable but cannot be applied. See [ollama-provider.md](ollama-provider.md).

## Security boundaries

- Only native open/save dialogs choose paths.
- File formats are detected from content and restricted to PNG, JPEG, and WebP.
- Dimensions and encoded file size are checked before full decode.
- Inputs are limited to 40 million pixels, 20,000 pixels per dimension, 256 MiB decoder allocation, and 750 MiB encoded size.
- Output paths must be absolute, have an allowed image extension, and differ from the canonical input path.
- Pixel-processing commands accept typed operations, never command strings. Guided text is accepted only by the bounded rule matcher and can produce only the typed plan schema.
- Edited plans are revalidated in Rust before they can enter history or preview processing.
- No shell, arbitrary filesystem API, model tool, or auto-apply analysis action is exposed. Component settings accept bounded local paths and loopback-only endpoint text; no command connects automatically. Ollama HTTP disables redirects and proxy discovery.
- Plugin scanning validates bounded JSON metadata only and always reports execution disabled.
- Model discovery reads local directory entries and file metadata only; it never loads model contents or inference code.

The settings dialog makes the application shell inert while open, traps keyboard focus, returns focus to the settings button on close, and supports Escape. Controls that require a document are removed from keyboard interaction while unavailable.

## Extension points

- New deterministic edits: add a domain variant, validation, processor implementation, TypeScript mirror, and tests.
- New selection algorithms: accept or return `MaskBitmap`, validate bounded inputs, preserve 8-bit coverage semantics, add cancellation where work is long, and expose only a typed command.
- Restoration: add another validated tagged operation and implement it inside the focused restoration processor.
- Guided planners: implement `EditPlanner` and return the same validated `EditPlan`; the approval boundary remains unchanged.
- Restoration engines: implement `RestorationEngine` while preserving typed operation validation and export safety.
- Provider registration: extend the factory and truthful typed registry metadata, then add bounded initialization/unload/failure tests.
