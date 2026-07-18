# Phase 0 and Phase 1 checklist

## Repository and architecture

- [x] Tauri 2 / Rust backend and Svelte 5 / TypeScript frontend
- [x] Domain, application, infrastructure, processing, commands, and presentation layers
- [x] Typed operations, results, and errors
- [x] Future model-provider interface without runtime dependency
- [x] Architecture, privacy, processing, performance, roadmap, and README documentation

## Editor

- [x] Native open picker
- [x] Desktop file drag-and-drop
- [x] PNG, JPEG, and WebP decode
- [x] Image preview and metadata
- [x] Brightness, contrast, saturation, gamma, grayscale, sepia
- [x] Reflection, rotation, Gaussian blur, and unsharp masking
- [x] Five ordinary typed pipeline presets
- [x] Undo, redo, reset
- [x] Before/after comparison
- [x] Zoom controls and fit action
- [x] Debounced downscaled previews
- [x] Full-resolution export and original overwrite prevention
- [x] Processing, loading, stale-result, timing, and operation-count indicators

## Verification

- [x] `cargo fmt --check`
- [x] `cargo clippy --all-targets -- -D warnings`
- [x] `cargo test` — 13 passed
- [x] `npm run check` — 0 errors and 0 warnings
- [x] `npm run test` — 5 passed
- [x] `npm run build`
- [x] Tauri package build — portable executable, NSIS setup, and MSI

The verification boxes are updated only after each command actually succeeds.
