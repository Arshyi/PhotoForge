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

## Later, optional AI work

- Lazy-loaded quality assessment, blur/noise estimation, OCR cleanup, super-resolution, and old-photo restoration
- Explicit model downloads with source, size, memory disclosure, and user approval
- Visible labels for any generated or reconstructed detail

No actual Ollama connection, OpenAI/cloud planning, neural restoration, ONNX inference, Real-ESRGAN, OCR, perspective correction, batch processing, chatbot, Python runtime, model download, or executable plugin integration is implemented in the current deliverable.
