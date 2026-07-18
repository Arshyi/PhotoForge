# PhotoForge

PhotoForge is a lightweight, privacy-first desktop photo restoration and enhancement tool. It processes PNG, JPEG, and WebP images locally with a typed, non-destructive edit pipeline and never uploads photos.

This repository contains the Phase 0 foundation, Phase 1 editor, and Phase 1.1 release-hardening fixes. The current version is **0.1.1**.

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
3. Use **Compare** to drag between images. Rotated comparisons switch to side-by-side views so neither image is distorted.
4. Undo with `Ctrl+Z`, redo with `Ctrl+Y` or `Ctrl+Shift+Z`, or reset the pipeline.
5. Select **Export** or press `Ctrl+S`. PhotoForge processes the original at full resolution and requires a destination different from the source file.

## Architecture and project notes

- [Architecture](docs/architecture.md)
- [Image processing](docs/image-processing.md)
- [Privacy](docs/privacy.md)
- [Performance](docs/performance.md)
- [Roadmap](docs/roadmap.md)
- [Phase checklist](docs/checklist.md)
- [Phase 1.1 audit](docs/phase-1-1-audit.md)
- [Phase 1.1 results](docs/phase-1-1-results.md)

## Honest scope

PhotoForge's sharpening and filters are deterministic image-processing operations. They can improve appearance and edge contrast, but they cannot recreate genuinely missing detail. Neural restoration and natural-language editing are intentionally outside Phase 1.

## License

No license has been selected yet. All rights are reserved until the repository owner adds one.
