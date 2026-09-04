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
| Rust unit | 830 | 861 |
| Layer command integration | 39 | 42 (one optional real-file case) |
| RAW command integration | 21 | 21 |
| Precision integration | 0 | 8 |
| Optional real-file tests | 6 | 6 |
| Frontend | 851 / 52 files | 854 / 53 files |

The supplied baseline called 830+39+21+6 "893"; it actually sums to 896.
Phase 10 enumerates **938** Rust tests. Without downloaded fixtures, seven
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

The final artifact and native-validation results below supersede the initial
source-commit packaging status. Previous phases remain historical evidence only.

## Measured CPU performance and memory

Windows x64, Intel Core i7-12850HX (16 cores / 24 logical), about 128 GiB RAM.
Release benchmark built from source `5fa058f`, before the subsequent ICC-header
timestamp-only fix. One run per scenario, a fresh process for each; these are
measurements, not statistical confidence intervals. CPU load, storage and
compressibility affect results. The 60.217 MP dimensions represent the common
61 MP camera class, not exactly 61 million pixels.

The synthetic CFA fixture uses deterministic generated samples. The three-layer
case **shares one immutable source**; it is not three independent 61 MP sources.
Peak working set is the actual Windows process high-water mark including fixture
generation/decode/development, not just renderer allocations and not WebView2.
`render` excludes decode/development; `export` is file writing after render.
RAW means decode/develop/store only. Adjustment is +0.25 EV over the RAW layer.
The legacy render-only comparison quantizes the same developed source first;
its total setup/peak therefore includes float RAW development too.

| MP / dimensions | Scenario | Decode ms | Develop ms | Render ms | Export ms | Total ms | Peak MiB |
| --- | --- | ---: | ---: | ---: | ---: | ---: | ---: |
| 12.008 / 4240×2832 | raw (1 layer) | 8.6 | 264.8 | 0.0 | 0.0 | 356.1 | 394.1 |
| 12.008 / 4240×2832 | single (1 layer) | 8.5 | 264.5 | 234.5 | 0.0 | 596.9 | 397.4 |
| 12.008 / 4240×2832 | adjustment (2 layers) | 8.4 | 273.0 | 383.8 | 0.0 | 753.2 | 580.7 |
| 12.008 / 4240×2832 | mask (1 layer) | 8.5 | 274.7 | 412.6 | 0.0 | 806.7 | 409.0 |
| 12.008 / 4240×2832 | multi (3 shared) | 8.6 | 268.6 | 578.3 | 0.0 | 943.1 | 397.4 |
| 12.008 / 4240×2832 | transform (1 layer) | 9.4 | 266.0 | 310.7 | 0.0 | 676.2 | 397.4 |
| 12.008 / 4240×2832 | preview (1 layer) | 8.4 | 268.0 | 32.7 | 0.0 | 392.9 | 394.1 |
| 12.008 / 4240×2832 | export (1 layer) | 9.5 | 266.1 | 233.6 | 4113.2 | 4715.2 | 398.2 |
| 12.008 / 4240×2832 | legacy (1 layer) | 9.3 | 269.8 | 11.3 | 0.0 | 813.1 | 394.1 |
| 24.000 / 6000×4000 | raw (1 layer) | 17.0 | 532.5 | 0.0 | 0.0 | 688.9 | 783.0 |
| 24.000 / 6000×4000 | single (1 layer) | 16.9 | 530.6 | 474.2 | 0.0 | 1173.0 | 783.0 |
| 24.000 / 6000×4000 | adjustment (2 layers) | 16.8 | 529.1 | 771.8 | 0.0 | 1465.1 | 1129.6 |
| 24.000 / 6000×4000 | mask (1 layer) | 17.5 | 540.0 | 844.9 | 0.0 | 1596.0 | 786.4 |
| 24.000 / 6000×4000 | multi (3 shared) | 16.8 | 533.8 | 1146.1 | 0.0 | 1846.4 | 783.0 |
| 24.000 / 6000×4000 | transform (1 layer) | 17.1 | 530.5 | 625.6 | 0.0 | 1320.3 | 783.0 |
| 24.000 / 6000×4000 | preview (1 layer) | 18.2 | 528.5 | 32.6 | 0.0 | 716.7 | 783.0 |
| 24.000 / 6000×4000 | export (1 layer) | 17.1 | 535.7 | 472.2 | 2071.3 | 3247.1 | 783.0 |
| 24.000 / 6000×4000 | legacy (1 layer) | 16.9 | 535.3 | 22.8 | 0.0 | 1604.2 | 783.0 |
| 45.441 / 8256×5504 | raw (1 layer) | 34.1 | 1011.8 | 0.0 | 0.0 | 1343.6 | 1478.2 |
| 45.441 / 8256×5504 | single (1 layer) | 33.3 | 1002.9 | 899.7 | 0.0 | 2193.5 | 1478.2 |
| 45.441 / 8256×5504 | adjustment (2 layers) | 32.8 | 1012.9 | 1470.9 | 0.0 | 2770.5 | 2111.1 |
| 45.441 / 8256×5504 | mask (1 layer) | 32.2 | 1012.9 | 1600.1 | 0.0 | 2996.7 | 1478.2 |
| 45.441 / 8256×5504 | multi (3 shared) | 34.2 | 1050.5 | 2213.0 | 0.0 | 3563.6 | 1478.2 |
| 45.441 / 8256×5504 | transform (1 layer) | 33.1 | 1021.9 | 1187.4 | 0.0 | 2498.6 | 1478.2 |
| 45.441 / 8256×5504 | preview (1 layer) | 33.7 | 1005.8 | 33.2 | 0.0 | 1309.7 | 1478.2 |
| 45.441 / 8256×5504 | export (1 layer) | 33.7 | 1012.1 | 896.3 | 8510.0 | 10713.8 | 1478.2 |
| 45.441 / 8256×5504 | legacy (1 layer) | 32.1 | 1017.4 | 44.7 | 0.0 | 3050.9 | 1478.2 |
| 60.217 / 9504×6336 | raw (1 layer) | 47.4 | 1416.6 | 0.0 | 0.0 | 1806.6 | 1957.3 |
| 60.217 / 9504×6336 | single (1 layer) | 45.5 | 1397.9 | 1239.5 | 0.0 | 3021.2 | 1957.3 |
| 60.217 / 9504×6336 | adjustment (2 layers) | 45.6 | 1368.5 | 2007.5 | 0.0 | 3757.6 | 2787.5 |
| 60.217 / 9504×6336 | mask (1 layer) | 45.1 | 1360.2 | 2129.0 | 0.0 | 3989.9 | 1957.3 |
| 60.217 / 9504×6336 | multi (3 shared) | 46.0 | 1369.1 | 2946.4 | 0.0 | 4693.3 | 1957.3 |
| 60.217 / 9504×6336 | transform (1 layer) | 45.1 | 1357.2 | 1613.0 | 0.0 | 3349.6 | 1957.3 |
| 60.217 / 9504×6336 | preview (1 layer) | 45.7 | 1372.7 | 33.3 | 0.0 | 1763.0 | 1957.3 |
| 60.217 / 9504×6336 | export (1 layer) | 45.2 | 1371.9 | 1197.8 | 13703.3 | 16669.3 | 1957.3 |
| 60.217 / 9504×6336 | legacy (1 layer) | 47.0 | 1485.5 | 63.7 | 0.0 | 4239.3 | 1957.3 |

