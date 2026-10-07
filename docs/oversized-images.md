# Oversized images

A file can be perfectly readable and still too large to open as a conventional
PhotoForge document, which holds its image as 16 bytes per pixel of linear float. A
100-megapixel JPEG is 400 MB of ordinary bytes and 1.6 GB of working float, before the
canvas frame, the retained original and the output. This page says what PhotoForge does
about that, format by format, and — as plainly — what it does **not** do.

How much is "too large" is decided by the memory budget
([resource-management.md](resource-management.md)), so the answer depends on the
machine: the same 100 MP file opens whole on a workstation and is offered a region on a
laptop. **Nothing is refused merely for being large**; a file is refused only if it
cannot be read honestly (a dimension no coordinate can hold, or a header claiming more
raster than the file's compression could possibly hold).

## What the person sees

Opening any file first **probes** it — reads the header, nothing else — *before the open
document is touched*, so cancelling leaves it exactly as it was. If the whole file
fits with room to spare it simply opens. If it fits but would use more than half the
budget it opens, and says so once afterwards ("High-memory document: about 3.2 GB while
editing, 54% of the memory budget"). Otherwise a dialog explains in numbers, for this
machine, why, and offers what genuinely exists:

* **Open a region at full resolution** — choose a rectangle on a bounded preview of the
  whole picture. The selector shows the rectangle's pixel size and megapixels and **what
  opening it would cost** against what the budget allows, live while the rectangle is
  dragged, using the planner's own formula (no second copy of the model). A rectangle
  over the limit is shown as such and cannot be confirmed. It can be moved and resized
  with the keyboard.
* **Open a reduced copy** — a smaller copy of *all* of the picture, at a size chosen from
  a short list, with the result's dimensions stated. It is labelled as having **less
  resolution than the file**, and the document records that it is a reduced copy.
* **Cancel.**

A dialog that has no option says so ("No way of opening this file is available"), and
says which of three things it is: a momentary shortage of free memory (closing other
programs would change the answer), a budget too small (raising it in Settings → Memory
may help), or an unsafe file. Every report also says why **out-of-core** is not offered,
below.

### Change Source Region…

A document that is a region of a larger file has **Change Source Region…** in the header
and the command palette. It first checks that the file is **still the one the region was
taken from** — by its contents, not its size or timestamp (below). If the file is gone,
or is a different picture, nothing is offered, and the message says the pixels in the
document are untouched. Otherwise it reopens the selector on the same file with the
current rectangle as the starting point.

Choosing a region **opens a new document** of it. Edits made to the current document are
**not carried across**, and you are asked before it is replaced. It is not a crop of the
open document and it does not pretend to be.

## What is decoded, per format

A decoder that allocates the whole frame and crops is never reported as having region
support. Each decoder states in its own header what it allocates, and the planner prices
it as exactly that:

| Format | Region | Reduced copy | Memory follows | Time follows |
| --- | --- | --- | --- | --- |
| **PNG** (non-interlaced) | **True row streaming.** Rows are inflated one at a time and only the window is kept. | Streamed row by row through an exact area-mean resampler. | the **region** (plus one row) | the region's **last row** |
| **PNG** (interlaced) | The frame is assembled first (Adam7 rows arrive in pass order), the window kept and the rest freed. | The same, then resampled. | the **source**, at its native depth | the whole file |
| **JPEG** | The **whole frame** is decoded, the window kept, the frame released before the float copy is built. **This is not region decoding** and the planner does not call it that. | **Decoded at a DCT scale of ½, ¼ or ⅛**, so the full frame is never produced, then area-reduced to the exact size. | region: the **source**, at 3 bytes a pixel instead of 16; reduced: the **scaled frame** | the whole file |
| **WebP** | The whole frame is decoded (the format's decoder offers nothing else), the window kept, the frame released. Not region decoding. | The whole frame, then resampled in bands. | the **source** | the whole file |
| **DNG** (camera RAW) | **Real region decoding by segment.** Only the strips or tiles that overlap the window are decoded, and the window is developed with the surrounding photosites the demosaic needs, so the result is **bit-identical to the same rectangle cut from a development of the whole sensor**. | **None is offered** for a RAW. A bounded *preview* is (needed to choose a region), built by decimating as segments are read. | the **region** plus one segment | the region's segments |

Three limits for DNG are stated rather than left to be discovered:

* The **compressed file is read whole.** A region bounds the decoded sensor and
  everything developed from it, not the read of the file, which is at most 750 MiB. The
  planner counts it as a fixed cost.
* A file stored as **one strip** has one segment, the whole sensor, so its window decode
  holds the whole decoded sensor (two or four bytes a photosite) while it works. The
  planner prices exactly that, from the file's layout.
* **Auto white balance is a statistic over the whole sensor.** A region asks for it by
  streaming the sensor one segment at a time: the *work* is whole-sensor even though the
  *memory* is not, and the region uses the gains the whole picture would, not its own.

### Measured

One scenario per process on the development machine (128 GB RAM, Windows 11, release
build), a 2000 × 2000 window of an 80 MP image (48 MP for WebP and DNG). "Peak" is the
process's peak working set, which includes the codec libraries' own buffers. Fixtures
are synthetic and are **not shipped**.

| Format (source) | Whole frame | Region (top / middle / bottom) | Reduced to ¼ per side |
| --- | --- | --- | --- |
| PNG 8-bit, 80 MP, 152 MiB file | 471 ms, **234 MiB** | 100 / 251 / 388 ms, **17 MiB** | 643 ms, 159 MiB |
| JPEG, 80 MP, 23 MiB file | 374 ms, 257 MiB | 378 / 393 / 384 ms, **257 MiB** | 559 ms, 173 MiB |
| WebP lossless, 48 MP, 42 MiB file | 538 ms, 325 MiB | 518 / 511 / 512 ms, **325 MiB** | 1,761 ms, 325 MiB |
| DNG, 48 MP, 96 MiB file | 1,212 ms, **1,657 MiB** | 163 / 168 / 171 ms, **231 MiB** | not offered |

What these show, and do not:

* PNG and DNG memory really follows the **region** (17 MiB and 231 MiB against 234 MiB and
  1,657 MiB). PNG's *time* still depends on how far down the window is, as documented —
  the top of the picture is four times cheaper than the bottom.
* JPEG and WebP **regions cost what the whole frame costs** (257 and 325 MiB). That is the
  honest figure, and the saving over opening whole is that the frame is held at 3 bytes a
  pixel (JPEG) or 4–7 (WebP) instead of 16, and is released before the float copy is
  built. The planner prices them as that.
* The 16-byte-per-pixel working copy of a 2000 × 2000 region is 61 MiB; the figures above
  are the *decode*, not the finished document.
* The DNG figure is for a tiled file. The fixture is synthetic; the numbers say how the
  decoder behaves, not how a particular camera's files compress.

## Are the prices right?

The planner's figures decide what a person is offered, so a price that is too low turns a
refusal that should have happened into an out-of-memory. `tests/estimate_accuracy.rs`
keeps them honest: with a counting allocator it measures the peak heap each decoder
allocates — PNG (8-bit, RGBA, 16-bit), JPEG, WebP with and without alpha — for a region and
for a reduced copy, both for the decoder alone and for the application's whole open
(decode, working copy, preview, the file's hash), and fails if any measurement is **more**
than the planner priced. On a 2400 × 1800 fixture the decoders land at 23–90% of their
price and the whole open at 8–48%; the margin is deliberate and the gate has no
lower bound beyond "not 40 times too high".

It earned its place by finding three things the first version of the planner got wrong,
all now fixed:

* **WebP without alpha was priced at 3 bytes a pixel, the size of the result. The decoder
  holds 7** — it decodes to RGBA and converts to RGB while both are alive (4 with alpha,
  which was right). Measured at 96 MP: 645 MiB against an old price of about 360.
* **A reduced copy held its result twice** while it finished (one premultiplied buffer,
  one final image), doubling the peak of exactly the operation that exists to bound it.
  It now builds the result in place.
* **A real open also builds a bounded preview** (a float and an 8-bit copy of up to
  1600 × 1600 pixels, about 51 MB) and every decoder has buffers of its own (the PNG
  reader's alone is 1 MiB), neither of which was priced. Both are now, as named
  constants, one of them tied by a test to the preview's real size bound.

What the gate does not see: it counts the Rust heap, where the decoders allocate. Memory
the operating system maps for the process by itself shows only in the working-set figures
above, which agree with it. It runs on small fixtures; the per-pixel coefficients are what
it checks, and the fixed parts are checked by the benchmarks at 48–96 MP.

## Is a region identical to the same part of the whole?

For every format the answer is a test, not an assertion. A region is compared with the
same rectangle cut from a whole decode **sample for sample**:

* PNG (every colour type and bit depth, and interlaced), JPEG and WebP:
  `src/source/{png,jpeg,webp}.rs`, `a_region_is_bit_identical_to_the_crop_of_a_full_decode`
  and its equivalents.
* DNG: `tests/raw_region.rs` compares 19 rectangles — the whole sensor, single
  photosites in every corner, odd and even origins, one-photosite-wide strips, windows
  against each edge and straddling tile boundaries — in strips, 8 × 8 tiles and 16 × 16
  tiles, under as-shot, custom and temperature-and-tint white balance, **bit for bit**, and
  checks that the gains applied equal the whole development's. It also proves the demosaic
  margin is *load-bearing* (removing it makes the result differ), that tiles outside the
  window are never read, and that hostile requests are refused cleanly.

## A document that remembers where it came from

A region or reduced copy is not a crop. Cropping throws the rest away and forgets it ever
existed. The layer records **which file, which part**:

* the file's identity is the **SHA-256 of its contents**, with its size as a quick filter
  and **no timestamps at all** — two files with the same size are never taken as the same,
  and a changed timestamp is not a changed file;
* the view is either `region` (a pixel-exact rectangle, unscaled) or `reduced` (the whole
  source, resampled, with the resulting size) — a reduced copy **says it has less
  resolution than the file**;
* a camera RAW region keeps its RAW source and view, so developing it again develops the
  same region.

The pixels themselves are saved in the project like any other layer's. The origin is
**provenance, not a dependency**: a project whose source is missing opens and edits exactly
as one without, and saves and reopens with its source view intact. Whether the source is
*available*, *missing* or *changed* is checked by content when it matters — before Change
Source Region, and when the project asks — and reported as such. Selections are remembered
per region: two regions of one file are different documents.

## Out-of-core editing is not offered

"Open Full Resolution (Out-of-Core)" does not exist in PhotoForge, and the report that
every dialog is built from says so in every case:

> Open Full Resolution (Out-of-Core) is not available: the pixel store, the layer
> compositor's source access and project persistence each hold complete buffers, so
> decoding in tiles would only move the allocation.

That is a statement of what the code does, not of what would be possible. A layer's pixels
are one buffer in the store; the compositor reads whole layers; saving a project writes
whole entries. Decoding in tiles into those would be a decoder that streams into a buffer
the size of the whole image, which is the same peak with more steps. Making it real means
changing all three, and nothing here claims otherwise. **The oversized-image story is
region and reduced copy, with a document that remembers its source.**

## What this does not do

* It does not **open a file larger than the budget whole**, however clever the decoder.
* It does not make JPEG or WebP **regions** cheap — it makes them honest.
* It does not offer a **reduced copy of a camera RAW**.
* It does not carry **edits from one region to another**.
* It does not **re-link** a document to a different file.
* The region for a source with a header that lies about its dimensions is refused before
  anything is decoded; a source that decodes to something other than its header said is
  an error, not a crop.
