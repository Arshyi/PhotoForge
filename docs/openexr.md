# OpenEXR

PhotoForge opens **OpenEXR** files (`.exr`) as high-precision documents and writes the document,
composited, as an OpenEXR file. OpenEXR is the format of renders and compositing: scene-linear
light, half or single precision, values above one, premultiplied alpha. PhotoForge's
high-precision document is the same kind of thing, so this is mostly a change of *convention*, not
of data, and it is done explicitly in both directions.

## Opening an EXR

**Open** an `.exr` (it is offered in the Open dialog, and a dropped `.exr` goes the same way) and
it becomes a new, unsaved **high-precision (linear float) document**, with a report of what the
file held that PhotoForge has no place for. The file is not changed; **Save** asks where to put the
PhotoForge project.

### What is converted

| In the file | In PhotoForge |
| --- | --- |
| Scene-linear light, any value (above 1, below 0) | Kept, unclipped. A highlight at 40 times white is 40.0 |
| Alpha premultiplied into the colour (the OpenEXR convention) | Divided out: the document uses straight alpha |
| Colour primaries other than Rec. 709 / sRGB (a `chromaticities` attribute: ACEScg, ACES 2065-1, Rec. 2020, XYZ…) | Converted to sRGB primaries, with the white point adapted (Bradford). Colour outside sRGB comes out below 0 or above 1 and is kept. Said in the report |
| No `chromaticities` attribute | Rec. 709, which is what the format says it means |
| Half (16-bit) or single (32-bit) samples | Single, in the document; the report says which the file stored |
| A data window that is not the display window | The canvas is the display window; the picture sits where the file puts it (a layer position) |
| A value that is not a number, or is infinite | Not-a-number becomes 0, infinity becomes 65,504 (the largest half-float). Counted in the report |
| Alpha outside 0 to 1 | Clamped; counted |
| Colour with no alpha behind it (light *added* to the picture) | Dropped, because straight alpha cannot hold it; counted |
| An alpha channel that is zero at *every* pixel, over colour that is there (a channel written and never filled) | Ignored: the picture is opaque, and the report says so. Taken at its word the picture would be invisible |
| Pixel aspect other than 1 | Shown square; said |

### What is read

* **Colour sets**, found by channel name: `R G B` with an optional `A`; the long names some
  compositors write (`red green blue alpha`); a luminance `Y` with an optional `A`, which is a grey
  picture. A set may sit under a prefix (`beauty.R`, `ViewLayer.Combined.R`).
* **Every complete set in the first part becomes a layer**, the unprefixed one first and visible,
  the others (render passes) **hidden and named**, up to 16.
* **Tiled files and mip/rip maps** (the largest level is read), scanline files, and the
  compressions **none, RLE, ZIPS, ZIP, PIZ, PXR24, B44 and B44A**.
