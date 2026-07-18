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

`EditPipeline` maintains the current ordered operations plus undo and redo snapshots. The presentation has a matching tested history model so interactions remain immediate; Rust still validates every pipeline at the trust boundary.

### Application

`src-tauri/src/application` holds one open editor session. A session keeps:

- canonical original path;
- decoded full-resolution image behind `Arc`;
- decoded preview capped at 1600 pixels;
- immutable image metadata.

The source is decoded once. Tauri commands clone only reference-counted handles before moving CPU work to a blocking worker, keeping the UI and async runtime responsive.

### Image processing

`src-tauri/src/image_processing` is independent of Tauri and filesystem code. It accepts an image and ordered operations and returns a new image. This makes algorithms unit-testable and replaceable.

### Infrastructure

`src-tauri/src/infrastructure` handles canonical paths, format detection, decoding, PNG preview encoding, and safe full-resolution export. It also contains a JSON preferences model and the `EditPlanProvider` boundary reserved for a future constrained local instruction layer. PhotoForge does not depend on Ollama, ONNX, or any model runtime.

### Presentation

Svelte components under `src/lib/components` implement the toolbar, editor controls, image stage, and status bar. TypeScript types mirror the Rust operation schema. The frontend debounces controls and attaches increasing request IDs to preview work.

## Preview transport and stale-result protection

The Rust session retains decoded pixels. Interactive operations run against the cached downscaled image, which is PNG-encoded and returned as a data URL. This avoids sending uncompressed RGBA buffers over IPC while keeping the preview entirely local.

Each preview request records a monotonically increasing generation. Rust suppresses encoding when a completed result is no longer current, and the frontend independently checks the request ID before replacing the visible preview. Export always rebuilds the pipeline from the cached full-resolution source.

## Security boundaries

- Only native open/save dialogs choose paths.
- File formats are detected from content and restricted to PNG, JPEG, and WebP.
- Dimensions and encoded file size are checked before full decode.
- Output paths must be absolute, have an allowed image extension, and differ from the canonical input path.
- Commands accept typed operations, never command strings.
- No shell, arbitrary filesystem API, remote endpoint, or model tool is exposed.

## Extension points

- New deterministic edits: add a domain variant, validation, processor implementation, TypeScript mirror, and tests.
- Restoration: introduce a separate tagged `RestorationOperation` handled by a restoration processor.
- Guided edits: implement `EditPlanProvider`; validate returned operations before presenting a proposal for user approval.
- Storage: move preferences serialization behind an application-directory repository without changing the domain or UI.
