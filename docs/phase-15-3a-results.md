# Phase 15.3a — metadata: EXIF, XMP and IPTC kept and controlled

The first part of 15.3 (metadata, EXR, SVG, clipboard). Until now an exported file carried none of
a photograph's metadata, whatever the person wanted, and a portrait photograph opened on its
side. The rules, the formats and the limits are in [metadata.md](metadata.md); this is what was
done and what was checked.

Status: **implemented and verified as listed; not committed, not released.** The version is still
0.14.0. On branch `phase-15-3-metadata`, on top of `6b208fe` (15.2).

## Completed

* **`src-tauri/src/metadata/`** (new): a bounded EXIF reader and in-place cleaner (`exif.rs`), IPTC
  (`iptc.rs`), an XMP inspector (`xmp.rs`), JPEG/PNG/WebP insertion (`container.rs`), a seek-based
  scanner that finds the three blocks of a file without decoding it (`scan.rs`), and the policy
  and the account of an export (`mod.rs`).
* **Opening** a JPEG, PNG or WebP keeps its metadata with the document and **turns the picture
  upright** from the EXIF orientation. Region and reduced opens of very large files carry the
  tag instead.
* **Three policies** on export: remove location and serial numbers (the default), keep
  everything, remove everything. Every export returns a `metadata` report: what was kept, what
  was removed, what the file had no place for. The edit is accounted for (upright orientation,
  new size, no stale thumbnail).
* **Every writer**: image export (8-bit, colour-managed, streaming 16-bit PNG, developed 16-bit
  PNG), the layered composite, batch, and Photoshop files (image resources 1058, 1060, 1028).
  Photoshop import reads them back.
* **Projects and recovery snapshots keep the metadata** as extra container entries that older
  versions ignore.
* **The interface**: a Metadata card (what the file carries, the policy and its reason), the
  policy in the batch panel, and the notice after an export naming what was removed.
* A stale line in the Color card ("Camera EXIF/GPS is not exported") and in `privacy.md` and
  `color-pipeline.md` is corrected; `privacy.md` now says a project holds the photograph's
  metadata, place included.

## Defects found by checking, and fixed

| Found by | Defect |
| --- | --- |
| The first end-to-end test | The JPEG decoder returns IPTC as the whole Photoshop resource block, not the stream; a reader that took it for the stream saw nothing. Now unwrapped, for JPEG and bare |
| The first unit run | An empty `SourceMetadata` defaulted to orientation 0, which looked like "not upright" and wrote an orientation block into files that had none |
| `clippy` | One needless `as_deref_mut` |
| Writing the tests | A form-feed character in an existing test's path (`\f` turned into a control character by a script in 15.2); raw control bytes in a new byte-string literal. Both replaced by escapes, and the whole tree is now scanned for control characters |

## Verification

| Gate | Result |
| --- | --- |
| `cargo fmt --check` | clean |
| `cargo clippy --all-targets --all-features -- -D warnings`, and the same with no GPU / no inference / no plugins | clean (all four) |
| `cargo test --all-targets --all-features --no-fail-fast` | **1,635 passed**, 0 failed, 46 binaries (15.2 ended at 1,586) |
| No GPU / no inference / no plugins | **1,628 / 1,627 / 1,596 passed**, 0 failed |
| `npx tsc --noEmit`, `npm run check` | clean; 512 files, 0 errors, 0 warnings |
| `npx vitest run --maxWorkers=4` | 86 files, **1,303 passed**, 0 failed (15.2 ended at 1,291) |
| `npm run build` | built |

**Real camera files.** 13 JPEGs from the MIT-licensed Piexif suite (Casio, Canon, Olympus,
Panasonic, Pentax/Ricoh, Sigma, Sony; three with a GPS directory) were scanned, cleaned under the
private policy and compared with **Pillow**: 13 of 13 cleaned (none refused), GPS, maker note,
serial numbers and thumbnail gone, every other tag with its original value, no removed value in
the cleaned bytes. Scratch only, nothing committed.

**Mutation checks**, each broken and then restored: the place left in the bytes (only its pointer
removed), the thumbnail left, serial numbers kept, orientation not applied on open, a
keep-everything default, an XMP packet that names a place kept, IPTC place fields kept, an
unplaceable block reported as kept. All eight were caught. A ninth guard pins that the `image`
decoder does *not* apply the orientation itself, because a future upgrade that did would turn
every photograph twice.

## What was not verified

* The files in **Lightroom, Photoshop, ExifTool or a phone**. The independent reader was Pillow.
* The **window**: the selector, the card and the notice are covered by component tests against a
  mocked command boundary and the commands by tests of the real commands.
* A **maker note that holds a place**: dropped whole under the private policy, which is the safe
  answer, but no sample had one.
* **Orientation on a very large file opened whole** costs one extra copy of the pixels.

## Decision for review

The default is **remove location and serial numbers**. It is one constant
(`MetadataPolicy::default`) and one in the interface (`DEFAULT_METADATA_POLICY`), and the choice
is remembered per machine once made. A photographer's own archive export would want *keep
everything* by default; a tool for sharing would not. It was set for the second because the
project's stance is private and local-first, and because every export says what it removed.

## Next

15.3b SVG (export of shape layers, import of a bounded subset), 15.3c OpenEXR (needs a dependency
review), 15.3d clipboard (Windows only, tested with real clipboard round trips).
