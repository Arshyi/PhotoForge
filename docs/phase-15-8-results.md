# Phase 15.8 — a developed DNG is cut to the picture its camera declares

A sensor has masked and optically black photosites around the image, and a DNG says where the
image is: an active area and a default crop. PhotoForge read both tags and applied neither, so
every RAW picture carried the border (the Canon sample developed to 5920 × 3950, not its 5760 ×
3840). Now a DNG developed whole is cut to the picture it declares. The rules are in
[raw-development.md](raw-development.md) ("The picture") and [project-format.md](project-format.md).

Status: **implemented and verified as listed; not committed.** On branch `phase-15-8-dng-crop`,
from `main` (`0db3538`, the merge of 15.7).

## Completed

* **`SensorImage::picture_rect`**: the default crop (relative to the active area's corner) inside
  the active area; the active area when no crop is declared or the crop is not inside it; `None`
  when that is the whole sensor.
* **`DevelopedRaw::cropped`**, **`develop_sensor_picture`**, **`develop_bytes_picture`**: a whole
  sensor is cut to that rectangle after demosaicing, then turned upright (these replace 15.7's
  `develop_sensor_upright` / `develop_bytes_upright`, which only turned). A preview is cut in
  proportion to how much the sensor was reduced; at full scale the cut is exact.
* **Every whole-sensor path cuts:** `open_raw_image` (whole), `open_raw_layer`, `develop_raw_layer`
  at preview and full scale, the layer's 16-bit PNG export, and batch DNG inputs.
* **The layer records the rectangle** (`RawLayerSource.crop`, in sensor coordinates, written only
  when there is one): re-developing cuts the same rectangle, so the raster keeps the size its
  masks and transform were made for. A layer saved before this has no record and is the whole
  sensor as it always was. `validate` refuses a crop off the sensor, and a crop on a region. A
  project that records a cut is format version 3 (an earlier build refuses it by name).
* **A region is not cut**, like it is not turned: its rectangle and the preview made to choose it
  are in the sensor's own coordinates, border included.

## Verification

| Gate | Result |
| --- | --- |
| `cargo fmt --check` | clean |
| `cargo clippy --all-targets -- -D warnings`, four feature sets | clean, all four (all features, no GPU, no inference, no plugins) |
| `cargo test --all-targets --no-fail-fast` | **1,973** / **1,966** / **1,965** / **1,934** passed, 0 failed (59 binaries), same four feature sets |
| `npx tsc --noEmit`, `npm run check`, `npx vitest run`, `npm run build` | 0 errors, 0 warnings (523 files); **1,387** tests in 93 files passed; build succeeds. One unhandled error, the `CommandPalette` test that throws on purpose, unchanged from before |
| Control-character scan of the tree | 0 files |

**Unit tests:** `picture_rect` for: nothing declared, an active area alone, a crop relative to the
active area, a crop with no active area, a picture that is the whole sensor, a crop outside the
active area, empty and off the sensor, and an active area that is not on the sensor.
**`develop`**: the picture is the whole development cut at that place, pixel for pixel, **and is
the development of that rectangle on its own** (`develop_region`), at odd offsets on purpose; the
cut comes before the turn; a sensor that declares nothing is developed whole; a preview is cut in
proportion and never empty. **`raw`**: the record is written only when there is a cut and reads as
none when absent; a crop off the sensor, empty, or on a region is refused.

**Through the commands** (`tests/raw_crop.rs`, 9): a 32 × 24 sensor opens as the 20 × 12 picture
with the rectangle recorded and the sensor still described as it is; the cut comes before the turn
(12 × 20 for orientation 6); re-developing at both scales and the layer's PNG export keep the size
and the record; a record from before the field is the whole sensor; a region is not cut; a sensor
that declares nothing is whole; a RAW layer added to a document is cut; an impossible record is
refused; a project that records a cut is version 3 and one that does not is version 2, and the
record survives a reload. Batch: one test (a 16 × 12 sensor is 10 × 6).

**A real photograph** (the Canon EOS 5D Mark III sample; scratch, not committed): its active area
(5796 × 3870 at (122, 80)) and default crop (5760 × 3840 at (18, 16)) were read out of the file by
a separate parser before any code was written; PhotoForge's picture is the rectangle
(140, 96, 5760, 3840), developed whole at full scale it is **5760 × 3840** (the camera's published
image size), a preview is cut in proportion, and a 48 × 48 window of it is **bit for bit** the
development of that window of the sensor on its own.

**Mutation checks**: **16 of 16 caught**, each by the test that names it: the active area's origin ignored; a crop outside the active area believed; a picture that is the whole sensor treated as a cut; the left edge not cut; the right edge taken as the width; the picture not cut; a whole open not cut; the layer forgetting its cut; re-developing not cutting; the layer's PNG export not cut; an added RAW layer not cut; a batch not cut; a recorded cut written as version 2; a region allowed to be cut; a crop off the sensor allowed; no-cut always written..

## What was not verified

* **A DNG with a fractional or odd crop from a real camera.** The Canon sample has whole, even
  offsets; odd offsets are covered in synthetic files (and the region identity is asserted there),
  fractional values are floored as the reader always has.
* **A DNG whose active area is not at an even offset to its CFA pattern**, which the DNG
  specification says a writer should avoid; the cut is after demosaicing, so the pattern is
  unaffected either way, but no such file was available.
* **Other makers' DNGs**: one real converter's file stands for the rest.
* **The interface in a real window**, and **memory at the limit** (the cut makes a second buffer
  while the first is read, within what development already counts; not measured).

## Known and left

* A region is still cut from the sensor, border included, and a portrait region still opens
  sideways (15.7). Both are one change: drawing the region on the picture and mapping it back.
* DefaultScale, BestQualityScale and non-square pixels are not applied.
* The masked photosites are not used to measure the black level (the file's black level tag is).
