# PhotoForge Phase 11 results — 0.11.0

This report records what is implemented in the live repository and what was
actually verified. It deliberately separates source/test evidence from native
Windows packaging evidence.

## Baseline

Phase 10 was frozen at `5711ae4f7b208acfeab5d2572fd7229c4dab11d9`, which was
the commit previously verified against `origin/main` and live GitHub main. The
Phase 10 history was not rewritten. Phase 11 work is layered on top of that
history and the four existing render commits (`c915306`, `1a2af8f`, `beb4e54`,
`7c48b00`) remain ordinary ancestors.

## Completed in source

- **Render graph:** the validated layer tree plus the runtime tiled `Plan`
  exposes source, bounds, locality, halo, cache and cancellation dependencies;
  the full-frame float renderer remains the oracle.
- **Tile model:** checked render-space coordinates, 256 px default (64–2048
  accepted), partial right/bottom edges, canonical tile keys and no
  uninitialised output.
- **Halos:** implementation-derived neighbourhood reach, accumulated for
  sequential adjustments and maximised for independent isolated branches;
  unsafe/global cases fall back rather than seam.
- **CPU tiled renderer:** bounded worker pool (maximum eight), transformed
  layers, masks, isolated/pass-through groups, cache-aware renders and ordered
  streaming bands.
- **Cache:** bounded deterministic in-memory LRU with source-content
  fingerprints, invalidation on document replacement, hit/miss/eviction/refusal
  counters and an explicit clear/budget command.
- **Optional disk cache:** local, opt-in, checksummed/versioned f32 tile files
  with fixed hexadecimal names, bounded eviction, temporary-file replacement,
  corrupt-entry misses and purge support. It is enabled only through
  `PHOTOFORGE_RENDER_CACHE_DIR` or `PHOTOFORGE_ENABLE_DISK_CACHE`; projects do
  not depend on it.
- **Streaming export:** direct linear-document PNG/PNG16 export can feed the
  encoder band by band without materialising a second whole-frame output.
  Operations that require a later full-frame pipeline retain the established
  encoder path. True 16-bit precision tests remain in place.
- **Cancellation/scheduling:** cancellation is checked between tiles/rows and
  streaming bands; at most eight render workers are used, with nested worker
  fan-out disabled.
- **GPU:** optional Vulkan/wgpu 30 backend, embedded shader, explicit Auto/CPU/
  GPU policy and diagnostics. Only eligible full-frame wide Gaussian blur is
  accelerated; all unsupported work falls back to CPU.
- **Frontend:** status bar distinguishes Rendering, Refining, Opening and
  Exporting; Diagnostics exposes policy, adapter/fallback state, GPU counters,
  cache budget/clear and the optional disk tier.
- **Version:** canonical package, Tauri and Rust versions are 0.11.0.

## Reference-equivalence and security tests

The latest offline Rust run passed 945 library tests, 42 layer-command tests,
8 precision tests, 21 RAW-command tests, 6 real-file tests when fixtures are
present, and 3 tiled-pipeline tests (1,025 reported test cases including the
library and integration suites; example targets contain no tests). Coverage
includes partial tiles, halo seams, sequential halos, transformed/off-canvas
sampling, pass-through groups, all existing blends, cache invalidation and
eviction, streaming equality, cancellation, and corrupt/truncated/versioned/
oversized/symlink-safe disk cache entries.

`cargo check --all-targets --no-default-features --features custom-protocol
--offline` passed, proving the CPU-only command stubs compile. `cargo clippy
--all-targets --all-features --offline -- -D warnings` and `cargo fmt --all`
also pass with the current Rust toolchain. `npx tsc --noEmit` passed.

The frontend gates that an earlier draft of this report could not run have now
been run in this checkout and pass:

| Gate | Command | Result |
| --- | --- | --- |
| Unit/component tests | `npm test` | 865 tests in 54 files, all passing |
| Type check | `npx tsc --noEmit` | clean |
| Svelte check | `npm run check` | 431 files, 0 errors, 0 warnings |
| Production bundle | `npm run build` | 193 modules, 431 kB JS / 79 kB CSS |

