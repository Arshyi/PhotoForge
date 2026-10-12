# Phase 15.6b — TIFF

The second half of "TIFF and DNG metadata" (the first is [phase-15-6a-results.md](phase-15-6a-results.md)).
PhotoForge opens a TIFF as a new, unsaved document with a report of what the file held that it
has no place for, and writes the composite as a TIFF through the same export path as PNG, JPEG
and WebP. The rules, the formats and the limits are in [tiff.md](tiff.md); this is what was done
and what was checked.

Status: **implemented and verified as listed; not committed.** On branch
`phase-15-6-dng-metadata-tiff`, on top of `main` (`2f82cee`) and the 15.6a work beside it.

## Decision: no TIFF library

The `tiff` crate was weighed and not taken. The build has no TIFF code today (`image` is built
with `jpeg`, `png` and `webp` only), so it would be a new family of packages (`tiff`, `weezl`,
and codecs behind it) reading a stranger's file; the part of the format photographs and scans
are written in is small; and everything it needs that is not small (Deflate) is already a direct
dependency (`miniz_oxide`, for the plugin-package reader). The reader is written here, bounded
as the others are: **no package is added** (`Cargo.toml` and `Cargo.lock` are unchanged).

## Completed

* **`src-tauri/src/tiff/`** (new): `structure.rs` (header and directories, read by seeking,
  bounded), `codec.rs` (PackBits, LZW, Deflate, both predictors; each told how many bytes it may
  produce), `import.rs` (layout, colour plan, admission, segments into an upright raster, report),
  `metadata.rs` (the picture's EXIF, XMP and IPTC), `export.rs` (the writer).
* **Open**: `import_tiff_document`, `.tif`/`.tiff` in the Open dialog and by drop, the shared
  report dialog ("TIFF import"). Strips and tiles; chunky and planar; 1, 2, 4, 8, 16 and 32-bit
  integers and 16, 32 and 64-bit floats; grey, RGB and palette with associated, unassociated or
  no alpha; none, PackBits, LZW and Deflate; both predictors; both byte orders; an embedded RGB
  profile; the orientation turned upright. CMYK, YCbCr, L\*a\*b\*, camera mosaics, JPEG-in-TIFF,
  CCITT, BigTIFF and the rest are refused **by name**.
* **Colour** follows the Photoshop importer's rule (the same sRGB-equivalence test is shared, not
  copied): 8-bit with no profile or an sRGB one stays 8-bit; deeper, floating-point and other
  profiles become high-precision documents; untagged 16-bit is read as sRGB, untagged float as
  linear light.
* **Save**: `.tif`/`.tiff` is an output format of the shared exporters (`export_layer_composite`
  and `export_image`, through `save_image_with_metadata` and `save_color_image_with_metadata`;
  the batch panel's formats are still chosen by profile and do not offer it): 8 or 16 bits, RGB or RGBA (straight alpha), Deflate with
  horizontal differencing, the chosen colour space's ICC profile, and the camera's metadata under
  the chosen policy (descriptive tags, Exif and GPS directories, XMP, IPTC). The maker note is not
  written, and the report says so.
* **One metadata path**: `metadata::tiff` gained `Parts`/`assemble` (what both the DNG reader and
  the TIFF reader feed) and `split` (the inverse, for writing).

## Verification

| Gate | Result |
| --- | --- |
| `cargo fmt --check` | clean |
| `cargo clippy --all-targets -- -D warnings`, four feature sets (all, no GPU, no inference, no plugins) | clean (two lints in the new code were found and fixed) |
| `cargo test --all-targets --no-fail-fast` | **1,937 / 1,930 / 1,929 / 1,898 passed**, 0 failed (57 binaries; 15.6a ended at 1,890 / 1,883 / 1,882 / 1,851). The no-inference run failed two `components::ollama` tests (a localhost mock server and a refused connection) once, under load; they passed three times alone and in the other three sets, and the whole no-inference run was repeated clean (1,929). Nothing in them touches this work |
| `npx tsc --noEmit`, `npm run check` | clean; 523 files, 0 errors, 0 warnings |
| `npx vitest run` | 93 files, **1,387 passed**, 0 failed (one unhandled error from `CommandPalette.test.ts`, as at `main`) |
| `npm run build` | built |
| Control-character scan of the tree | 0 files |

**Unit tests**: 71 have `tiff::` in their name (52 of the reader and writer, 19 of the metadata builder they share), of which the reader's matrix alone is 160 files (8 and 16 bits ×
five compressions × predictor × planar × tiled × both byte orders, compared sample for sample),
floating point in three widths with and without the predictor, grey / white-is-zero / 1-, 2-
and 4-bit / palette, the three alpha conventions, all eight orientations, profiles (sRGB stays
8-bit; Display P3 converts), and every refusal by name. Hostile input: every prefix and three
changed values at every byte of a tiled, LZW, differenced 16-bit file read without a panic;
strips that overlap (a table declaring 64 × 60,000 bytes of a 4 KiB file) and a tile of four
billion by four billion are refused before a buffer exists.

