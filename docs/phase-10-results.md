# Phase 10 implementation and verification

The frozen 0.9.0 baseline is
`e7932f27e1f6ea54df5f24cf6c5229ad405e4d32`. Local HEAD, the local remote-tracking
ref and live GitHub main were checked and matched before implementation.

The pre-migration precision audit is in
[high-precision-rendering.md](high-precision-rendering.md).

## Implemented and tested

Phase 10 implements the typed linear-f32 layer renderer, straight source alpha
with premultiplied composition/resampling, all sixteen existing blends,
float adjustments, groups/masks/transforms, native PNG16 import, genuine layered
PNG16 export, and exact typed v2 project/recovery data. Old v1 documents retain
their original byte renderer. DNG placement and re-development retain source
hashes and parameters. Batch DNG development is sequential, source-safe and
honors selected RGB profiles/bit depth.

sRGB, Display P3 and Adobe RGB perform real transforms. Embedded RGB ICC input
uses pinned moxcms 0.8.1 with bounded parsing; exports embed the selected
profile. Display remains sRGB RGBA8 under Windows/WebView2, not a controlled
monitor-profile chain. See [color-pipeline.md](color-pipeline.md).

### Verification on 2026-09-04

| Suite | Frozen baseline | Phase 10 |
| --- | ---: | ---: |
| Rust unit | 830 | 860 |
| Layer command integration | 39 | 42 (one optional real-file case) |
| RAW command integration | 21 | 21 |
| Precision integration | 0 | 8 |
| Optional real-file tests | 6 | 6 |
| Frontend | 851 / 52 files | 854 / 53 files |

The supplied baseline called 830+39+21+6 "893"; it actually sums to 896.
Phase 10 enumerates **937** Rust tests. Without downloaded fixtures, seven
optional test bodies return early (six real-file plus one layer test). All
seven also ran separately with actual fixtures, successfully, in release mode.
The isolated recovery child is the same test, not an extra counted test.

Passed: cargo fmt --all -- --check; cargo clippy --all-targets --all-features
--offline -- -D warnings; cargo test --all-targets --all-features --offline;
npm test; npx tsc --noEmit; npm run check; npm run build.
Toolchain: rustc 1.97.1, Node 24.18.0, npm 11.19.0, Windows x64.

The real Canon EOS 5D Mark III lossless DNG (5920x3950) traversed Tauri's mock
command boundary: Open RAW, float layer, half-coverage mask, pass-through group,
brightness adjustment, full source re-development, save, reopen, PNG16 export.
Output included samples not divisible by 257 and the expected half alpha;
the original SHA-256 remained
`3118116735d4f6dd01fd9d6a80ee9aac52a2c2debdc6dcd112bd5e3059044a55`.
This is integration evidence, **not packaged GUI evidence**. Real lossless and
uncompressed Canon samples also decoded to identical sensor samples; lossy DNG
was refused. The six real-file tests passed with fixtures in 4.42 seconds.

Generated tiled/partial-edge DNG and one-channel LinearRaw are verified
synthetically only. Three-channel LinearRaw and other unsupported variants are
explicitly rejected. Bounded malformed-input tests cover truncation, offsets,
dimensions, segment counts, bit depths, metadata, profiles and project masks;
they are not a proof against every malformed file or a long-running fuzz campaign.

The full-frame byte-cost model admits representative 45/60 MP synthetic cases,
but limits immutable source+history storage to 1 GiB. Independent large sources
and full RAW re-development with retained 61 MP Undo data can exceed it.
See [high-precision-rendering.md](high-precision-rendering.md) for exact boundaries.

Packaging, final artifact hashes, native GUI/DPI and installer lifecycle checks
are pending in this source commit. No production signing or zero-process-tree
networking claim is made. Previous phases are historical evidence only.
