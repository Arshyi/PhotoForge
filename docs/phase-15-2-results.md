# Phase 15.2 — writing Photoshop documents (PSD and PSB)

[15.1](phase-15-1-results.md) opened Photoshop files. This increment writes them: the layers go
into the file as layers, with a report of what was written exactly, what approximately, and what
was left out. The format and its rules are in [psd-export.md](psd-export.md).

Status: **implemented and verified as listed; not committed, not released.** The version is still
0.14.0. It is on branch `phase-15-2-psd-export`, on top of `c11cdb0` (15.1b–d), and nothing here
has been pushed. **The files have never been opened in Photoshop**; see "What was not verified".

## Completed

* **`src-tauri/src/psd/export.rs`**: the writer. Header, sRGB profile (8/16-bit), layer records with
  Unicode names, ids and lock, folders with their dividers, masks (inversion baked into the stored
  data), the clipping flag, 26 blend modes, Curves / Levels / Invert adjustment layers, PackBits
  channels (raw for 32-bit), the flattened image, and PSB (8-byte lengths, `Lr16`/`Lr32` for deep
  documents) where a side passes 30,000 pixels or the data 2 GiB.
  * **Exact:** whole-pixel-placed pixel layers, opacity, visibility, lock, blend, groups, masks,
    clipping, names.
  * **Approximate, and reported:** transformed layers and transformed groups (baked, the mask
    sampled the way the renderer does, so the edges agree), shape / text / smart layers
    (rasterised), Curves reduced to 16 points by a fit against Photoshop's own spline, per-channel
    tone adjustments (found by trying them on a ramp and on mixed colours) as Curves.
  * **Left out, and reported:** adjustments that mix channels or look at neighbours (blur,
    hue/saturation, white balance, plug-in filters); edits over the whole document (in the
    flattened image when they keep its size, in neither when they do not).
* **The depth is chosen from the document**: 8-bit for 8-bit sRGB, 16-bit for high precision,
  32-bit when a high-precision document holds colour brighter than white (a note says so). A depth
  can be named through the command; highlights are then clipped and the report says so.
* **`export_psd_document` command** (`commands/layers.rs`): validates the document, operations and
  plug-ins, requires a `.psd` or `.psb` name, refuses to replace a file the document reads from,
  writes beside the destination and renames, and turns a `.psd` name into `.psb` when the document
  needs the large format (unless that name is taken, which it says).
* **The interface**: *Photoshop document (layers kept)* in the Export dialog, a report dialog
  titled "Photoshop export" with three sections (written approximately / left out / notes), and a
  one-line notice with the counts.
* **`ReportKind::NotExported`** next to the importer's kinds, so one report component serves both.

## Against real files

The 316 psd-tools files used since 15.1 (MIT, not committed; `PHOTOFORGE_PSD_SAMPLES`), each
imported, exported, imported again and rendered:

| Result | Files |
| --- | --- |
| Imported (the other 26 are refused on import: CMYK, Lab, indexed, bitmap, duotone, multichannel) | 290 |
| Exported and read back with the same layers (names, order, blend, visibility, clipping, masks) | 290 |
| **The picture identical** | **274** |
| The picture within a level on average (largest single difference 26 levels, `curves_rgb.psd`, a sharp feature of one curve) | 16 |
| Further than that | 0 |
| Written | 55 MiB; slowest file 1.64 s (`gray1.psb`) |

What the exports said, by number of files: 54 had Curves written as Photoshop Curves
(approximated), 15 were written as 32-bit because they hold colour brighter than white.

**Independent structural check.** `psd_check.py` (outside the repository, written separately from
the reader and writer) parses each exported file: signatures, section lengths, every record's
blocks ending exactly where its extra data does, block lengths a multiple of four, the channel
data filling the layer information, folders and dividers balanced, a name and an id on every
layer. **All 290 pass.** The same checker on the 316 files Photoshop and its relatives wrote
passes 142 strictly and 146 loosely, so it is not a rubber stamp: it is stricter than some real
files (it expects mask data of 0, 20 or 36 bytes, Unicode names padded to four) and simplified in
places (the global additional blocks). A pass means *consistent with the conventions in files
Photoshop wrote*, not *Photoshop opens it*.

## Defects found by checking, and fixed

