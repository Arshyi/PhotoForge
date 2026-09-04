# Phase 9 results — RAW and professional colour pipeline

This is an engineering checkpoint, not a 0.9.0 release claim. Work began only
after the local 0.8.2 freeze commit `9c4610e`.

## Implemented in this checkpoint

- bounded straight-alpha linear-sRGB `FloatImage` with finite-value and
  40-million-pixel guards;
- IEC sRGB decode/encode tests, signed/out-of-range intermediate handling, and
  8/16-bit boundary tests;
- deterministic development parameters for as-shot/custom/auto/normalised
  temperature-tint white balance, stop-based exposure, contrast, highlights,
  shadows, whites, and blacks;
- a validated non-destructive `raw_development` operation that applies those
  controls to raster previews through the linear-light boundary and participates
  in masks, adjustment layers, workflows, and ordinary undo/replay;
- bounded RGB/luminance histogram with explicit shadow/highlight clipping
  counters;
- direct 16-bit RGBA PNG encoding from float results, including an
  `export_developed_png16` command and a Professional workspace action;
- RAW extension/magic inspection, SHA-256 source hashing, DNG TIFF-header
  validation, and explicit decoder-unavailable status;
- serialisable RAW source/development contracts and optional camera metadata
  fields that do not make metadata mandatory for raster images; and
- a typed `inspect_raw` Tauri command and frontend invocation helper.

## Not yet implemented

- no RAW decoder or demosaic backend is bundled, so no camera RAW file is
  imported into the editor yet;
- no float layer pixel store or float compositor is enabled by default; legacy
  operations bridge through RGBA8 while `raw_development` keeps its own pass in
  linear `f32` until display/export;
- no ICC profile reader/transform or Display-P3/Adobe-RGB conversion is
  claimed;
- no RAW develop panel, non-destructive project persistence, RAW workflows,
  batch development, presets, or RAW-aware Ollama plan schema is wired;
- no automatic lens profile, chromatic-aberration profile, RAW noise model, or
  camera-calibrated Kelvin conversion exists; and
- camera RAW source-backed project persistence and demosaic are still absent;
  raster projects retain their 0.8.2 RGBA8/sRGB layer storage, while the new
  command can export the current operation pipeline as a genuine 16-bit PNG.

## Verification

The new Rust numerical, development-operation, and RAW-inspection tests run with
the existing Rust test suite: 746 Rust unit tests and 39 IPC/integration tests pass in the current
source. `cargo fmt` and Clippy remain required gates. The frontend Vite/Svelte
and Tauri packaging gates retain the 0.8.2 environment blocker recorded in
[`phase-8-results.md`](phase-8-results.md): esbuild cannot read the workspace
directory in the current sandbox, and the supported elevated retry was denied
by the host usage limit. No packaged 0.9.0 artifact, native GUI/DPI matrix,
elevated MSI lifecycle, or production signature is claimed.

The current unbundled Rust release executable also builds successfully (version
0.8.2 because Phase 9 is not release-ready). Its SHA-256 is
`36AFD2A343A36D053F0B30863A59108707FA4F778F7ACB32560F93991C2CB25B`; this is
not a Tauri installer or a 0.9.0 artifact. The companion `photoforge_lib.dll`
hash is `F6398E1B5189A7BA31D548C0F495C35E4BBBD065094BA75ED4CF1C460C005703`.

## Exit criteria for the next checkpoint

Phase 9 can move from foundation to an importable 0.9.0 preview only after a
decoder is selected and tested against redistributable fixtures, its license and
Windows build are recorded, and the source-backed project/layer/workflow/batch
boundaries are implemented with cancellation, memory, corruption, and metadata
privacy tests. Until then this document is intentionally marked incomplete.
