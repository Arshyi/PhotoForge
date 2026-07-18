# Performance

## Phase 1 design targets

- Near-zero idle CPU: no polling loop, animation is limited to an active processing indicator, and no model loads at startup.
- Fast startup: the application loads a small Svelte bundle and native Rust commands; all image decode work begins only after open.
- Responsive adjustment: controls debounce for 120 ms and use a decoded preview capped at 1600 pixels.
- Responsive UI: decode and pipeline execution run on Tauri blocking workers rather than the webview thread.
- Bounded input: encoded images above 750 MiB, dimensions above 20,000 pixels, decoder allocation above 256 MiB, or images above 40 megapixels are rejected before full processing.
- Reduced copying: full and preview images are retained behind `Arc`; full resolution is processed only for export.
- Bounded work: one preview and one export may consume CPU at a time; superseded preview states are coalesced before Rust processing.

## Measurement

The status bar reports end-to-end processing time for each preview and export using Rust's monotonic `Instant`. This is the useful measurement for a user's actual image, hardware, pipeline order, and output format.

Phase 1.1 records packaged-app observations and a 24 MP fixture workflow in `phase-1-1-results.md`. These are practical release observations on one Windows machine, not cross-machine performance guarantees.

The packaged Windows release completed a startup smoke test on July 18, 2026: the process remained alive and Windows reported it as responsive after five seconds. This verifies startup viability, not a formal startup-duration measurement.

## Known tradeoffs

- The source converts to RGBA once per non-empty pipeline, but each operation still allocates an output buffer for clarity and correctness.
- Preview transport PNG-encodes the result, reducing IPC size at the cost of encoding time.
- Unsharp masking creates one temporary blurred image.
- ICC/EXIF preservation and tiled processing are not in Phase 1.
- Full-resolution export is deliberately serialized and may take time near the 40 MP ceiling.
