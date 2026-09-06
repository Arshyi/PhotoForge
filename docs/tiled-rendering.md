# Phase 11 tiled CPU rendering

The high-precision layer compositor now has a bounded CPU execution path. It
does not change the Phase 10 float semantics: straight-alpha linear-sRGB f32
buffers remain authoritative, composition and resampling remain premultiplied,
and the old full-frame renderer remains the correctness oracle and fallback.

## Tile geometry

Tiles use render-space (already-scaled document) coordinates. The default edge
is **256 px**; callers may use 64–2048 px. The default was selected from the
repository benchmark: 128 px paid too much scheduling/copy overhead, while
512 px repeated too much halo work. A `TileGrid` uses checked `u64` products for
tile indices and returns partial right/bottom tiles, including one-pixel edges.
Every output pixel is written exactly once; edge regions are clipped to the
canvas and are never left uninitialised.

Each request has a valid tile rectangle and, for neighbourhood work, an
expanded halo rectangle. Only the valid rectangle is copied to the output or
cache. A cache entry never contains a halo, so halo size changes cannot make an
old overlap look like a new tile.

## Halo rules

The halo is derived from the implementation, not the control label:

| Operation | Dependency used by the plan |
| --- | --- |
| Gaussian blur | `ceil(3 × sigma) + 1` for the bilinear taps |
| Sharpen | the fixed 1.2-sigma blur reach plus one tap |
| Edge-aware sharpen / mild deblur | their validated sigma reach plus one tap |
| Denoise | the chroma radius, which is wider than the luma radius |
| Dust & hot pixels | one pixel, the ring it reads |
| Deconvolution | twice the kernel reach per iteration, so usually global |
| Local contrast | half the validated local window, at least one |
| Uneven lighting | the validated radius |
| Decontaminate Colors | the validated integer radius |
| Shape and text layers | none — they rasterise into the rectangle and read no neighbouring pixels |

Sequential neighbourhood adjustments accumulate their dependency radius.
Independent branches in an isolated group take the maximum instead, because a
sibling does not read another sibling's intermediate. A halo larger than 64
pixels, an origin-anchored filter, or a global statistic is deliberately marked
global and rendered once by the oracle. Clamping an unsafe halo would create
seams, so the renderer refuses that shortcut.

The seam tests compare blurred, sharpened and sequential-neighbourhood output
against the full-frame renderer at several tile sizes and thread counts. They
also exercise transformed layers, pass-through groups, transparency and
partial-edge tiles.

## Scheduler and cancellation

The scheduler uses a fixed worker pool capped at eight threads. Whole-frame
renders claim horizontal tile-row bands; streaming exports keep rows ordered and
parallelise tiles inside one band. No OS thread is created per tile, and nested
operation parallelism is suppressed while a tile worker is active. This is a
per-render ceiling; the existing application job gate still serialises large
decode/render/export jobs and batch remains sequential.

Cancellation is checked before each tile, between layer rows and before a band
is emitted. A cancelled or stale render returns `RenderCancelled`/a stale
request result; partially filled output is never published as a successful
image.

`peak_region_bytes` is the largest single expanded tile/intermediate reported
by the plan. It is not a process working-set measurement and does not include
the full output returned by the non-streaming API, the webview, allocator
overhead or several in-flight worker tiles. The streaming API is the bounded
memory route for direct exports.

## What is and is not tiled

Layer compositing, masks, transforms, isolated groups and pass-through groups
are evaluated per requested region. A high-precision render with no global
dependency can therefore avoid full-frame group/adjustment intermediates. The
non-streaming API still materialises its final `FloatImage` because callers
need an image object.

Document-level operations applied after the layer composite, global reductions,
and operations whose coordinate system cannot be reproduced on a sub-region
fall back to the full-frame oracle. The current command path streams a direct
linear-document PNG export with no document operations; PNG/JPEG/WebP exports
that require later operations retain the existing full-frame encoder contract.

## Equivalence evidence

Rust tests cover ordinary and RAW-backed sources, masks, all blend modes,
opacity, transformed and partially off-canvas layers, nested/isolated and
pass-through groups, sequential halos, cancellation, cache reuse and
streaming. Since 0.13.0 the random layer-tree generator also produces vector
shapes and text, so the equivalence and staleness properties cover rasterised
content as well as sampled content. Doing that found a real cache defect: see
[vector-layers.md](vector-layers.md#caching) for the pass-through group bug it
exposed. The `tiled_pipeline` integration suite asserts the streamed PNG is
the same file as the whole-frame export and that changing tile size does not
change the result. The CPU full-frame path remains in-tree and is used as the
comparison reference rather than comparing a new implementation with itself.
