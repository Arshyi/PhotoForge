# Phase 15.11 — three review items from the 0.15.0 ledger, fixed

The 0.15.0 ledger carried three items that were recorded, understood and not fixed: highlights
above white clipped by most blend modes, PSD memory that was estimated at less than it took, and
four gaps in the SVG round trip. Each is small and independent. This fixes them and says what was
decided where a fix was a decision.

Status: **implemented and verified as listed; integrated for 0.15.1.** Developed on
`phase-15-10-clipboard-window` with Phase 15.10 and 15.12, from `main` (`242f6f9`).
Current integration checks and additional PSD estimate repairs are recorded separately in
[0.15.1](release-0-15-1.md).

## 1. Highlights under the blend modes

**The decision.** The seven modes that map 0 to 1 into itself (Normal, Multiply, Screen, Darken,
Lighten, Difference, Exclusion) were already evaluated without a ceiling, and are unchanged. The
other modes are *defined on 0 to 1*: they pivot on mid-grey or clamp their own result, so there is
no honest extension above white (Photoshop's 32-bit mode does not offer them either). They are
still applied to the operands clamped to that range, and now **what the backdrop held above white
is added back**: `result = f(clamp(b), clamp(s)) + max(b − 1, 0)`.

* In range nothing changes, bit for bit (a test over every mode and a grid of operands).
* A backdrop highlight passes through instead of being flattened to white; a neutral source (grey
  under Overlay, Soft Light, Hard Light) leaves any backdrop alone, above white too.
* The result is continuous at white and never lower than the clamped result.
* A **source** above white counts as white under these modes, and its excess is not added: the
  modes disagree about what a source highlight should do, and a wrong guess is worse than none.
* Below 0 nothing changes (a wide-gamut negative is clamped as before).

`src-tauri/src/layers/linear.rs` (`blend_linear`), 7 tests in `highlight_tests`, documented in
`layers.md` and `openexr.md`.

## 2. PSD memory

* **Import decoded every channel.** A spot or alpha channel (id 3 and up in an RGB file, 1 and up
  in a grayscale one) is a plane as large as the layer, and a file can carry dozens: up to about 14
  times the per-layer estimate. Only the colour channels and transparency are decoded now. Tests:
  an RGB layer and a grayscale layer each carry extra channels deliberately too short for the
  layer, which is an error if decoded.
* **Export's peak was about three times what was guarded.** It counted the size of the file once,
  and held the layers' bytes three or four times (the layer data copied into the layer
  information, that into a padded copy, that into the section, that into the file). Two changes:
  * the file is **written as a list of pieces** (the small parts built in memory, the layers'
    channel data from where it already is) and never copied into one buffer; the flattened
    picture's planes are moved, not cloned, and the float picture is dropped once they exist;
  * the guard is on the **peak**, counted from the document before anything is made (the layers
    encoded, the largest one's planes while it is written, the picture as rendered, its planes and
    its encoding), in a function with its own tests. It is an estimate from what is held, not a
    measurement.
* **The bytes are unchanged.** Five files (8-bit with a mask, a group, a blend mode and a hidden
  layer, at 8, 16 and 32 bits; a high-precision document at 16 and 32) were exported before the
  change and after: **byte-identical** (SHA-256). The dump is a test that stays
  (`PHOTOFORGE_PSD_EXPORT_DUMP`). A sweep of widths and colours finds the exports whose layer
  information has an odd length, which need the pad, and reads each back.

## 3. SVG round trip

