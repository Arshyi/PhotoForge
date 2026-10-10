# Saving as a Photoshop document (PSD and PSB)

PhotoForge writes a layered document as a `.psd` (or `.psb`) file. The layers go into the file as
layers, not as a flattened picture, and a **report** says what was written as it is, what was
written approximately, and what was left out, because nothing is dropped silently.

Choose **Export**, then *Photoshop document (layers kept)* in the file dialog (or give the file a
`.psd` or `.psb` name). The picture-format choices of the export panel (profile, colour space,
dither) do not apply: a Photoshop file holds the layers. The document on screen is not changed.

The depth is chosen for you: 8-bit for an 8-bit document, 16-bit for a high-precision one, and
32-bit when a high-precision document holds colour brighter than white (the report says so). A
`.psb` name asks for the large format; a `.psd` that is too big for the format is written as
`.psb` instead and the report says so.

## What is written exactly

| In PhotoForge | In the file |
| --- | --- |
| Pixel layers placed by a whole-pixel move, including the parts off the canvas | Layers with the same pixels, transparency and rectangle |
| Opacity, visibility, locked | Opacity (in 255ths), the visible flag, the lock |
| The 26 blend modes | The same modes, by Photoshop's keys |
| Groups: isolated or pass-through, open or closed, nested | Folders with their dividers, in the order Photoshop writes them |
| Layer and group masks, enabled or not, inverted or not | Layer masks; an inversion is applied to the stored mask, so a reader that ignores the flag paints the same |
| Clipping (live) | The clipping flag on each clipped layer |
| Names, in Unicode | The 8-bit name and the Unicode name block |
| Curves, Levels (master settings), Invert | Photoshop's Curves, Levels and Invert adjustment layers, with opacity, visibility, blend mode, mask and clipping |
| The picture, composited by PhotoForge | The flattened image the file also holds (what other programs show) |

## What is written approximately

Each appears in the report as **approximated**.

* **A layer with a scale, rotation or flip, or placed between pixels** has its transform applied
  and is stored as the pixels that result. It can no longer be changed in the file. A mask that
  moves with the layer is resampled the way PhotoForge draws it.
* **Shape, text and smart-object layers** are stored as the pixels they draw; they are not
  editable shapes, editable text or smart objects in the file.
* **A group with a transform** is flattened into one layer with the transform applied.
* **Curves** are stored as Photoshop's Curves with at most 16 points per channel, which
  Photoshop joins with a smooth curve (PhotoForge joins its points with straight lines). A curve
  that has more points is reduced by a fit against Photoshop's own spline, and the report says so.
* **Per-channel tone adjustments** (Brightness, Contrast, Gamma, Black/White point and the like,
  where each colour channel is changed on its own) are sampled into Photoshop Curves in the same
  way.
* **Highlights brighter than white** are clipped when the file is made 8- or 16-bit on request
  (the command takes a depth; the Export dialog does not ask for one). Left to itself the exporter
  writes such a document as 32-bit and nothing is lost.

## What is left out

Each appears in the report as **not exported**.

* **Adjustment layers that mix the colour channels or look at neighbouring pixels**: blur,
  sharpen, hue/saturation, white balance, plug-in filters. They are not in the file, and the
  layers beneath them are shown without them there. The flattened image the file holds still
  includes them, so other programs that show it show them.
* **Edits over the whole document** (those not in a layer). When they keep the picture's size
  they are in the flattened image the file holds, not in the layers; when they change its size or
  shape (a crop, a rotation, a resize) they are in neither. The report names them.

## Colour

8- and 16-bit files carry an sRGB profile and hold encoded values. A 32-bit file holds linear
light and carries no profile, which is what Photoshop's 32-bit mode is. Colour is RGB;
PhotoForge does not write CMYK, Lab, indexed or greyscale files.

## Compression and size

8- and 16-bit channels are compressed with PackBits (RLE), which every Photoshop reader accepts;
32-bit channels are stored raw. A file is written as PSB when a side is over 30,000 pixels or the
data would pass 2 GiB. The pixel memory needed is checked against the same budget as opening a
file, before anything is written, and a file that would not fit is refused with the numbers.

The file is written beside its destination and then renamed, so a failed export leaves what was
there. PhotoForge will not write over a file that a layer in the document is linked to.

## How it was checked

* **Reading it back.** Every layer comes back with the same name, order, blend mode, visibility,
  opacity (to 1/255), clipping, mask and position, and the picture comes back the same, in unit
  tests of each rule, through the command boundary, and through the importer.
* **Real files.** The 290 psd-tools files the importer opens (MIT, not committed) were each
  imported, exported and imported again: all 290 came back with the same layers; **274 as the
  identical picture** and 16 within a level on average, none further; 55 MiB written, the slowest
  file 1.6 s. The 16 are files with adjustment layers or opacities that are sampled or rounded
  a second time; the largest single difference is 26 levels, in `curves_rgb.psd` at a sharp
  feature of one curve (mean 0.30).
* **Independently of this code.** A parser written outside PhotoForge reads the same file for
  the layer records, the tagged blocks, their padding and the channel data; it found that block
  lengths must include their padding (the first version did not, and PhotoForge's own reader did
  not mind), which is now fixed and has a test that walks the bytes without the reader's
  leniency.

**Not checked: opening the file in Photoshop** (or any program that is not this one's own reader
or the independent parser). The layout follows the specification and the files Photoshop wrote
in the same suite, and a program with a stricter reader could still disagree. Until someone opens
a file in Photoshop, Affinity or GIMP, treat compatibility as likely and unconfirmed.

To repeat the real-file check on your own files, point `PHOTOFORGE_PSD_SAMPLES` at a file or folder
and run `cargo test --test psd_export_real -- --nocapture` from `src-tauri`. Without the variable
it does nothing.

## What is not done

Vector shape and fill layers as editable shapes; editable text; smart objects as smart objects;
layer styles (PhotoForge has none to write); a PhotoForge Hue/Saturation, blur or sharpen as the
Photoshop layer of that name (they are left out and named); a Brightness or Contrast layer as
Photoshop's Brightness/Contrast (it is written as the Curves that do the same); CMYK and Lab; a
depth chosen in the Export dialog; and, above all, **opening the result in Photoshop**.
