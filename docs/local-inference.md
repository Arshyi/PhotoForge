# Local inference

PhotoForge can run a neural model you install yourself. It ships none, it
downloads none, and it is a complete editor without one.

## Optional means optional

A normal install has no models. In that state the model registry reports every
capability as unavailable with a reason, the Model Manager says so, and every
restoration tool in PhotoForge still works — they are all deterministic. A
build with neither the `gpu` nor the `inference` feature compiles and passes
its tests.

Two things are optional and reported separately, because they are different
problems with different fixes:

- whether an inference **runtime** is compiled into the build, and
- whether a **model** is installed.

## Runtime selection

Evaluated on evidence, not popularity, reviewed 2026-09-05.

| Candidate | Version | Licence | Native dependency | Notes |
| --- | --- | --- | --- | --- |
| **tract-onnx** (selected) | 0.23.6 | MIT OR Apache-2.0 | **None — pure Rust** | ONNX and NNEF inference, CPU only. Requires Rust 1.91. |
| ort | 2.0.0-rc.13 | MIT OR Apache-2.0 | ONNX Runtime | No stable release. `download-binaries` is in its default feature set, fetching a native runtime at build time. |
| candle | — | MIT OR Apache-2.0 | Optional CUDA | A framework rather than a general model loader; models are described in Rust per architecture. |
| wonnx | — | MIT OR Apache-2.0 | wgpu | GPU-first, less mature ONNX coverage. |

tract was selected because it adds **no native library**. A runtime that ships
DLLs has to be deliberately packaged and can otherwise be picked up from a
developer's PATH, which is exactly the failure mode a reproducible installer
must not have. It also means the CPU claim is unambiguous.

**tract is CPU only.** There is no GPU inference in PhotoForge and none is
claimed. The GPU work in Phase 11 is an unrelated compute path for wide blurs.

The dependency costs 251 crates and raises the crate's minimum Rust to 1.91.

## What runs, and what does not

The runtime is real and exercised end to end by the test suite: a minimal ONNX
model is authored in memory — protobuf written directly, no code generator, no
Python — written to a file, imported through PhotoForge's own import path, and
run over a whole image through the tiling code. It reproduces the graph's
arithmetic exactly.

**No production model has been tested**, because none ships and none is
downloaded. The capabilities below are architecturally supported; whether a
particular published model works is a question only its user can answer by
importing it.

| Capability | Architecture | Model shipped | Tested with a production model |
| --- | --- | --- | --- |
| Super resolution | Supported, integer scale from the descriptor | No | No |
| Denoise | Supported | No | No |
| Deblur | Supported | No | No |
| Artifact removal | Supported | No | No |
| Segmentation | Declared; would produce a mask, not pixels | No | No |
| Face restoration | Declared | No | No |
| Inpainting | Declared; deliberately not implemented in Phase 12 | No | No |

## How an image reaches a model

1. **Colour.** The document is linear light. Almost every published imaging
   model was trained on gamma-encoded sRGB, so the conversion is explicit and
   driven by the model's own descriptor. A model that wants linear has to say
   so; nothing is assumed.
2. **Normalisation.** `[0,1]` or `[-1,1]`, from the descriptor.
3. **Tiles.** A 60 MP image is not one tensor. Tiles are the size the descriptor
   declares, overlap by the amount it declares, and are blended on a linear ramp
   so the join is a gradient rather than a line.
4. **Alpha** is carried *around* the model, not through it. An RGB model has no
   opinion about opacity and inventing one would be worse than preserving what
   the document already knows.
5. **Output shape** is checked against what the descriptor promised before a
   single value is read out, so a model that does not scale by the factor it
   claims is refused rather than producing a partly filled image.

## What PhotoForge will not do

- **No downloads.** Not on first run, not on first use of a capability, not
  ever. Models are installed by explicit user action from a file already on the
  machine.
- **No Python.** Not required, not invoked, not expected.
- **No shelling out.** No `python.exe`, no external inference binary, no script.
- **No pickle.** `.pt`, `.pth`, `.ckpt`, `.pkl` and friends are refused by name.
  Unpickling executes arbitrary code by design, and the answer is to not support
  the format rather than to try to sanitise it.
- **No network.** Inference is local. Nothing about an image or a model leaves
  the machine.

## The planner is not the runtime

The optional Ollama planner suggests restoration *plans* as text. It never
receives image pixels and it is not an inference provider. The two are separate
architectures and are kept that way. See
[ollama-provider.md](ollama-provider.md) and
[local-ai-privacy.md](local-ai-privacy.md).

## Reproducibility

Classical restoration is deterministic. For a neural result, the model's
SHA-256 is recorded at import and re-checked before use: a model whose bytes
have changed since import is refused rather than silently substituted, so a
result cannot be attributed to a model that is no longer there.

Floating-point inference may vary slightly across runtime versions and CPU
feature sets. PhotoForge does not promise bit-identical output across machines
for a neural operation, and does promise it for classical ones.

See [model-format.md](model-format.md) for what a model must declare.
