# PhotoForge Phase 12 results — 0.12.0

What was built, what was measured, and what was not verified.

## Baseline

Phase 11 was frozen at `778a7fea3a774d24d92b81c8674fe7451c8683b6`, verified
before any Phase 12 work as clean, and matching `origin/main` and live GitHub
main. No history was rewritten and nothing was force-pushed.

Baseline gates at that commit: 1,025 Rust test cases, 865 frontend tests across
54 files, `cargo fmt --check`, `clippy --all-targets --all-features -D warnings`,
`tsc --noEmit`, `svelte-check` over 431 files and a production bundle, all
passing.

## How restoration is judged here

Every figure below is measured against a **known-clean fixture** by
`image_processing::metrics`, not asserted. Four metrics, and never one alone:

- **PSNR / SSIM** — closeness to the original.
- **`edge_retention`** — gradient energy surviving where the clean image has
  edges. A denoiser can always win on error by erasing the picture; this is
  what stops it.
- **`flat_area_noise`** — residual noise only where the clean image is flat, so
  a filter cannot win by leaving the noise.
- **`blocking_energy`** — steps on the 8-pixel grid against steps elsewhere.

Each metric is checked against something other than itself: PSNR against its
closed form for a known offset, SSIM against an inverted stripe pattern, the
noise generator against its own mean and variance.

The fixtures are synthetic and say so. A Gaussian field is not a sensor and a
block-quantised image is not a JPEG encoder.

## Classical restoration

Measured on 256 px fixtures, Phase 11 algorithms against Phase 12:

| Case | Degraded | Phase 11 | Phase 12 | Change | Edges 11 → 12 |
| --- | --- | --- | --- | --- | --- |
| Sensor defects | 34.94 dB | 30.17 dB | **45.28 dB** | +15.11 | 0.68 → 1.00 |
| Colour noise | 23.88 | 27.38 | **35.29** | +7.91 | 0.64 → 0.98 |
| Impulse noise | 21.97 | 25.17 | **30.92** | +5.75 | 0.74 → 1.00 |
| Motion blur | 22.67 | 22.58 | **26.94** | +4.36 | 0.61 → 0.71 |
| Dust | 30.91 | 28.45 | 31.34 | +2.89 | 0.68 → 1.01 |
| Gaussian, light | 30.49 | 31.93 | 33.76 | +1.83 | 0.85 → 1.00 |
| Defocus blur | 23.31 | 23.35 | 25.11 | +1.76 | 0.50 → 0.56 |
| JPEG blocking | 35.80 | 34.96 | 35.79 | +0.83 | 0.96 → 1.00 |
| Gaussian, heavy | 21.94 | 26.39 | 25.90 | **−0.49** | 0.69 → 1.00 |
| Uneven illumination | 17.37 | 18.10 | 18.10 | 0.00 | 0.79 → 0.79 |

**Heavy noise is a deliberate regression.** It gives up half a decibel and gains
0.31 of edge retention. PSNR rewards blurring, which is exactly why it is never
reported alone here.

**Uneven illumination is unchanged.** It was not touched in this phase. A radius
sweep found it only helps at all once its background estimate approaches the
scale of the defect, and the existing bounded resource limit rejects radius 128.

### What changed

- **Denoise** rebuilt around a YCoCg luma/chroma split with a Gaussian range
  kernel. The previous weight fell off hyperbolically, so a tap across a hard
  edge still carried real weight. Gains a **Colour** control alongside Strength
  and Detail, each mapped to a real algorithm parameter and each covered by a
  test asserting it does something.
- **Dust & hot pixels** is a new operation, because a bilateral filter cannot
  repair an impulse — its range kernel is centred on the damaged pixel. Ring
  median with the centre excluded, gated on the ring agreeing with itself so
  stars and single-pixel lines survive. Running it on an undamaged image costs
  under 0.02 dB.
- **Deconvolution** is new: Richardson-Lucy against a named kernel. It does not
  estimate an unknown one and the interface asks rather than guessing.
- **JPEG cleanup** rebuilt to gate on interior gradients and spread the
  correction over three pixels either side. Removes ~74% of the added grid
  energy; the previous version lost 0.84 dB and 5% of the edge energy.

### Cost

A full-resolution 24 MP denoise takes ~2.1 s on eight threads, substantially
slower than the filter it replaces. A quantised decay table and hoisted spatial
weights cut it from 3.3 s while moving PSNR by 0.003 dB. Interactive work goes
through the preview scale and the tiled renderer.

## Three bugs found by these tests

Recorded because they were introduced in this phase and caught by its own gates.

1. **The adaptive noise estimate broke tiling.** Sizing the range kernel to the
   measured noise scored better and was wrong: the tiled renderer hands the
   operation one tile at a time, so each tile measured its own noise and chose
   its own sigma. The seam was 0.12 — eight thousand times one step of a 16-bit
   channel. A tile-local operation has to be a pure function of its parameters.
