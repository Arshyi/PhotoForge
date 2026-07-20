# PhotoForge

PhotoForge is a lightweight, privacy-first desktop photo restoration and enhancement tool. It processes PNG, JPEG, and WebP images locally with a typed, non-destructive edit pipeline and never uploads photos.

This repository contains the Phase 0 foundation through the Phase 6 professional editing and workflow system. The current version is **0.6.0**.

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

## Requirements

- Windows 10 or 11 with the Microsoft Edge WebView2 runtime
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
cargo clippy --manifest-path src-tauri/Cargo.toml --all-targets -- -D warnings
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

## Architecture and project notes

- [Architecture](docs/architecture.md)
- [Image processing](docs/image-processing.md)
- [Privacy](docs/privacy.md)
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

## Honest scope

PhotoForge 0.6.0 adds professional curves, levels, geometry, color, inspection, workflow, batch, workspace, shortcut, comparison, metadata, and export-profile systems. Rule Planner remains the default, optional Ollama remains a local planning adapter only, and the Deterministic Engine remains the only component that changes pixels. PhotoForge does not install or download models, send images to Ollama, execute model-supplied code, call cloud providers, execute plugins, generate missing image content, or reconstruct factual details that were never captured. OpenAI, ONNX, Real-ESRGAN, and future adapters remain unavailable placeholders. OCR, neural restoration, super-resolution, inpainting, and generative editing remain outside this release.

## License

No license has been selected yet. All rights are reserved until the repository owner adds one.