The float reference renderer is substantially slower than legacy RGBA8: at
60.217 MP the measured single-layer render was 1239 ms versus 64 ms legacy.
Preview rendering remained about 33 ms after source/cache preparation; a cold
RAW open includes the separately measured development time. PNG16 export at
that size took 13.7 seconds in this run. The adjustment case peaked near 2.72 GiB.
These numbers motivate future tiled/filter optimization; no GPU-speed claim is
made. Reproduce with `cargo build --release --example precision_benchmark` and
`scripts/benchmark-precision.ps1`.

## Final artifacts and installer audit

Implementation: `5fa058feae0b7a9b4c034688b587346a82f68449`.
Final executable source (ICC determinism fix):
`fb50462e3fff2c361e84e02db396f8b84f5ac1ca`.
Subsequent changes are validation documentation only. The final release source
passed the real-camera layered IPC test again in 28.30 seconds and all six
real-file tests in 4.54 seconds with the fixtures actually present.

The installed Tauri CLI was used through `npm run tauri -- build -- --offline`
(`cargo-tauri` is not installed as a separate executable). Frontend build,
release Rust build, makensis, candle and light completed successfully.
Artifacts are in the gitignored `release/0.10.0` folder; earlier artifacts were
preserved. No fixture binaries, build output, private keys or local test logs
are committed. The acquisition and benchmark scripts are committed.

| Final file | Bytes | SHA-256 |
| --- | ---: | --- |
| PhotoForge-portable.exe | 18,346,496 | a358142f2dd629e82741de9dae9c61ab4f46d662bb3ce931c4cfaa5a31d70a55 |
| PhotoForge_0.10.0_x64-setup.exe | 4,186,256 | 237fde69d523b2d48b5b3a1e59dbc73adaeb9b78beb502ebed233cc84522b63b |
| PhotoForge_0.10.0_x64_en-US.msi | 6,119,424 | 1960d6a87d9d022535b8e2532ca9ad79dec9d888e6a7918238d8cbbb6dd0c617 |

