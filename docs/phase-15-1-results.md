# Phase 15.1 — opening Photoshop documents (PSD and PSB), import only

The second increment of Phase 15. It adds a reader for Photoshop files and an importer that
turns one into a layered PhotoForge document with a report of what did not survive. It writes
no Photoshop files. What is mapped, approximated and left out is in
[psd-import.md](psd-import.md); this page is what was done and how far it was verified.

Status: **implemented and verified as listed; not committed, not released.** The version is
still 0.14.0. It builds on [15.0](phase-15-0-results.md) (the panel's edits are the engine's),
which is on the branch below it.

## Completed

* **A bounded reader** (`src-tauri/src/psd/`): header, resources (ICC profile, resolution),
  layer records for PSD and PSB at 8, 16 and 32 bits (16/32-bit layers are found in the
  document-level `Lr16` / `Lr32` block), additional-information blocks (Unicode name, section
  dividers, layer id, fill opacity, and the keys of everything else), the group tree, and
  channel decoding in raw, PackBits, zlib and zlib-with-prediction (8, 16 and 32 bits), plus the
  merged image. Every declared length is checked against its own section before anything is
  allocated.
* **An importer** that maps pixel layers (position, opacity with fill opacity, visibility, 16
  blend modes), groups (isolated and pass-through, collapsed or not), layer and group masks,
  and Unicode names, in an 8-bit document or — for 16-bit, 32-bit and profile-tagged files —
  a high-precision one converted through the embedded profile. A file with no usable layers
  imports its flattened image.
* **A report** in the document of everything approximated or left out, shown after opening
  (grouped, with repeated findings condensed) and returned by the command.
* **The command `import_psd_document`** and the interface: Photoshop files appear in the Open
  dialog, drop on the window, and open as an unsaved layered document (Save asks for a
  PhotoForge project path, never the Photoshop file).
* **Refusals that say what to do:** CMYK, Lab, indexed, bitmap, duotone, multichannel and
  1-bit files; canvases and layers beyond the resource budgets.

## Verification

Same machine and settings as 15.0 (dev profile, dependency debug info reduced per invocation).

| Gate | Result |
| --- | --- |
| `cargo fmt --check` | clean |
| `cargo clippy --all-targets --all-features -- -D warnings`, and the same with no GPU / no inference / no plugins | clean |
| `cargo test --all-targets --all-features --no-fail-fast` | **1,495 passed**, 0 failed, 43 test binaries (15.0 ended at 1,460) |
| No GPU / no inference / no plugins | **1,488 / 1,487 / 1,456 passed**, 0 failed |
| `npx tsc --noEmit`, `npm run check` | clean; 510 files, 0 errors, 0 warnings |
| `npm test` | 85 files, **1,264 passed**, 0 failed (15.0 ended at 1,257) |
| `npm run build` | built |

New Rust tests: 30 reader and importer unit tests (including a sweep of **every prefix** of a
valid file and **4,515 single-byte mutations**, none of which panics), 3 through the real
command boundary, and two real-file checks that run when `PHOTOFORGE_PSD_SAMPLES` is set. New
frontend tests: 7.

**Against real files.** The specification-based tests prove only consistency with one reading of
the specification, so the reader and importer were run on **315 real PSD and PSB files** — the
test suite of the MIT-licensed *psd-tools* project, downloaded for this and **not committed**
— plus one of Arshyia's own:

| | |
| --- | --- |
| Files read by the reader | 315 of 315; 4,445 channels decoded to exactly their declared sizes; 32-bit samples checked finite and plausible |
| Files imported | 289; 26 refused (CMYK, Lab, indexed, bitmap, duotone, multichannel, 1-bit), 0 failed |
| Imports that reported nothing approximated or left out | 35; **34** match Photoshop's own flattened image (mean error ≤ 2 levels, ≥ 97 % of pixels within 8); 1 does not, whose stored flattened image is entirely white although its layer is a clean triangle (checked by decoding its channels independently) |
| Imports that reported approximations or omissions | 220; 115 of them still match closely |

Photoshop's flattened image stored in each file is an independent picture of what the layers
should look like, which is what makes this a real check. **Photoshop itself was not run**.

Running real files found three defects that the written-from-the-spec tests had not: the
field order in a layer-mask block, the row table of a four-channel flattened image, and that
32-bit files use zlib with prediction throughout (so refusing it, as first written, would have
refused every 32-bit file). All three are fixed and covered.

Each importer check was shown able to fail by breaking what it protects — the fill-opacity
multiply, the mask offset into the layer grid, the clip bake, the blend-mode report — and
restoring it. The mask-offset check first *survived* (the test had a layer at the origin, so
the offset did nothing); the test was strengthened and the mutation is then caught.

**In the real application window** (a debug build with WebView2 and the real Rust importer,
driven over the browser debugging port; not the packaged release): 12 files were dropped on the
window — a 4×4 16-bit file, a 4×4 32-bit file, PSD and PSB with groups, a file with 3 groups
and masks, clipping masks, a 108-layer / 56-group blend-mode chart, a layer-effects file, and
CMYK files. Ten opened and two CMYK files were refused with the stated message; the window stayed
responsive throughout; import times were 24–696 ms in a debug build. The report dialog and the
document view were checked on screenshots.

## What is not done

* **No writing of Photoshop files** (15.2).
* **Clipping is baked, not live.** The layer model has no clipping relation yet; Arshyia
  approved adding one, and it needs the compositors (8-bit, float, tiled) changed together.
* **Shape and fill layers stored as vectors with no pixels, adjustment layers, layer styles,
  text and smart objects as editable content** are left out or imported as pixels, and
  reported. In the 315 files these are the most common reasons a picture differs: layer styles
  (202 layers), adjustment layers (about 160), 11 blend modes PhotoForge lacks (80), pixel-less
  shape layers (49).
* **No colour modes other than RGB and greyscale**, and no 1-bit.
* **The real-file checks need files.** They are not in the repository and are skipped without
  `PHOTOFORGE_PSD_SAMPLES`, so the gate that caught the three defects does not run on a clean
  checkout. The unit tests that do run use files written from the same reading of the
  specification as the reader.
* A PSB wider or taller than 131,072 pixels is refused (the canvas ceiling).
* Not run: the packaged release, display scaling, a screen reader, a Photoshop file saved by
  a current Photoshop.
* An unrelated thing seen on the screenshots, not changed: the "Text, shapes and smart
  objects" box in the sidebar draws with the browser's default border and overlaps the
  "Color and precision" heading. It predates this increment.
