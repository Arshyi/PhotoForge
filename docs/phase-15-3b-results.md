# Phase 15.3b — SVG export of shape layers

The second part of 15.3. Shape layers can now be written as SVG, with a report of what an SVG
cannot hold. The rules and the format table are in [svg.md](svg.md); this is what was done and
what was checked. Reading SVG is the next part and is not in this increment.

Status: **implemented and verified as listed; not committed, not released.** The version is still
0.14.0. On branch `phase-15-3b-svg`, on top of `3451e3b` (15.3a).

## Completed

* **`src-tauri/src/svg/export.rs`** (new): the shape layers and groups of a document as SVG text.
  Rectangles, ellipses and lines keep their form; polygons, stars and paths are path data from
  the commands the renderer itself flattens; fill, stroke, cap, join, miter limit, fill rule,
  opacity, visibility, the sixteen W3C blend modes and nested groups are written. The layer
  transform is one `matrix(…)` about the centre of the canvas, which is how the compositor maps a
  shape (and is checked against the compositor's own `forward`).
* **`export_svg_document` command**: validates the document, requires an absolute `.svg` name in an
  existing folder, never replaces a file the document links to, and writes beside the destination
  and renames, so a failure leaves what was there.
* **The interface**: *SVG (shape layers)* in the Export dialog, and the same report dialog as a
  Photoshop export, which now also reads "SVG export", counts shapes, and no longer calls the
  approximated section of an export "Imported, but not exactly as in Photoshop" (a wording defect
  from 15.2).
* **Nothing is rasterised behind the person's back.** Pixel layers, text, smart objects,
  adjustments, masks and whole-document edits are left out and named; a blend mode the W3C set
  lacks, clipping, a pass-through group with an opacity, an uneven scale under a stroke, and
  translucent overlaps in a linear-light document are written as nearest and named.

## A defect in the renderer, found by an independent renderer

Five shape documents were exported and rendered by `resvg`, then compared with PhotoForge's own
render of the same documents. The strokes document showed notched corners on a rectangle with a
**bevel** join: the stroker built a bevel on one side of the path only, which is the outside for
turns one way and the inside for turns the other. Every corner that turned the other way lost its
outer triangle (32 of 1,248 square pixels of a 40-unit square stroked 8 wide). Joins, caps and the
miter fallback were not at fault. `vector/stroke.rs` now builds the bevel on both sides, as the
miter join already did; a test covers both windings and fails without the fix.

| Document | Premultiplied mean difference (0–255) | Note |
| --- | --- | --- |
| Plain shapes | 0.07 | edges |
| Strokes of every cap and join | 0.09 (was 0.16 with the notches) | edges |
| Transforms and flips | 0.23 | edges |
| Even-odd and multiply | 0.39 | a multiply in linear light against sRGB |
| Opacity and a group | 5.35 | **understood**: two translucent shapes overlap; PhotoForge blends in linear light, an SVG viewer in sRGB |

## Defects found by checking, and fixed

| Found by | Defect |
| --- | --- |
| `resvg` | the one-sided bevel above |
| The first gate run after the mutation checks | a test that passed on its own failed in the gate: the mutation harness had restored a file with its old time, and cargo reused the last *mutant* build. The harness now refreshes the time; the gate was rerun on a rebuilt tree |
| Reading the report dialog | the approximated section of an *export* was titled "Imported, but not exactly as in Photoshop" |

## Verification

| Gate | Result |
| --- | --- |
| `cargo fmt --check` | clean |
| `cargo clippy --all-targets --all-features -- -D warnings`, and the same with no GPU / no inference / no plugins | clean (all four) |
| `cargo test --all-targets --all-features --no-fail-fast` | **1,654 passed**, 0 failed, 47 binaries (15.3a ended at 1,635) |
| No GPU / no inference / no plugins | **1,647 / 1,646 / 1,615 passed**, 0 failed |
| `npx tsc --noEmit`, `npm run check` | clean; 514 files, 0 errors, 0 warnings |
| `npx vitest run --maxWorkers=4` | 87 files, **1,308 passed**, 0 failed (15.3a ended at 1,303) |
| `npm run build` | built |

The tree was also scanned for control characters (none). What was added:

* **Rust**: 14 unit tests in `svg::export` (numbers, colours encoded to sRGB, strokes, open
  geometry, polygons and stars, the matrix against the compositor, flips, groups and visibility,
  what is left out, escaping, an empty canvas, the translucency note, joins and caps, the
  pass-through note), 4 through the command boundary, 1 in the stroker.
* **Frontend**: the Export dialog offers SVG; an SVG name goes to its own command and report and
  not to the picture or Photoshop path; the helper's name and sentence functions.
* **Mutation checks**: the translation left out of the matrix, colours not encoded, open geometry
  filled, names not escaped, a bevel written as a mitre, hidden layers shown, the pass-through
  note never made, and the stroker's bevel on one side — all eight caught (the pass-through
  note was first *not* caught, which is why it now has a test).

## What was not verified

* The file in **Inkscape, Illustrator or a browser**. `resvg` is one renderer.
* The interface in a real window.
* **Text as outlines**: not done.

## Next

SVG import of a bounded subset (15.3b, second part), OpenEXR (15.3c; a dependency review first),
clipboard (15.3d).