**Through the commands** (`tests/tiff_ipc.rs`): open with report and metadata, 16-bit as a
high-precision document, the layered export written and read back (8-bit exact, 16-bit Display P3
within 0.01 after the profile), the source file never replaced, refusals by name.

**Against programs that are not PhotoForge** (scratch, nothing committed):

* **Reading**: 70 files written by `tifffile` and by libtiff through Pillow (strips and tiles,
  chunky and planar, none / PackBits / LZW / Deflate, both predictors, both byte orders, 8-bit,
  16-bit, half, single and double floats, the three alpha conventions, grey, white-is-zero,
  bilevel, palette, all eight orientations, XMP and IPTC, a second page; JPEG-in-TIFF and CMYK
  refused by name). Every one read as its writer meant, to within 1/255 for the palette and
  associated-alpha cases and exactly otherwise. The orientation expectations were themselves
  checked against Pillow's `exif_transpose`.
* **Writing**: seven files PhotoForge wrote (8- and 16-bit, RGB and RGBA, uncompressed and
  Deflate, with a Display P3 profile, in 97-row strips, and with a camera's EXIF, XMP and IPTC)
  read by **tifffile and Pillow**: pixels exact, compression, predictor, photometric, planar
  configuration, extra-sample type, strip layout and ICC bytes as written; make, model, the Exif
  directory (exposure, ISO, date, serial number, lens) and the GPS directory equal to what was
  put in; XMP and IPTC byte for byte.

**Mutation checks**: 33 breakages of the new code (an orientation, white-is-zero, premultiplied
alpha, the float predictor's byte order, sRGB decoding, the admission budget, the overlap check,
reduced copies, alpha kinds, depth scaling, palette scaling, non-finite counting, the profile,
tile placement, planes, fill order, maker-note and byte-order bookkeeping, LZW's code-width
change, PackBits runs, both differencing directions, the read bound, the strip's alpha tag, the
profile embed, the maker-note report, the camera block, the byte swap of a block, and the two
export hooks) are each caught. One survived until its test was strengthened (the palette test
used only the extremes, 0 and 65535, so a wrong scale clamped to the same answer).

## Defects found by checking, and fixed

| Found by | Defect |
| --- | --- |
| Thinking through hostile input before the corpus | The segment table was built whole before any strip was decoded: a file declaring four million strips cost about 170 MB first. It is now made one segment at a time |
| The same | A tile of four billion by four billion overflowed the size arithmetic (a debug panic, and in release a wrong answer to the admission check). Saturating, so it is refused by the budget; test |
| The same | Strips could overlap: four million strips each declaring 64 KiB of one 64 KiB region would be read and inflated four million times. The declared lengths together may not exceed the file; test |
| The first corpus run | A generator bug (separate planes written from an interleaved array), not a reader bug; fixed in the generator |
| The mutation run | The palette test could not see a wrong scale (above) |

## What was not verified

* **A TIFF out of Lightroom, Photoshop or a scanner's own software.** None was available. The
  writers that were used (tifffile, libtiff) cover the format's mechanisms, not those programs'
  choices; a Photoshop TIFF with layers is covered only by its tag being recognised.
* **The files written, in Photoshop, Lightroom or Windows' own viewers.** They were read by
  tifffile and Pillow (libtiff) and by this program.
* **IPTC byte order**: written as an UNDEFINED value (the stream as it is); a reader that expects
  the older 32-bit-integer spelling of the tag may swap it. Pillow and tifffile read it as written.
* **The interface in a real window**: the Open and Export dialogs and the report are covered by
  component tests against a mocked command boundary (`src/App.tiff.test.ts`).
* **Large files**: nothing above a few megapixels was decoded in the tests, so the speed on a
  100-megapixel scan is unmeasured. Memory is bounded by the budget, and a strip at a time.

## Not done

BigTIFF; JPEG, CCITT, JPEG 2000, LZMA and Zstandard compression; CMYK, YCbCr and L\*a\*b\*;
signed-integer samples; region and reduced-size opens of a TIFF too large for the budget (it is
refused with the size and the budget, as an oversized EXR is); writing tiles, several pages,
thumbnails or a resolution; reading Photoshop's layers out of a layered TIFF; placing a TIFF as a
layer in an open document (the Place dialog still lists PNG, JPEG and WebP).
