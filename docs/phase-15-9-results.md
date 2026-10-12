# Phase 15.9 — a RAW region is chosen on the picture, and is turned with it

15.7 turned a developed DNG upright and 15.8 cut it to the picture its camera declares, but both
left one case alone: a **region** of a sensor (the way a sensor too large to develop whole is
opened) was still chosen on the raw sensor, border included, and a portrait region opened on its
side. The chooser drew its rectangle on a preview of the sensor and sent it as sensor coordinates.
Now the chooser shows the picture the camera declares, turned upright, the rectangle is the
picture's own, and it is mapped back to the photosites it covers. The rules are in
[raw-development.md](raw-development.md) ("Choosing a region") and
[project-format.md](project-format.md).

Status: **implemented and verified as listed; not committed.** On branch
`phase-15-9-region-picture`, from `main` (`63eecd0`, the merge of 15.8).

## Completed

* **`PictureGeometry`** (`raw/dng.rs`, read from the header alone by `inspect_geometry`, so no
  sample is decoded): where the picture lies on the sensor (the same rule as
  `SensorImage::picture_rect`, which now shares one function with it), which way it is turned,
  its upright size (`presented_size`), and the mapping both ways: `to_sensor` (a rectangle chosen
  on the upright picture → the photosites it covers) and `from_sensor` (the inverse, for a region
  saved in sensor coordinates). `orientation::unplace` is the inverse of `place`.
* **The chooser is on the picture.** `probe_image_source` reports the picture's size
  (`Probed::presented`; the planner still prices the sensor), the preview (`source::dng::preview`)
  is cut to the picture in proportion and turned upright after it is built, and
  `source::open::gate_region` holds the rectangle to the picture it was chosen on.
* **A region is developed as the photosites it covers and turned like a whole open**
  (`open_raw_image`): the layer's `view` is those photosites in sensor coordinates, and
  `orientationApplied` is the turn it was given (which `RawLayerSource::validate` now allows with
  a `view`). The tag is spent for a region as for a whole sensor, so every export says upright.
* **Re-developing and exporting a region layer** (`develop_raw_layer`, `export_raw_layer_png16`)
  turn it the same way, so its raster keeps the size its masks and transform were made for.
* **Change Source Region** starts from where the layer's photosites are on the picture
  (`raw_region_on_picture`, a new command), or from the chooser's own default when they took in
  the masked border.
* **A region saved before this is developed as it was**: its `view` was chosen on the sensor and
  it records no turn, so re-developing gives the same raster; no migration, and a project that
  does not record a turn is still format version 2.

## Verification

| Gate | Result |
| --- | --- |
| `cargo fmt --check` | clean |
| `cargo clippy --all-targets -- -D warnings`, four feature sets | clean, all four (all features, no GPU, no inference, no plugins) |
| `cargo test --all-targets --no-fail-fast` | **1,986** / **1,979** / **1,978** / **1,947** passed, 0 failed (60 binaries), same four feature sets (15.8 ended at 1,973 / 1,966 / 1,965 / 1,934) |
| `npx tsc --noEmit`, `npm run check`, `npx vitest run`, `npm run build` | 0 errors, 0 warnings (523 files); **1,387** tests in 93 files passed; build succeeds. One unhandled error, the `CommandPalette` test that throws on purpose, unchanged from before |
| Control-character scan of the tree | 0 files |

**Unit tests:** `unplace` inverts `place` for all eight orientations on six shapes, and every
pixel of the upright picture is a destination. `PictureGeometry`: the header's picture is the
decoded sensor's for six ways of declaring (or not) a picture and four orientations;
**exhaustively**, for all eight orientations and every rectangle of a 10 × 8 picture at (7, 3),
`to_sensor` gives a rectangle of the sensor inside the picture holding exactly the photosites the
turn carries into the chosen one, and `from_sensor` gives the chosen rectangle back; a rectangle
off the picture, empty, or across its border is not mapped. `develop`: for all eight orientations,
a region developed from the mapped rectangle and turned equals that window of the whole upright
development, pixel for pixel, at odd offsets, an edge, a corner and almost all of it.
`raw`: a region may carry a turn, and a turn must be a real orientation.

**Through the commands** (`tests/raw_region_picture.rs`, 7): for all eight orientations a region
exported at 16 bits equals that window of the whole picture's export, pixel for pixel, with its
`view` inside the picture on the sensor and its turn recorded; re-developing at both scales keeps
size and turn; a region record from before regions were turned is developed unturned (5 × 8, not
8 × 5), in the export too; a rectangle that is on the sensor but not on the picture is refused
(four ways) and the whole picture is accepted; the probe reports 20 × 12 or 12 × 20 for a framed
sensor by its tag and the sensor's own size when nothing is declared; the preview is the picture
(12 × 20 exact on a small sensor, portrait with the picture's proportions when it is decimated);
`raw_region_on_picture` gives back what was chosen, and nothing for a region on the border or off
the sensor. Rewritten because they asserted the old behaviour: three tests in `raw_crop.rs`,
`raw_metadata_ipc.rs` and `raw_upright.rs`. Frontend: the Change Source Region test now has the
backend map the layer's rectangle (`App.oversized.test.ts`).

**A real photograph** (the Canon EOS 5D Mark III sample; scratch, not committed): for each of the
eight orientation values (the tag rewritten in a copy), a 48 × 40 window at an odd offset and a
33 × 21 window at the far corner of the 5760 × 3840 picture are mapped to the sensor, developed on
their own from that copy and turned; every photosite of the sensor rectangle lands inside the
window, and every pixel of the region is the picture's own pixel at that place (one full
development stands for all eight; the turn itself is the one checked against Pillow in 15.7).

**Mutation checks**: **27 of 27 caught**, each by the test that names it: the inverse of a quarter turn (6 and 8), of the transverse (7) and of a mirror (2); the picture offset dropped (x, y); a chosen width one short; a rectangle off the picture mapped; a sensor rectangle across the border placed; a sensor rectangle placed without the offset; the presented size ignoring the turn; the header geometry ignoring the orientation tag or the default crop; a region opened as the rectangle chosen and not the photosites; a region not turned; the layer recording the chosen rectangle as its view; the tag not spent for a region; the layer forgetting a region's turn; re-developing a region not turned; its export not turned; the place on the picture asked the wrong way; the chooser preview not cut, not turned; the probe giving the frame; the dialog told the sensor size; a region refused a turn again; and, in the interface, Change Source Region starting from the sensor rectangle.

## What was not verified

* **The chooser in a real window.** The dialog and the selector are unchanged and get a picture
  and its size; component tests run against a mocked command boundary, and the IPC tests cover
  what the dialog is given. Nothing was drawn on a real picture with a pointer.
* **A portrait DNG written by a real camera**: the only real file is upright, so the tag was
  rewritten in a copy (as in 15.7).
* **A DNG with a fractional or odd crop from a real camera**, and other makers' files (as in 15.8).
* **Memory at the limit**: a region's window and its turn (a second buffer of the region while
  the first is read) are within what the planner already prices per photosite; not measured.

## Known and left

* A region saved before this was chosen on the sensor, border included, and stays as it is. Change
  Source Region on one starts from the chooser's default when its rectangle took in the border.
* The chooser's preview is as sharp as it was, measured on the sensor: a picture much smaller
  than the sensor (a crop mode) is a smaller preview than the edge asked for.
* DefaultScale, BestQualityScale and non-square pixels are not applied; the masked photosites are
  not used to measure the black level (as in 15.8).
