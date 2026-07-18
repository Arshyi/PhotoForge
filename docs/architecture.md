# Architecture

## Goals

PhotoForge is local-first, non-destructive, modular, and conservative with memory. The desktop boundary exposes a small set of typed Tauri commands; the webview never receives unrestricted filesystem or shell access.

## Layers

```text
Svelte presentation
  └─ typed Tauri commands: open_image, analyze_image, generate_edit_plan,
       validate_guided_plan, render_preview, export_image
       └─ application state and use-case orchestration
            ├─ domain: operations, plans, rule-based planner, validation, pipeline, results
            ├─ image_processing: deterministic pixel algorithms
            └─ infrastructure: decoding, preview encoding, export safety
```

### Domain

`src-tauri/src/domain` owns `EditOperation`, `EditPlan`, `EditPlanner`, `RuleBasedPlanner`, plan validation, `EditPipeline`, image metadata, analysis heuristics, and command result types. Operations use Serde's tagged representation, so TypeScript and Rust exchange JSON such as `{ "type": "auto_white_balance", "strength": 0.7 }`. Ordinary and restoration operations share one ordered pipeline and validate every parameter before processing.

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

Preview, analysis, planning, and export use independent bounded gates and request generations. Planning reads only the cached analysis and user request; it never receives decoded pixels or paths.

The source is decoded once. Tauri commands clone only reference-counted handles before moving CPU work to a blocking worker, keeping the UI and async runtime responsive.

### Image processing

`src-tauri/src/image_processing` is independent of Tauri and filesystem code. `processor` applies ordered operations, `restoration` contains deterministic restoration algorithms, and `analysis` calculates lightweight heuristics. This keeps algorithms unit-testable without changing the application boundary.

### Infrastructure

`src-tauri/src/infrastructure` handles canonical paths, format detection, decoding, PNG preview encoding, and safe full-resolution export. It also contains a JSON preferences model. PhotoForge does not depend on Ollama, ONNX, or any model runtime.

### Presentation

Svelte components under `src/lib/components` implement the toolbar, Guided Edit panel, ordinary controls, Restoration panel, Analysis panel, image stage, and status bar. TypeScript types mirror the Rust operation and plan schemas. The guided inspector exposes summary, confidence, warnings, operations, explanations, removal, ordering, strength controls, Apply, and Cancel. Applying a validated plan is one undoable history commit and replaces the visible pipeline with exactly the reviewed operations.

## Preview transport and stale-result protection

The Rust session retains decoded pixels. Interactive operations run against the cached downscaled image, which is PNG-encoded and returned as a data URL. This avoids sending uncompressed RGBA buffers over IPC while keeping the preview entirely local.

Open and preview requests record separate monotonically increasing generations. A new open invalidates pending preview work immediately. The frontend keeps at most one preview IPC request in flight and retains only the newest queued state. Rust independently serializes preview CPU work, discards superseded queued requests before processing, and rechecks both document and request identifiers before encoding. This two-sided protocol prevents a stale open, the debounce window, or a slow old preview from replacing newer state.

Export uses a separate single-job gate and always rebuilds the pipeline from the cached full-resolution source. An empty pipeline exports the shared source without an unnecessary full-image clone.

## Analysis lifecycle

Analysis runs once per open document against the cached preview on a blocking worker. An analysis gate bounds CPU work independently from preview/export, and request/document generations prevent stale observations from appearing after a new open. A completed result is cached in the editor session so UI rerenders do not recalculate it. Analysis never changes the edit pipeline.

The metrics are deterministic heuristics—not definitive diagnoses—and the frontend uses cautious language such as “appears” and “possible.”

## Guided planning lifecycle

`generate_edit_plan` accepts a bounded text request plus document/request identifiers. It reads the already cached analysis from the active session and delegates to the pure `RuleBasedPlanner` through the `EditPlanner` trait. Rules recognize conservative lighting, color-cast, contrast, noise, JPEG, softness, document, scan, and uneven-lighting phrases. Analysis suppresses contradictory recommendations—for example, an already bright image does not receive a brightness increase.

The result is a proposal only. The frontend ignores stale generations, optionally stores at most 25 request strings in local WebView storage, and presents the typed plan. After the user edits the plan, `validate_guided_plan` revalidates the complete proposal at the Rust trust boundary. Only the returned operations can be committed to existing history and sent through the unchanged preview/export processor. The planner never invokes image-processing functions.

## Security boundaries

- Only native open/save dialogs choose paths.
- File formats are detected from content and restricted to PNG, JPEG, and WebP.
- Dimensions and encoded file size are checked before full decode.
- Inputs are limited to 40 million pixels, 20,000 pixels per dimension, 256 MiB decoder allocation, and 750 MiB encoded size.
- Output paths must be absolute, have an allowed image extension, and differ from the canonical input path.
- Pixel-processing commands accept typed operations, never command strings. Guided text is accepted only by the bounded rule matcher and can produce only the typed plan schema.
- Edited plans are revalidated in Rust before they can enter history or preview processing.
- No shell, arbitrary filesystem API, remote endpoint, model tool, or auto-apply analysis action is exposed.

The settings dialog makes the application shell inert while open, traps keyboard focus, returns focus to the settings button on close, and supports Escape. Controls that require a document are removed from keyboard interaction while unavailable.

## Extension points

- New deterministic edits: add a domain variant, validation, processor implementation, TypeScript mirror, and tests.
- Restoration: add another validated tagged operation and implement it inside the focused restoration processor.
- Guided planners: implement the domain `EditPlanner` trait and return the same validated `EditPlan`; an optional future local adapter would not change the restoration engine or approval boundary.
- Storage: move preferences serialization behind an application-directory repository without changing the domain or UI.