| Gap | Now |
| --- | --- |
| **Isolated groups were written as a plain `<g>`**, which SVG does not isolate | Written `style="isolation:isolate"` (unless opacity or a blend mode already isolates). Checked against resvg: a Multiply layer inside an isolated group, rendered by PhotoForge and by resvg, **identical (max difference 0)**; the same file with the plain `<g>` of 0.15.0 differs by 255 at the overlap (resvg black, PhotoForge blue) |
| **Colour names outside 40 read as no colour** | All 148 CSS names, equal to Pillow's own table (a cross-check at the time of writing: 148 of 148 names and values) |
| **`hsl()` read wrongly** | `hsl()` and `hsla()`, degrees or `deg`, `grad`, `rad`, `turn`, comma or space syntax, alpha as a number or a percentage |
| **Percentage lengths** of the canvas, not the view box | Of the view box (the canvas when there is none); `r` and a stroke width of the normalised diagonal. Before, `stroke-width="2%"` was 0 |
| **`<title>` not read back**, so layer names were lost on a round trip | A `<title>` child names the layer, else the `id` |

`isolation` and `mix-blend-mode` are still **not read** (every imported group is isolated, every
imported shape Normal).

## Verification

| Gate | Result |
| --- | --- |
| `cargo fmt --check` | clean |
| `cargo clippy --all-targets -- -D warnings`, four feature sets (all, no GPU, no inference, no plugins) | clean in all four (0 diagnostics) |
| `cargo test --all-targets --no-fail-fast`, same four | **2,015** / **2,008** / **2,007** / **1,976** passed, 0 failed (60 binaries each); 0.15.0 was 1,993 / 1,986 / 1,985 / 1,954. One env-gated test (the SVG corpus dump) was added after; clippy and the SVG tests were re-run |
| Real RAW fixtures set for the whole run | `raw_real_files` **10 of 10** (it was 9 of 10 on a fresh clone before the 15.8 test baseline was corrected, see the Phase 15 results) |
| **230 real icons** (Feather, Bootstrap Icons, Simple Icons), imported, rendered by PhotoForge and by resvg | **230 of 230 imported, none refused**; mean difference median 0.22, 90th percentile 0.86, worst 2.2 of 255: the same as the 0.15.0 figures (0.2, 0.9, 2.2). The new importer rules did not change a real file's render |
| Frontend | see 15.12 |
| **Mutation checks** (`tools/mutate_review_items.py`) | **18 of 18 caught** (17 first, one after a test was added; below) |

The mutants: a backdrop highlight not added back; a source highlight added too; Multiply clamped
again; in-range operands bounded below white; a backdrop below white pushed down; the importer
decoding every channel again; the importer reading three planes for grey; the guard forgetting the
rendered picture; the guard counting the largest layer once; the layer information not padded; the
layer data written before its records; the hsl hue sector wrong; a colour value changed; a title
not a name; the normalised diagonal not divided; percentages swapping the viewport's sides; an
isolated group written plain; a pass-through group written isolated.

**One survived the first run** (the padding): the sweep's contents never made the layer
information odd, and the check on what follows it was satisfied by a zero byte. The test now makes
the data odd (one varying channel, an even width), asserts that the padding was needed, and checks
that the image data starts where the section's length says. The mutant is then caught.

## What was not verified

* **Photoshop, Inkscape, Illustrator.** No file was opened in them. The PSD bytes are unchanged and
  were read by the project's reader; the SVG was read by resvg, which is the independent renderer.
* **Memory under real pressure.** The export guard is an estimate counted from what is held; a
  document at the budget was not exported while the process's peak was measured.
* **A source highlight under the clamped modes** is still clipped, by decision.
* `isolation` and `mix-blend-mode` on import.
* The PSD, EXR and TIFF corpora of other programs' test files (`PHOTOFORGE_PSD_SAMPLES` and the
  others) were not re-run for this change; no byte of an export changed (hashes above) and the
  importer reads fewer channels. (The SVG icon corpus was: table above.)

## Known and left

* Of the 0.15.0 ledger: the PSD export of a high-precision document is 16-bit sRGB and composited
  differently by Photoshop; a multi-part EXR with a part this reader lacks is refused as a whole;
  a project file can store `clipped: true` on a bottom layer.
* The mutation harnesses of earlier phases have 30 stale patterns (see `tools/README.md`).