SHA256SUMS.txt verifies all three final files. All are **NotSigned**. No usable
code-signing identity was found in CurrentUser/My or LocalMachine/My, and no
production signing service was supplied. No self-signed substitute was made.
The bundled moxcms BSD notice is 1,978 bytes, SHA-256
`52d01e01773b15bf07e55dbacde09295564d016da68b6cb964aa20ec4ff5c987`.

NSIS: exact final setup installed silently per-user into a fresh validation
directory, exit 0. Registry version was 0.10.0. The installed executable was
18,346,496 bytes with SHA-256
`674e8858934f366b506a736de60089bc53928833786047e683936e57d1368fec`.
Tauri patches bundle-type metadata, so installed and portable hashes differ.
The installation included the matching third-party notice and uninstaller.
Uninstall returned 0; the validation directory, per-user registration and
shortcut were absent afterward. No PhotoForge process remained. Pre-existing
WebView user data was preserved, not counted as new installer residue.

**NSIS launch/basic-operation acceptance is not complete.** The Computer Use
skill launched its normal app-approval flow for the installed executable; the
tool returned exactly `Computer Use app approval timed out`, before a process
or targetable PhotoForge window was available. No CLI/UI-automation bypass was
used. Installation plus cleanup is not presented as a full lifecycle pass.

MSI: the final database reports ProductVersion 0.10.0, ALLUSERS=1 and ProductCode
`{88212484-A6DC-4544-AA96-F3216FBA2327}`; generated WiX uses perMachine scope.
Sandboxed administrative extraction first returned 1603 with installer service
errors 2502/2503. Repeating the same extraction under the normal Windows account
succeeded, exit 0. This is **not an installed all-users lifecycle**.
The image contains photoforge.exe (18,346,496 bytes, SHA-256
`b7f0a67b6631ce428a282195aa29a679783eefa7bbec6ea64e27e256d56adac0`),
the notice, and Tauri's generated PhotoForge Rust library photoforge_lib.dll
(137,216 bytes, SHA-256
`946d0e08a1158e1edee675208b155323bd424fa2d1de6ff5d48e580027fc58a2`).
Both PE files are NotSigned. The DLL is the project's own generated library,
not a separately introduced CMS backend. No third-party CMS DLL is required.

## Still incomplete / unverified

- Actual packaged real-DNG edit/save/reopen/PNG16 workflow and packaged
  greater-than-8-bit proof: blocked by the Computer Use app-approval timeout.
  Mock IPC, source inspection and artifacts are not substitutes for this.
- Native Windows 100%/125%/150%/200% DPI matrix: not performed. Browser zoom was
  not used as a substitute, and system scaling was not changed.
- NSIS launch/basic-operation portion and elevated MSI install/launch/uninstall:
  unverified. No legitimate UAC consent was obtained for an all-users install
  in this run; no UAC bypass or elevated-install success is claimed.
- Production Authenticode signing: unavailable; all shipped PE/installers are unsigned.
- Final-binary runtime-network observation: not performed because launch was
  not authorized through Computer Use. Existing network-boundary tests pass;
  no zero-socket claim is made for PhotoForge or its WebView2 process tree.
- Arbitrary ICC profiles are not exhaustively validated; controlled RGB profiles
  and malformed-profile cases are tested. No complete OS/monitor color control.
- Tiled DNG/monochrome LinearRaw are synthetic tests only. Other RAW formats,
  three-channel LinearRaw, additional DNG codecs/predictors and general TIFF
  import are unsupported. Unprofiled RAW is not calibrated camera color.
- Large independent-source stacks and 61 MP full-source Undo/re-development can
  exceed the 1 GiB store budget. Low-RAM active-session overlap is not certified;
  per-job estimates are not an OS process-memory cap. RAW decode cancellation
  waits until existing decode/development stages return.
- CPU float rendering is slower than legacy; full-frame filter/compositor
  intermediates remain. Cross-CPU/toolchain bitwise determinism is not promised.
- Remote CI is not claimed; no CI workflow is configured in this checkout.

## Deferred to Phase 11+

Complete tiled/out-of-core rendering, halo scheduling and disk-backed caches;
arbitrary output ICC, calibrated monitor/proof workflows, CMYK and HDR display;
additional RAW libraries only after distribution and fixture decisions; GPU,
PSD, text/vector layers, semantic/neural/generative editing, cloud services,
accounts, collaboration, scripting/plugins and video. None was added to Phase 10.
