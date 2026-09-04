# High-precision rendering

## Frozen baseline and audit

Before Phase 10 source changes, local HEAD, `origin/main`, and live GitHub main
were verified to equal `e7932f27e1f6ea54df5f24cf6c5229ad405e4d32` (0.9.0).
The checkout and `git diff --check` were clean. No force push was needed.

The following map was recorded before migration. It describes the 0.9.0 code,
not a claim that the migration is already complete.

| Boundary | 0.9.0 representation and loss | Phase 10 design |
| --- | --- | --- |
| DNG decode and development | bounded sensor samples to straight linear-sRGB `FloatImage` | retain these authoritative samples |
| Ordinary decode | `DynamicImage` can retain PNG16, but layer registration calls `to_rgba8` | typed encoded RGBA8 or linear RGBA f32; preserve PNG16 at import |
| RAW registration | `commands/raw.rs::register` and `open_raw_image` call `to_rgba8` | store developed float pixels directly |
| Immutable source store | `Arc<RgbaImage>` full and preview buffers, 1 GiB cache budget | shared typed immutable buffers and bounded previews, counted by actual bytes |
| Layer rendering | source sampling, masks, blend and source-over round to u8 per layer | explicit linear document renderer; float color and premultiplied interpolation |
| Pass-through groups | full RGBA8 backdrop copies and premultiplied crossfade | same tree/group semantics with float intermediates |
| Adjustment layers | encoded byte processor | explicit linear or encoded-float operations, no implicit byte bridge |
| Masks | u8 scalar coverage, not RGB | keep canonical mask format; convert coverage to float when sampling |
| Merge/flatten/rasterize | renderer output is registered as RGBA8 | preserve the selected document render format |
| Document-level operations | `apply_pipeline` converts to RGBA8 | route linear documents through the float operation pipeline |
| Preview and thumbnails | RGBA8/PNG, limited to 1600/256 pixels on longest edge | intentional sRGB display conversion, never authoritative source state |
| Histogram and pixel inspector | encoded-byte composite | histogram from float composite; inspection labels the display boundary |
| Project persistence/recovery | version 1, RGBA8 PNG entries | versioned typed float payloads and explicit render semantics; read version 1 unchanged |
| Final layered export | RGBA8 renderer and byte exporter | actual float layer composite to selected RGB output and PNG16 |
| Single-RAW PNG16 export | bypasses layers | retain existing command for compatibility; UI uses layered export for the document |
| Batch | ordinary loader or RGBA8 project composite | sequential memory-aware RAW/typed project rendering with existing cancellation/progress |

## Compatibility policy

Projects without explicit precision metadata retain the legacy encoded-sRGB
renderer and its existing blend/adjustment semantics. Opening a file does not
rewrite it. A linear document uses the same validated layer tree, visibility,
opacity, group, mask, transform and blend contracts. It must never silently
fall back to the legacy renderer because an operation is unsupported.

The working primaries are sRGB/D65; linear f32 can represent negative and
above-one RGB, including wide-gamut colors outside sRGB. Source profiles,
working primaries, display encoding and export profiles are distinct concepts.
The UI preview is encoded sRGB; Windows/WebView2 own monitor presentation.

## Alpha and operation policy

Authoritative `FloatImage` sources are straight-alpha linear RGB. Internal
compositing and resampling use explicitly premultiplied linear values. For
source-over with a blend function B, alpha is `as + ab * (1 - as)` and the
premultiplied color is `(1-ab)*as*Cs + ab*as*B(Cb,Cs) + (1-as)*ab*Cb`.
Pass-through opacity mixes the two complete premultiplied composites. Zero
alpha unassociates to transparent black; interpolation never mixes unassociated
colors from transparent neighbors.

RAW exposure/white balance and convolution operate in linear light. Traditional
curves, levels, HSL and selective-color controls operate in explicitly encoded
floating-point sRGB. Color-component blend modes retain documented W3C encoded
sRGB semantics through float transfer conversion, not naive HSL of linear RGB.
Legacy documents keep their original byte-exact path.

## Resource work

The 40 MP baseline is a security boundary. It will only be replaced where
checked byte accounting covers decoding, source storage, recursive compositor
scratch, masks, previews, operations and export. Admission of 45/61 MP is not
claimed merely because one dimension check accepts it. Full-frame operations
and global reductions need explicit budgets even if tile-local rendering later
becomes possible. History stores parameter trees and shared pixel identifiers,
not per-slider snapshots of float images.

See [phase-10-results.md](phase-10-results.md) for implementation and verification
status; this audit is not release evidence.

## Implemented admission and scheduling

The implemented source limit is 1 GiB / 16 bytes = 67,108,864 working pixels,
with a separate 20,000-pixel side limit. The immutable pixel store includes
full-resolution data, downscaled previews and Undo-reachable buffers in its
1 GiB/1,024-buffer ceiling. Small previews share the original Arc where possible.
Rendering estimates resident buffers, one encoded-source promotion, canvas,
deepest live group stack, operation scratch, decoded masks and output reserve.
All arithmetic is checked and a job estimate over 4 GiB is rejected.

This is an admission model, not a Windows working-set cap. WebView2, original
display copies, allocator overhead and a previous session temporarily retained
by an in-flight operation are additional process memory. The measured isolated
benchmarks do not certify every possible active-session overlap on low-RAM PCs.
No OS-level memory limiter or out-of-core fallback is claimed.

At 9504x6336, one float source plus preview costs 990,792,704 store bytes. A
second independent source (including Undo for full RAW re-development) exceeds
the 1 GiB store budget and is cleanly rejected. Parametric adjustments and
multiple layers sharing that source do not duplicate it. Large multi-source
projects therefore remain limited; a 61 MP source is not a promise of arbitrary
61 MP layer stacks. Projects themselves retain a 1 GiB file ceiling.

One full-frame decode/render/export worker is admitted at a time through a
shared cancellable gate. Batch uses one file worker; point adjustments use up
to eight fixed contiguous row bands for images >=262,144 pixels. The pixel
arithmetic/order is unchanged across bands. No parallel floating reduction is
introduced; histogram/global statistics stay ordered. Cancellation is checked
between operation/output rows and jobs; RAW decode itself is not interruptible
inside an existing segment. Some sampling/thumbnail jobs are separately bounded
but not covered by the global full-frame gate.

## Tile feasibility and deterministic reference

OperationLocality classifies point operations as tile-local, neighborhood
filters as halo-dependent, and auto white balance/global statistics as global.
Geometric operations also need source-coordinate mapping across tile edges.
PNG export is already row-streamed without a full-frame u16 intermediate.
Point work is processed in bounded row bands, but the compositor and filter
intermediates are still budgeted full-frame buffers. A complete tiled/out-of-core
renderer, halo scheduler and disk cache are deferred, not implied by the enum.

Same-build repeated renders use the same per-pixel order and are exact in tests.
Reference color matrices use f64; stored channels and compositor operations are
f32. Numerical tests use explicit tolerances (matrix reference 2e-6,
named-space round trips 2e-5, ICC float import 3e-4). Bit-identical output across
different CPUs, Rust versions or future CMS versions is not promised.

Legacy v1 project tests keep exact byte pixels and the unchanged renderer.
There was no layered .photoforge format in 0.7.x; its selection/workflow tests
remain in the suite. The new format is v2 with explicit typed metadata. It
preserves native float bits and refuses NaN, infinite values, invalid alpha,
format/encoding mismatches, future versions and aggregate mask/decode bombs.