* Display-window and data-window offsets (the OpenEXR project's own `DisplayWindow` test files).

### What is left out, and named

* **Deep data** (several samples at each pixel): refused, named.
* **DWAA, DWAB and HTJ2K compression**: refused **by name, from the header**, before any pixel is
  read. (Save the file again with ZIP, PIZ or none.)
* **Luminance/chroma images** (`Y`, `RY`, `BY`, the chroma at half resolution): refused, named.
  Reading the `Y` alone would show the picture without its colour.
* **Subsampled channels** and **sets whose colour channels have different sample types**.
* **A set of 32-bit integers** (object identifiers, not light).
* **Channels that are not colour** (depth `Z`, masks, identifiers…): listed by name, not read.
* **Other parts of a multi-part file**: named; only the first part with a picture is imported.
  (The reader finds a part by its channel names, so a file whose earlier part has the *same*
  names but a codec this reader lacks is refused as a whole, with the codec named.)
* **Header attributes** other than the primaries, the windows and the pixel aspect.

### Bounds

A file is admitted from its **header** before any pixel is decoded: the data and display windows
are checked against the largest canvas and the working-pixel limit, and one layer's pixels (16
bytes each) against the entry, store and job budgets of the resource manager. So is the memory the
reader needs to decode **one block**, which the header names, not the picture: a tile size, the
lines a codec packs, and the bytes of *every* channel, read as colour or not. The block as the file
may claim it must fit one buffer (the entry budget), and it read and decoded (up to four copies of
it in the worst codec, a row of pixels and the codecs' tables) beside the pictures must fit the job
budget. Run-length data is the exception to the four copies: the reader does not stop it at the
size the block is meant to be, so it is costed at what its runs can expand to, 192 times its
stored size (64 times, held three times over). That refuses a run-length block of more than about
20 MiB at the default 4 GiB budget (a tile about 1,150 pixels square in four single-precision
channels); a scan-line block is one line, and a tile 256 pixels square in the same channels is
1 MiB. This is worked out for every part of the file that is not deep data, because the part a
block belongs to is read from the block itself. A picture that would not fit beside the first is
left out and named; a first picture that does not fit is refused. A file over 4 GiB is refused
before it is opened. A file whose **header** (every attribute of every part) is longer than 1 MiB
is refused as well: the reader keeps all it parses, and a header of tiny attributes is held as
about a hundred times its length, so what it may read is capped, not left to the size of the file
(a thousand channels are 37 KB). An allocation is never made from a size the file claims without
that check.

## Writing an EXR

Choose **Export**, then *OpenEXR (scene-linear float, composited)* in the file dialog, or give the
file a `.exr` name. The picture-format choices (profile, colour space, dither) do not apply: the
file holds **one picture of scene-linear light**, and nothing is tone-mapped or clipped. The
document on screen is not changed.

| In PhotoForge | In the file |
| --- | --- |
| The visible layers, composited as PhotoForge composites them, with the document's own edits over them | One picture |
| Linear light on sRGB primaries, straight alpha, unbounded | Single precision (32-bit float), colour **premultiplied** by alpha, as the format stores it |
| An 8-bit (sRGB-encoded) document | Decoded to linear light; the report says the file carries only the 256 levels the document had |
| Rec. 709 primaries | A `chromaticities` attribute stating Rec. 709, so a reader that honours it and one that assumes it agree |
| Opaque picture | `R G B` only, no alpha channel |
| Translucent picture | `R G B A` |
| Layers, masks, adjustment layers, groups | Composited: **said in the report**, because an EXR here holds no layers |
| EXIF, XMP, IPTC | Not written (no place for them); the export reports what the policy would have kept as not stored |

The compression is ZIP in blocks of 16 scan lines, which every OpenEXR reader has and which is
lossless. The writer names itself in the `software` attribute.

**Half precision** is written by the command when `bitDepth` is 16 (values beyond ±65,504 are
clamped and counted in the report). The interface writes single precision; it has no control for
half yet.

## Dependency

Reading and writing use the `exr` crate, **version 1.74.0 exactly**, with its default features
off (no `rayon`):

* BSD-3-Clause, `#![forbid(unsafe_code)]`, an independent Rust implementation of the format.
* **Four packages are new to the build**: `exr`, `lebe` (byte order; BSD-3-Clause; it contains
  `unsafe`), `bit_field` (MIT) and `zune-inflate` (deflate decompression; Zlib-licensed here; no
  `unsafe`). `half`, `miniz_oxide`, `smallvec` and `simd-adler32` were already in the build.
* **1.74.1 and 1.74.2 were deliberately not taken.** They add the DWA codecs, which bring `pulp`
  (SIMD) and `num-complex`, and are a month old; the codecs are a feature this release does not
  need. The version is pinned with `=` so that an update is a decision.
* `cargo audit` against the advisory database dated 3 August 2026 reports nothing for the new
  packages (the same 18 allowed warnings as before, none of them these).
* The notices are in `THIRD_PARTY_NOTICES.md`.

Everything EXR-specific that is not the container (the channel sets, the conventions, the
primaries, the admission, the report) is PhotoForge's own code in `src-tauri/src/openexr/`.

## How it was checked

* **Unit tests** (71): each conversion above on its own (premultiplied colour, values above one and
  below zero, light without alpha, alpha outside 0 to 1, not-a-number and infinity, a tiny alpha
  that must not become infinity, the primaries matrix applied to straight colour); the channel
  sets (long names, a prefix, a pass that must not borrow the main picture's alpha, integer
  channels, no colour at all, luminance/chroma); the windows; a multi-part file; every codec the
  reader has; tiled files; DWAA, DWAB and HTJ2K refused by name from a patched header; deep data;
  headers that claim more than the budget, and blocks they name that it cannot hold (a tile 2²⁹
  pixels square in a 4 × 4 file, 61 channels nobody reads, another part's block, run-length data
  whose runs step over the size of its block, with the heap it holds measured); a header at the
  1 MiB cap and one byte over it, and one of tiny attributes, with the heap it holds measured;
  sets that would not fit; the 16-set bound; a file that is not an EXR; a sparse file claiming
  more than 4 GiB; **every truncation and a changed byte of a file at every position, none of
  which panics**.
* **The primaries matrix against published numbers.** ACEScg (AP1), ACES 2065-1 (AP0, whose blue
  has a *negative* y) and Rec. 2020 each reproduce the matrix the ACES documentation and BT.2087
  list, and the rows of every matrix sum to one (white stays white).
* **Through the real commands** (6): the names, the arguments the interface sends, the document and
  report that come back, a file written and read back, half and single precision, the refusals
  (not `.exr`, not absolute, no such folder, an invalid document leaves an existing file), and a
  file left beside nothing.
* **In the interface** (16): the Open and Export dialogs, the routing away from image admission and
  the other importers, the report wording, the notification, a refusal.
* **Against the OpenEXR reference implementation.** 62 files from the OpenEXR project's own
  `openexr-images` set (BSD-3-Clause, downloaded to scratch, not committed) were imported and
  exported by PhotoForge and read again by the reference reader (OpenEXR 3.5.2 for Python), with
  the same conventions applied by an independent implementation:

  | | |
  | --- | --- |
  | Imported and exported | **52 of 62** |
  | Refused, each for a stated reason | 10: 6 luminance/chroma, 3 deep, 1 with no `R G B` or `Y` (a lone `G`) |
  | Largest imported difference from the reference | **3.6 × 10⁻⁷** (relative, to `max(|v|, 1)`), on the file whose primaries are XYZ |
  | Exported single-precision file against the composite | **bit for bit** on every file |
  | Exported half-precision file | within half-float precision (largest 4.9 × 10⁻⁴) |
  | Exported header | data window = canvas, chromaticities = Rec. 709, ZIP, on every file |
  | Alpha channel present exactly when the picture has transparency | every file |

  The 52 include the `DisplayWindow` files t01 to t16 (data windows inside, outside and offset
  from the display window), the multi-resolution files, tiled files, files with NaN and infinity,
  a stereo pair (the `right` view comes in as a hidden layer), a wide-gamut file, a file whose
  primaries are XYZ, and `Carrots`, an ACES 2065-1 file whose alpha channel was never filled (the
  case the "alpha zero everywhere" rule exists for; it first opened as a clear sheet, found by
  this comparison).
* **Mutation checks**: 27 deliberate breakages (premultiplication not undone or not applied, the
  primaries ignored or the white point not adapted, DWAA let through, a luminance/chroma file read
  as grey, an unfilled alpha obeyed, a pass made visible, the data-window offset ignored, infinity
  passed on, each admission limit removed, integer sets read, deep data let through, half precision
  not clamped, an alpha channel always written, and so on) are each caught. One of them survived the
  first run, which found a test that passed for the wrong reason: the importer turns an infinity
  into 65,504, which hid a half-float clamp that had never been made. The test now reads the file's
  samples raw.

## Highlights above white under blend modes

An EXR is kept unclipped in the document, and **Normal, Multiply, Screen, Darken, Lighten,
Difference and Exclusion** keep it unclipped when layers are composited. **Every other blend mode
is defined on 0 to 1**, and is applied to the operands clamped to that range **with what the
backdrop held above white added back**: a highlight under Overlay, Soft Light, Vivid Light and the
like passes through (it is not flattened to white), and the mode still acts on the part of the
picture that is in range. A source above white counts as white under these modes. Before Phase 15.11 a
layer in one of them lost the highlights beneath it. A single-layer EXR is unaffected. (See
[layers.md](layers.md).)

## Not checked

* The files PhotoForge writes **in Nuke, Blender, After Effects or Photoshop**. They have been
  read by the OpenEXR reference implementation, which is the strongest reader there is, and by
  this program; they have not been opened in those applications.
* The interface in a real window (component tests only).
* Files larger than the 62 samples (the largest was 2,048 × 1,556).

## Not done

Luminance/chroma (`Y RY BY`) conversion; DWAA, DWAB and HTJ2K decoding; deep data; reading *every*
part of a multi-part file (and writing one); writing layers; a half-precision control in the
Export dialog; keeping header attributes (comments, owner, exposure…); mip-mapped output;
cropping to the data window on export; a chosen display window.
