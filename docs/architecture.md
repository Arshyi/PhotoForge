# Architecture

## Goals

PhotoForge is local-first, non-destructive, modular, and conservative with memory. The desktop boundary exposes a small set of typed Tauri commands; the webview never receives unrestricted filesystem or shell access.

## Layers

```text
Svelte presentation
  └─ typed Tauri commands: open_image, render_preview, export_image
       └─ application state and use-case orchestration
            ├─ domain: operations, pipeline, metadata, results, errors
            ├─ image_processing: deterministic pixel algorithms
            └─ infrastructure: decoding, preview encoding, export safety
```

### Domain

`src-tauri/src/domain` owns `EditOperation`, `EditPipeline`, image metadata, and command result types. Operations use Serde's tagged representation, so TypeScript and Rust exchange JSON such as `{ "type": "brightness", "amount": 0.12 }`. Every operation validates its parameter range before processing.

`EditPipeline` maintains the current ordered operations plus undo and redo snapshots. Both Rust and TypeScript cap history at 200 snapshots. The presentation coalesces rapid events from one slider gesture into a single undo step; Rust still validates every pipeline at the trust boundary.

### Application

`src-tauri/src/application` holds one open editor session. A session keeps:

- canonical original path;
- decoded full-resolution image behind `Arc`;
- decoded preview capped at 1600 pixels;
- immutable image metadata.
- a monotonically increasing document identifier.

The source is decoded once. Tauri commands clone only reference-counted handles before moving CPU work to a blocking worker, keeping the UI and async runtime responsive.

### Image processing

`src-tauri/src/image_processing` is independent of Tauri and filesystem code. It accepts an image and ordered operations and returns a new image. This makes algorithms unit-testable and replaceable.

### Infrastructure

`src-tauri/src/infrastructure` handles canonical paths, format detection, decoding, PNG preview encoding, and safe full-resolution export. It also contains a JSON preferences model and the `EditPlanProvider` boundary reserved for a future constrained local instruction layer. PhotoForge does not depend on Ollama, ONNX, or any model runtime.

### Presentation

Svelte components under `src/lib/components` implement the toolbar, editor controls, image stage, and status bar. TypeScript types mirror the Rust operation schema. The frontend debounces controls and attaches increasing request IDs to preview work.

## Preview transport and stale-result protection

The Rust session retains decoded pixels. Interactive operations run against the cached downscaled image, which is PNG-encoded and returned as a data URL. This avoids sending uncompressed RGBA buffers over IPC while keeping the preview entirely local.

Open and preview requests record separate monotonically increasing generations. A new open invalidates pending preview work immediately. The frontend keeps at most one preview IPC request in flight and retains only the newest queued state. Rust independently serializes preview CPU work, discards superseded queued requests before processing, and rechecks both document and request identifiers before encoding. This two-sided protocol prevents a stale open, the debounce window, or a slow old preview from replacing newer state.

Export uses a separate single-job gate and always rebuilds the pipeline from the cached full-resolution source. An empty pipeline exports the shared source without an unnecessary full-image clone.

## Security boundaries

- Only native open/save dialogs choose paths.
- File formats are detected from content and restricted to PNG, JPEG, and WebP.
- Dimensions and encoded file size are checked before full decode.
- Inputs are limited to 40 million pixels, 20,000 pixels per dimension, 256 MiB decoder allocation, and 750 MiB encoded size.
- Output paths must be absolute, have an allowed image extension, and differ from the canonical input path.
- Commands accept typed operations, never command strings.
- No shell, arbitrary filesystem API, remote endpoint, or model tool is exposed.

The settings dialog makes the application shell inert while open, traps keyboard focus, returns focus to the settings button on close, and supports Escape. Controls that require a document are removed from keyboard interaction while unavailable.

## Extension points

- New deterministic edits: add a domain variant, validation, processor implementation, TypeScript mirror, and tests.
- Restoration: introduce a separate tagged `RestorationOperation` handled by a restoration processor.
- Guided edits: implement `EditPlanProvider`; validate returned operations before presenting a proposal for user approval.
- Storage: move preferences serialization behind an application-directory repository without changing the domain or UI.