Running them found one real defect, in the test harness rather than the
product: `beforeEach(() => invokeMock.mockReset())` returns the mock, and
Vitest treats a value returned from `beforeEach` as a teardown callback, so the
Tauri `invoke` mock was being called with no arguments after every test. That
was invisible while every fallback resolved and became a failure as soon as a
test made its fallback reject. Three test files used the concise form; all
three now use a braced body.

## Performance and memory evidence

Collected with `cargo build --release --example tiled_benchmark`, three
repetitions per configuration, best of three, on an i7-12850HX (24 logical
cores) with an RTX A5500. `full` is the Phase 10 whole-frame renderer, `tiled1`
is the tiled renderer pinned to one thread, and `tiled` is the shipped
configuration. Every mode's sampled checksum agrees, and `tiled1` equals
`tiled` exactly, which is what shows the scheduler is deterministic.

| MP | scenario | full | tiled1 | tiled | vs full | of which scheduler |
| --- | --- | --- | --- | --- | --- | --- |
| 12 | stack | 701 ms | 800 ms | 224 ms | 3.13x | 3.57x |
| 12 | group | 736 ms | 841 ms | 240 ms | 3.07x | 3.51x |
| 12 | spot | 255 ms | 347 ms | 140 ms | 1.82x | 2.47x |
| 12 | blur | 801 ms | 1557 ms | 379 ms | 2.11x | 4.11x |
| 12 | global | 496 ms | 491 ms | 485 ms | 1.02x | 1.01x |
| 24 | stack | 1415 ms | 1594 ms | 400 ms | 3.54x | 3.99x |
| 24 | group | 1491 ms | 1705 ms | 427 ms | 3.49x | 3.99x |
| 24 | spot | 521 ms | 696 ms | 280 ms | 1.87x | 2.49x |
| 24 | blur | 1357 ms | 3138 ms | 656 ms | 2.07x | 4.78x |
| 24 | global | 990 ms | 983 ms | 988 ms | 1.00x | 1.00x |
| 45 | stack | 2619 ms | 2996 ms | 784 ms | 3.34x | 3.82x |
| 45 | group | 2790 ms | 3213 ms | 828 ms | 3.37x | 3.88x |
| 45 | spot | 952 ms | 1322 ms | 520 ms | 1.83x | 2.54x |
| 45 | blur | 2382 ms | 5957 ms | 1289 ms | 1.85x | 4.62x |
| 45 | global | 1848 ms | 1847 ms | 1864 ms | 0.99x | 1.00x |

Two things in that table are worth stating plainly rather than rounding off.
**Tiling on its own is slower**, by 13–150%: every `tiled1` figure is worse than
its `full` figure. The entire gain comes from the scheduler, and a report that
quoted only the `full`-to-`tiled` column would be crediting tiling with work the
thread pool did. **Untileable documents gain nothing**, as the `global` rows
show; they fall back to the whole-frame oracle and say so in `TiledStats`.

The largest single intermediate the tiled path allocates is 1.05 MB for
tile-local work and 1.27 MB with a halo, against whole frames of 192 MB at
12 MP and 719 MB at 45 MP. `peakRegionBytes` is an allocator statistic for that
one region, not a process working-set result: the non-streaming API still
returns a whole `FloatImage`, so it does not by itself bound the process. The
streaming export is the route that does.

Streaming export against whole-frame export, PNG16, best of two, byte-identical
output in every case:

| MP | scenario | whole-frame | streamed | speedup | peak working set | saved |
| --- | --- | --- | --- | --- | --- | --- |
| 24 | stack | 2623 ms | 1600 ms | 1.64x | 801 MB → 451 MB | 44% |
| 24 | blur | 3642 ms | 1898 ms | 1.92x | 1568 MB → 473 MB | 70% |
| 45 | stack | 4935 ms | 3052 ms | 1.62x | 1470 MB → 794 MB | 46% |
| 45 | blur | 7075 ms | 3622 ms | 1.95x | 2907 MB → 816 MB | 72% |

