# Roadmap

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

## Phase 2 — Restoration tools (planned)

- Automatic white balance
- Adaptive local contrast / CLAHE
- Denoising and JPEG artifact reduction
- Edge-aware sharpening and conservative motion-blur restoration
- Document enhancement, perspective correction, auto-crop, and readability mode
- Clear language distinguishing restoration from reconstructed content

## Phase 3 — Guided editing (planned)

- Deterministic rule-based natural-language interpreter
- Strict JSON edit-plan schema and proposal review UI
- Approval before applying a plan
- Optional adapters for local runtimes without coupling core code to Ollama

## Later, optional AI work

- Lazy-loaded quality assessment, blur/noise estimation, OCR cleanup, super-resolution, and old-photo restoration
- Explicit model downloads with source, size, memory disclosure, and user approval
- Visible labels for any generated or reconstructed detail

No neural restoration or LLM integration is implemented in the current deliverable.