2. **`estimated_bytes` overflowed.** `tile_size² × 4` without saturating, so a
   hostile tile size wrapped to a small number — precisely the input that would
   then pass an admission check.
3. **The first damping implementation increased ringing.** Relaxing the
   per-iteration step is the obvious reading and does not control ringing:
   measured overshoot *grew* from 0.045 to 0.098 at damping 0.8. Ringing is the
   estimate leaving the range of the values around it, so the control now bounds
   exactly that.

## Local inference

**Runtime: tract-onnx 0.23.6** (MIT OR Apache-2.0), selected because it is pure
Rust and adds no native library. `ort 2.0.0-rc.13` was rejected: no stable
release, and `download-binaries` in its default feature set fetches a native
runtime at build time. tract is **CPU only**; no GPU inference exists or is
claimed.

**The path is real and exercised end to end.** The tests author a minimal ONNX
model in memory — protobuf written directly, no code generator, no Python,
nothing downloaded — write it to a file, import it through PhotoForge's own
import path, and run it over a whole image through the tiling code. It
reproduces the graph's arithmetic exactly.

| Capability | Architecture | Model ships | Tested with a production model |
| --- | --- | --- | --- |
| Super resolution | Supported | No | **No** |
| Denoise | Supported | No | **No** |
| Deblur | Supported | No | **No** |
| Artifact removal | Supported | No | **No** |
| Segmentation | Declared, would produce a mask | No | **No** |
| Face restoration | Declared | No | **No** |
| Inpainting | Declared, deliberately not implemented | No | **No** |

Model files are treated as untrusted: pickle-bearing extensions refused by name,
size bounded 32 bytes to 512 MB, leading protobuf tag checked, symlinks and
directories refused, every declared dimension range-checked, identifiers
restricted so they cannot steer a path. Recorded size and hash come from reading
the file, never from the import request, and `resolve_file` re-hashes before use.

Costs: 251 crates, and the crate's minimum Rust rises to 1.91. The packaged
executable grows from 23.5 MB at 0.11.0 to **42.5 MB**.

## Test totals

| Suite | Count |
| --- | --- |
| Rust library | 989 |
| Layer commands | 42 |
| Precision pipeline | 8 |
| RAW commands | 21 |
| RAW real files (fixtures present) | 6 |
| Restoration | 16 |
| Tiled pipeline | 3 |
| **Rust total** | **1,085** |
| Frontend (55 files) | 875 |

`cargo fmt --check`, `clippy --all-targets --all-features`, `tsc --noEmit`,
`svelte-check` (433 files) and the production bundle all pass. A build with
neither `gpu` nor `inference` compiles, which is the CPU-only, model-free
configuration the editor is designed around.

## Packaged artifacts

Built with `npm run tauri build`.

| Artifact | Size | SHA-256 |
| --- | --- | --- |
| `photoforge.exe` | 42,487,808 B | `7609c833e17c082df13d869cd00ddfb63947944d9f2345453394bd3979ca1be9` |
| `PhotoForge_0.12.0_x64-setup.exe` | 9,439,885 B | `3ac5c99237ae145ef52aa95d8a37a132d5947fe425a6931419596a5dab11d545` |
| `PhotoForge_0.12.0_x64_en-US.msi` | 19,996,672 B | `70258bd11960bc2f32517d6705e54b881937c191ebb9a85edf44a7924117c41d` |

Also in `SHA256SUMS.txt`. All three report **NotSigned**; no production
Authenticode identity is available and no certificate was fabricated. The
packaged executable launches and presents its window.

## Still incomplete / unverified

- **No production neural model has been run.** The runtime is proven on a model
  PhotoForge authored for its own tests. Whether any published model works is
  answerable only by importing one.
- **No model import UI.** The Model Manager lists, reports and removes; import
  needs a file picker and a descriptor form, and a button that cannot finish the
  job would be worse than none. Import works through the command and its tests.
- **No GPU inference**, and none claimed — tract is CPU only.
- **Sharpening was not reworked** (Stage E). Capture/creative/output modes are
  not implemented; the existing sharpen and edge-aware sharpen are unchanged.
- **Scratch detection, faded-photo presets and document enhance v2** were not
  implemented (Stages G and J beyond dust). Fixtures for scratches and fading
  exist and are unused by any tool.
- **Clarity and Texture** were not separated from Local Contrast (Stage I).
- **Neural batch scheduling** is not implemented; classical restoration works in
  batch through the existing engine.
- **Workflow steps** were not extended with the new operations (Stage AB).
- **Uneven illumination** is unchanged and weak.
- **Packaged GUI workflow, DPI matrix (100/125/150/200%), MSI elevated
  install/uninstall under a real UAC prompt, Authenticode signing and
  process-tree network observation** remain unverified, as in Phase 11.

## Deferred to Phase 13+

Generative fill, text-to-image, diffusion editing, cloud models, accounts,
collaboration, PSD, text and vector layers, smart objects, a plugin SDK,
unrestricted scripting, video, additional proprietary RAW decoders and CMYK
editing all remain out of scope, as the Phase 12 brief required.