Tile cache, 45 MP, best of two:

| case | cold | after | reused |
| --- | --- | --- | --- |
| re-render, nothing changed | 1245 ms | 371 ms (3.4x) | all tiles |
| small layer moved 3 px | 512 ms | 376 ms (1.36x) | 720 of 726 tiles |
| full-canvas layer moved | — | no gain | 0 tiles, 1–5% overhead |

The cache pays when a change is spatially local and costs a little when it is
not. A fully warm 45 MP render still takes about 370 ms because assembling
cached tiles into a whole frame is memory bandwidth, not rendering.

### A GPU threshold that had to be recalibrated

The GPU radius threshold was first calibrated by timing
`high_precision::apply` on a bare frame, which put the CPU/GPU crossover at
about nine taps. Re-measuring the same operation **inside a document render**,
where the pixel store, the composited canvas and the renderer's intermediates
are already resident and the device path allocates several more frame-sized
buffers, moved the crossover to about twenty-four taps:

| radius | CPU only | GPU default | gain |
| --- | --- | --- | --- |
| 12 | 1211 ms | 1332 ms | 0.91x |
| 24 | 1484 ms | 1361 ms | 1.09x |
| 36 | 1956 ms | 1372 ms | 1.43x |
| 60 | 2816 ms | 1391 ms | 2.02x |

`MIN_GPU_RADIUS` is therefore 24, not 8. At 8 the shipped default would have
made a radius-12 blur about nine per cent *slower* than a CPU-only build. After
the correction, a radius-12 blur declines the device and is bit-identical to
CPU-only, and a radius-36 blur runs on the device 1.48x faster with a sampled
checksum difference of 2.0e-5 on a sum of roughly 23,000 float channels — far
below one step of a 16-bit channel.

The source-level memory guarantees are therefore precise: the tile scheduler,
cache and disk tier enforce their own byte budgets; full-frame output and
global/document-level pipelines remain subject to the existing 1 GiB store and
4 GiB job estimates. Independent 45/61 MP source stacks and large Undo-backed
RAW redevelopment remain safely rejected when those budgets would be exceeded.

## Packaged validation and artifacts

No final 0.11.0 Tauri package or hashes are claimed in this report yet. The
frontend build gate above must be available before the real `tauri build` can
produce trustworthy installers. The previous 0.10.0 artifacts remain historical
and are not relabelled as 0.11.0. Native packaged DNG editing, GPU initialization
in the packaged executable, NSIS launch, MSI elevated install/launch/uninstall,
100/125/150/200% Windows DPI, and final-binary network observation are not
verified here. Production Authenticode signing remains unavailable; no fake
certificate is generated.

## Still incomplete / unverified

- document-level operations after a layer composite are not generally tiled;
  global/statistical operations use the full-frame oracle;
- GPU support is limited to the measured wide-blur path, Vulkan-only in the
  current graph, with no GPU blend/adjustment/group/transform/cache parity;
- no complete tile-aware RAW decoder or independent multi-source 61 MP stress
  test is claimed;
- visible-viewport priority/progressive refinement is represented in tile
  ordering and UI status, but a complete viewport scheduler is not shipped;
- release packaging, packaged GUI workflow, DPI matrix, UAC MSI lifecycle,
  signing and process-tree network observation need an environment with the
  required approvals; the frontend Vitest/Svelte/Vite gates now run and pass.

## Deferred

General GPU compositing, all-GPU blend/adjustment/group/transform parity,
persistent VRAM caches, arbitrary ICC/soft proofing/HDR display, additional
proprietary RAW decoders, PSD/text/vector/smart-object layers, neural or
generative editing, cloud/accounts/collaboration, scripting/plugin SDK, video,
CMYK and other product features listed in the Phase 11 prompt remain deferred
to later phases.