| Found by | Defect |
| --- | --- |
| The independent parser, on the first real export | A tagged block's length did not include its padding to four bytes, so a strict reader would have landed in the middle of the next block. PhotoForge's own reader and importer did not mind, which is why the tests had not. Fixed; a test now walks the bytes without the reader's leniency |
| Comparing records with Photoshop's own | A pass-through group's record blend key must be `norm` with the `pass` in the section block, not the group's mode in both |
| Round trip, first version | A transformed layer's mask edge was 30 levels off (the mask had been rendered through a carrier layer); replaced by sampling the mask exactly as the renderer does |
| Round trip, first version | A Brightness lift exported as three curve points rounded off the kink (31 levels); the reduction now fits Photoshop's spline |
| Writing this document | A `.psd` name that became `.psb` for size said nothing in the report; now a note, with two tests (renamed; name taken) |
| `clippy` on the new file | Four lints (derivable `Default`, `is_multiple_of`, an indexed loop, a complex type); fixed |
| Reading the diff | A literal NUL byte in `PsdImportReport.svelte` from 15.1, which made git treat the file as binary; replaced by the escape `\u0000` |

## Verification

Same machine and settings as 15.1 (dev profile, dependency debug info reduced per invocation).

| Gate | Result |
| --- | --- |
| `cargo fmt --check` | clean |
| `cargo clippy --all-targets --all-features -- -D warnings`, and the same with no GPU / no inference / no plugins | clean (all four) |
| `cargo test --all-targets --all-features --no-fail-fast` | **1,585 passed**, 0 failed, 45 test binaries (15.1d ended at 1,561) |
| No GPU / no inference / no plugins | **1,578 / 1,577 / 1,546 passed**, 0 failed |
| `npx tsc --noEmit`, `npm run check` | clean; 510 files, 0 errors, 0 warnings |
| `npx vitest run --maxWorkers=4` | 85 files, **1,291 passed**, 0 failed (15.1d ended at 1,285); one *unhandled error*, from a `CommandPalette` test that makes a command throw on purpose, a file this work does not touch |
| `npm run build` | built (the chunk-size warning is the existing one) |

New tests: 16 in the writer (every blend mode has a key that reads back as itself; a layered
document reads back the same; the file is laid out as Photoshop's are; **every tagged block is
padded and every record ends where it says**; off-canvas rectangles; baked transforms; Curves,
Levels and Invert; per-channel tone as Curves; channel-mixing adjustments left out and named;
whole-document edits reported; 16-bit; highlights kept at 32-bit and clipped at 16; PSB;
**inverted and disabled masks come back as they paint**; a layer that paints nothing is still
there; a failed export leaves what was there), 7 through the command boundary (the name, the
arguments, the report, the refusals, the large format by request and by size, a name that is
taken), 6 in the interface (the dialog offers Photoshop files; the layers are written to a Photoshop
name and the file's report is shown; what was left out and what was approximated are listed;
the file named is the one the engine wrote, which may not be the one asked for; a failure shows
its reason and leaves the document alone; flattened images still go the image path), and the
real-file round trip (runs only with
`PHOTOFORGE_PSD_SAMPLES`).

Each check was shown able to fail by breaking what it protects:

| Mutation | Caught by |
| --- | --- |
| Block bodies not padded | the byte-walking layout test (`luni holds 6 bytes`); the other 15 writer tests still passed, which is why that test exists |
| Mask inversion not applied | the mask test |
| Clipping flag not written | the layered round trip |
| Hidden flag not written | the layered round trip |
| Disabled-mask flag not written | the mask test |
| Alpha plane and red plane swapped | eleven tests |

## What was not verified

* **Opening a written file in Photoshop**, Affinity, GIMP or any program other than this one's own
  reader and a separately written parser. Compatibility is likely and unconfirmed. This is the
  first thing to do with a real Photoshop; the report dialog is the right place to mention it
  until then.
* **The window.** The dialog, the report and the notice are covered by component tests against a
  mocked command boundary and the command by tests of the real command, but the packaged app
  has not been driven through an export.
* **DPI and the GUI matrix**, as before.
* **Files of several gigabytes.** The size rule (PSB beyond 30,000 pixels or 2 GiB) is tested at
  30,001 pixels wide; a document near 2 GiB has not been written.
* **Timing**: the numbers above are one run on one machine.

## What is still not done

Editable text, editable shapes and smart objects in the file (they are written as pixels);
Photoshop's Brightness/Contrast, Hue/Saturation, Posterize and Threshold as layers of those names;
layer styles; a depth chosen in the dialog; CMYK and Lab. The importer's own gaps (vector shape
and fill layers, layer styles, text) are listed in [phase-15-1d-results.md](phase-15-1d-results.md).

## Next

15.3: metadata, EXR, SVG, clipboard. The README and the changelog are not changed by 15.1 or
15.2 and will be at the 0.15.0 release, because they describe 0.14.0.
