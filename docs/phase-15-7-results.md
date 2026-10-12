# Phase 15.7 — a developed DNG is turned upright

A portrait photograph in a camera RAW file is stored as the sensor saw it, with a tag saying how to
turn it. Until now PhotoForge read the tag and did nothing with it, so a portrait DNG opened on its
side (15.6a recorded this and carried the tag with the pixels so other programs showed it right).
Now a DNG developed whole is turned upright. The rules are in
[raw-development.md](raw-development.md) ("Orientation") and [metadata.md](metadata.md).

Status: **implemented and verified as listed; not committed.** On branch
`phase-15-7-dng-upright`, from `main` (`903472b`, the merge of 15.6).

## Completed

* **`src-tauri/src/orientation.rs`** (new): the eight EXIF orientations, in one place:
  `place` (where a pixel goes), `oriented_size`, and `turn` (the mirrors and the half turn in
  place, the four that exchange the axes into a second buffer). The TIFF importer now uses it
  instead of its own copy, so a TIFF opened whole and a RAW developed whole are turned by one rule.
* **`DevelopedRaw::turned_upright`, `develop_sensor_upright`, `develop_bytes_upright`**: a whole
  sensor is turned as the last step of development. `source_dimensions` still names the sensor as
  it is.
* **Every whole-sensor path turns:** `open_raw_image` (whole), `open_raw_layer`, re-developing a
  layer (`develop_raw_layer`, at preview and at full scale), the layer's 16-bit PNG export, and
  batch DNG inputs. The tag is spent (`orientation_applied`), so every export says upright under
  every policy.
* **The layer records the turn** (`RawLayerSource.orientation_applied`, 2 to 8, written only when
  there is one): re-developing turns the same way, so the raster keeps the size its masks and
  transform were made for. A layer saved before this has no record and is developed as it always
  was. A project that records a turn is written as format version 3, which an earlier build
  refuses by name.
* **A region is not turned.** Its rectangle (and the preview made to choose it) is in the
  sensor's coordinates; it stays in the file's orientation and the tag goes with its pixels, as a
  region of a very large JPEG does. A portrait region still opens sideways.

## Verification

| Gate | Result |
| --- | --- |
| `cargo fmt --check` | clean |
| `cargo clippy --all-targets -- -D warnings`, four feature sets (all, no GPU, no inference, no plugins) | clean |
| `cargo test --all-targets --no-fail-fast` | **1,953 / 1,946 / 1,945 / 1,914 passed**, 0 failed (58 binaries; 15.6 ended at 1,937 / 1,930 / 1,929 / 1,898). Five 15.6a tests (`raw_metadata_ipc.rs`) asserted the old behaviour (a whole DNG keeps its tag, remove-everything writes the orientation) and were rewritten to the new one |
| `npx tsc --noEmit`, `npm run check`, `npx vitest run`, `npm run build` | clean; 523 files, 0 errors, 0 warnings; 93 files, **1,387 passed** (one unhandled error from `CommandPalette.test.ts`, as at `main`); built |
| Control-character scan of the tree | 0 files |

**Unit tests** (`orientation::`): every one of the eight values on a 3 × 2 picture against an
explicit table and against `place`; opposite quarter turns, two half turns, two mirrors and two
transposes cancel; an impossible orientation or pixel count leaves the picture alone; a picture one
pixel wide. **`raw::`**: the record is written only when there is a turn and reads as 1 when absent;
a region with a turn, and an orientation of 0 or 9, are refused.

**Through the commands** (`tests/raw_upright.rs`, 8): the sensor's top-left block lands in the
corner EXIF says for each of the eight values, and the size is exchanged for 5 to 8; an upright
sensor is unchanged; a portrait DNG opens 24 × 32 from a 32 × 24 sensor, says it was turned, and
exports upright under all three policies; re-developing at both scales and the layer's PNG export
keep the size and the turn; a record from before the field is developed as it was; a region is not
turned and exports with the tag; a layer added to a document is turned and says so; a project that
records a turn is format version 3 and round-trips it, and one that does not is version 2. Batch:
one test (a DNG is 4 × 8 from an 8 × 4 sensor, tag 1).

**A real photograph** (the Canon EOS 5D Mark III sample; scratch, not committed): its orientation
tag was rewritten in a copy to each of 2 to 8 and each was developed upright. Each is **bit for bit
identical to Pillow's transpose of the unturned development**, channel by channel in float (Pillow's
own `exif_transpose` table: flip, rotate, transpose, transverse), and 1480 × 986 becomes 986 × 1480
for 5 to 8 as it should.

**Mutation checks**: 17 breakages (each of the half turn, the two mirrors, the transpose and a quarter turn; the sensor developed with no turn; a whole open, a layer added, a re-development and the layer's PNG export not turned; the tag not spent; the layer forgetting its turn; a batch not turned or not spending the tag; a recorded turn written as version 2; a region allowed to be turned; a no-turn record always written) are each caught.

## What was not verified

* **A portrait DNG from a real camera.** The only real DNG available is upright; the tag was
  rewritten in a copy. What a camera writes for each value is the specification's, which is what
  Pillow's table is.
* **A project made before this** was exercised with a hand-made record that lacks the field (the
  shape an earlier build writes), not with a project file from an earlier build.
* **The interface in a real window**: the document's size and the Metadata card are covered by the
  IPC tests of what they are given; the window was not driven.
* **Memory at the limit**: the turn of a 100-megapixel sensor was not measured. Its peak (two
  16-byte pixels) is within what development already counts, by that count and not by measurement.

## Known and left

* **A region of a portrait sensor opens sideways**, as above: choosing the region on an upright
  preview and mapping it back to the sensor is a change to the region chooser and the planner, and
  is not made here.
* The sensor's **DefaultCrop and active area** are still not applied (the 5920 × 3950 sensor of
  the Canon sample develops to 5920 × 3950, not its 5760 × 3840 default crop). Not new; not
  changed.
