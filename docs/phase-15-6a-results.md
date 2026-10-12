# Phase 15.6a — the metadata of a DNG

The first half of "TIFF and DNG metadata". Until now a developed DNG exported with no EXIF, XMP
or IPTC at all, whatever the policy: the interface said the EXIF of a RAW source was "not read
yet". The rules, the formats and the limits are in [metadata.md](metadata.md); this is what was
done and what was checked. TIFF as a picture is 15.6b.

Status: **implemented and verified as listed; not committed.** On branch
`phase-15-6-dng-metadata-tiff`, on top of `main` (`2f82cee`, the merge of Phase 15).

## Completed

* **`src-tauri/src/metadata/tiff.rs`** (new): the EXIF, XMP and IPTC of a TIFF-structured source.
  A TIFF is the structure EXIF was copied from, so there is no block to cut out: the camera's tags
  sit in the first directory among the picture's own, the exposure in the Exif directory, the
  place in the GPS directory. The block is **built**: a new small TIFF holding the descriptive
  tags of IFD0 and the Exif, GPS and Interoperability directories, each value copied byte for
  byte under its own type and count, in the file's byte order, entries in tag order, values on
  even offsets. XMP (tag 700) and IPTC (tag 33723, else the Photoshop resources of 34377) are
  read as the blocks they are. The same function reads a TIFF picture when 15.6b opens one.
* **Opening a DNG** (whole or as a region) keeps that metadata with the document; the Metadata
  card, every export (single, layered, batch, Photoshop, project) and the policy apply as they do
  to a JPEG. The orientation is read and *not* applied (development leaves the sensor's own way
  up), so the tag travels with the pixels, under *Remove everything* too.
* **What cannot be moved is said.** The maker note (Exif 0x927C) and a DNG's private data (0xC634,
  where Adobe's converter keeps the camera's own) hold offsets into the file they came from. They
  are not carried, and every export that writes EXIF says so in its report; the summary still lists
  them as present. A field over 16 KiB, or past a 60,000-byte budget, is left and said likewise.
  `SourceMetadata` gained a `left_behind` field for this, which a project stores and reads back
  (older builds ignore the line).
* **Batch**: a `.dng` input now carries its metadata under the chosen policy. (Projects still do
  not: recorded in `phase-15-5-review-results.md`, unchanged.)
* **Fixtures**: the synthetic DNG builder can write extra IFD0 entries and an Exif and a GPS
  directory after the pixels, so tests have real directory structure to read.

## Verification

| Gate | Result |
| --- | --- |
| `cargo fmt --check` | clean |
| `cargo clippy --all-targets -- -D warnings`, four feature sets (all, no GPU, no inference, no plugins) | clean |
| `cargo test --all-targets --no-fail-fast` | **1,890 / 1,883 / 1,882 / 1,851 passed**, 0 failed (55 binaries; Phase 15 ended at 1,860 / 1,853 / 1,852 / 1,821) |
| Frontend (`vitest`, `tsc`, `svelte-check`, build) | not run: nothing under `src/` changed |
| Unit tests of `metadata::tiff` | 19: the block's contents and order, both byte orders, XMP terminator, IPTC from Photoshop resources, the maker note in either place, oversize and budget, a pointer outside the file, a pointer loop, unknown types, a repeated tag, even offsets; every prefix and 5 values at every byte of a file are read without a panic, and any block that comes out is walkable and within budget |
| Through the command boundary (`tests/raw_metadata_ipc.rs`) | 9: open, region open, three policies by `export_image`, the layered `export_layer_composite`, a DNG with no Exif directory, a damaged Exif pointer, a real file |
| Batch (`application::batch`) | 1 new: a DNG input under keep-all and the private policy |
| Mutation checks | **21 of 21** caught (below) |

**Real files.** The three Canon EOS 5D Mark III frames of the optional fixture set (one
photograph written by Adobe DNG Converter 9.8 uncompressed, lossless and lossy; 6,238-byte XMP, no
place, no IPTC, serial numbers in the Exif directory, the maker note in DNGPrivateData):

* The directories of each file were read by a second parser written for the purpose
  (`tiff_dump.py`, scratch) before any code was written, which is how the shape above was known.
* The block built from each was read with **Pillow** (`Image.Exif().load`): make, model, software,
  date, artist and orientation of IFD0, and **all 30** Exif tags, equal to the values read out of
  the DNG by the second parser; none of the sensor's tags in the block; the XMP byte for byte.
* The three exports of the uncompressed file (keep everything, the default, remove everything)
  were read with Pillow: the first has 30 Exif tags, the artist and a 6,238-byte XMP; the second
  has 28 (the body and lens serial numbers gone) and no XMP, because the packet carries
  `aux:SerialNumber`; the third nothing. The serial numbers are not among the bytes of the second
  and third files.

**Mutation checks** (the code broken one way at a time; each must fail a test):

| Broken | Caught by |
| --- | --- |
| Orientation not a descriptive tag; the maker note let through; private data not noticed; an Exif maker note not noticed | 5, 1, 1, 1 tests |
| No per-field cap; no block budget; oversize not flagged | 1, 1, 2 |
| Entries not sorted; values not padded; the Interoperability pointer not rebuilt | 1, 1, 1 |
| XMP terminator kept; Photoshop-resource IPTC not read; GPS directory not followed | 1, 1, 5 |
| Unknown field types copied; a repeated tag not skipped | 1, 1 (both **survived** until their tests were strengthened: they asserted "no model was read", which is also true when the entry is copied as the wrong type) |
| The summary ignores what was left; the export does not say it; a project forgets it | 1, 1, 1 |
| A RAW document holds no metadata; a RAW open shows no summary; a batch drops it | 4, 3, 1 |

## Defects found by checking

| Found by | Defect |
| --- | --- |
| The mutation run | Two tests were too weak (above); both now read the block's directory and not a summary of it |
| Reading the real files first | The maker note of an Adobe-converted DNG is not in the Exif directory but in tag 0xC634; a reader that only looked in the Exif directory would have said "no maker note" of a file that has one |

## What was not verified

* **The files written, in Lightroom, Photoshop, ExifTool or a phone's gallery.** The independent
  reader was Pillow, and Pillow is lenient.
* **DNGs from other makers.** One real converter's file stands for all. A DNG a camera writes
  itself may keep its maker note in the Exif directory (covered in a synthetic file only), carry a
  GPS directory or IPTC (no real sample had either) or be big-endian (synthetic only). IPTC stored
  as byte-swapped 32-bit integers is not undone; such a stream is unreadable and is not carried.
* **The interface in a real window**: the Metadata card for a RAW document is covered by the
  existing component tests of the card and by the IPC tests of what it is given, not driven in the
  packaged application.
* **A portrait DNG still opens on its side** (as before): the orientation is carried, not applied.
  Turning the developed picture upright changes its size relative to the sensor reference and to
  a region's rectangle, which is a design decision and is not made here.
