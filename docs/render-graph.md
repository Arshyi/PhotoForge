# Phase 11 render graph

PhotoForge does not persist a render graph in a project and does not expose a
shader or plugin graph API. The graph is derived at render time from the
validated `LayerDocument`, the immutable `PixelSource`, the render scale and
the ordered document operations. That keeps the editable project authoritative
and makes deleting a cache safe.

The CPU reference renderer in `layers::linear` is the semantic oracle. The
Phase 11 `layers::tiled::Plan` is its region-addressable execution plan. Both
walk the same layer tree, apply the same transforms, masks, alpha equations and
blend modes, and check the same cancellation flag. A cache key is derived from
the layer JSON, source-content fingerprints, canvas, scale, tile size and halo;
it is never a revision number supplied by the webview.

## Node contract

| Node | Input dependency | Output | Locality | Cacheable | Cancellation |
| --- | --- | --- | --- | --- | --- |
| Source | immutable pixel identifier and source bytes | straight linear RGBA f32 (or legacy encoded RGBA8) | source/preparation | yes, by content fingerprint | between source jobs |
| Pixel layer | source, transform, mask, opacity, blend | canvas region | geometric + tile-local | yes | per output row |
| Isolated group | child composite and group transform/mask | bounded group region | tile-local after inverse bounds | yes | per output row/depth |
| Pass-through group | current backdrop plus children | canvas region | tile-local, with backdrop dependency | yes, conservatively | per output row/depth |
| Adjustment layer | current backdrop and operation parameters | canvas region | tile-local or halo-dependent; global operations force fallback | yes when the dependency key matches | per operation/row |
| Document operation | composited canvas | final document buffer | may be global or geometry-changing | not shared with layer tiles | per operation/row |
| Export sink | final rows/bands | PNG/JPEG/WebP bytes | row-streamed where encoder permits | no | per band |

The plan records the output canvas, tile grid, operation halo, whether the graph
is globally dependent, the worker bound and cache hits. `TiledStats` reports
these facts to benchmark callers; the diagnostics command reports the stable
engine settings and cache/GPU counters rather than a noisy trace stream.

## Locality classification

Operations are classified before allocation:

- `TileLocal`: point edits, masks, simple blends, opacity and most tone/color
  controls read only the pixel they write.
- `HaloDependent`: blur, sharpen, denoise, local contrast, uneven lighting,
  mild deblur, edge-aware sharpen and decontamination read a bounded
  neighbourhood. The plan grows the requested tile by the operation-derived
  radius and crops the result back to the valid tile.
- `Global`: automatic white balance, origin-anchored deblock, and any operation
  whose neighbourhood exceeds the safety bound need a whole-image context.
  The plan calls the full-frame oracle and marks `fellBackToFullFrame=true`; it
  never silently normalises each tile independently.
- `Geometric`: transforms map destination coordinates back to source space.
  The inverse mapping is evaluated in document coordinates, including across
  tile boundaries and off-canvas areas.
- `Source`: RAW decode, promotion from encoded pixels and other preparation
  stages happen before the graph can request a tile. RAW source state is not
  re-decoded for every tile.

Histogram and automatic statistics are global reductions. They are not faked
as independent tile results. Document-level crop/perspective/lens operations
remain outside the layer-tile plan and use the existing checked pipeline.

## Revision and fallback rules

An open/replaced document clears the shared cache. A late worker can still
finish safely because its key includes the exact source content and layer
state; no result is published to a newer request. A missing source, invalid
region, cancelled request, failed device or unsupported GPU operation returns
through the normal error/fallback path. The CPU reference remains available in
every build.
