# Performance

## Phase 1 design targets

- Near-zero idle CPU: no polling loop, animation is limited to an active processing indicator, and no model loads at startup.
- Fast startup: the application loads a small Svelte bundle and native Rust commands; all image decode work begins only after open.
- Responsive adjustment: controls debounce for 120 ms and use a decoded preview capped at 1600 pixels.
- Responsive UI: decode and pipeline execution run on Tauri blocking workers rather than the webview thread.
- Bounded input: encoded images above 750 MiB and decoded images above 120 megapixels are rejected before full processing.
- Reduced copying: full and preview images are retained behind `Arc`; full resolution is processed only for export.

## Measurement

The status bar reports end-to-end processing time for each preview and export using Rust's monotonic `Instant`. This is the useful measurement for a user's actual image, hardware, pipeline order, and output format.

Formal representative image benchmarks have not yet been recorded in this repository. Build and test timings are environment-dependent and are not substitutes for editor latency. Before a tagged release, record cold open, one-operation preview, five-operation preview, and PNG/JPEG export for 12 MP, 24 MP, and the 120 MP boundary on baseline hardware.

The packaged Windows release completed a startup smoke test on July 18, 2026: the process remained alive and Windows reported it as responsive after five seconds. This verifies startup viability, not a formal startup-duration measurement.

## Known tradeoffs

- Operations currently allocate an RGBA output per pipeline step for clarity and correctness.
- Preview transport PNG-encodes the result, reducing IPC size at the cost of encoding time.
- Unsharp masking creates one temporary blurred image.
- ICC/EXIF preservation and tiled processing are not in Phase 1.
