# Opening Photoshop documents (PSD and PSB)

PhotoForge opens `.psd` and `.psb` files as layered documents. It does not write them. The
file is never changed; the result is a new, unsaved document that you save as a PhotoForge
project. Every time a file is opened a **report** lists what was approximated and what was left
out, because nothing is dropped silently.

Open one with **Open** (it is listed in the file dialog) or by dropping it on the window;
there is no separate menu.

## What comes across exactly

| In the file | In PhotoForge |
| --- | --- |
| Pixel layers, their position (including off-canvas parts) and their transparency | Pixel layers with the same pixels, placed by translation |
| Opacity, and fill opacity (the two are multiplied) | Layer opacity |
| Visibility | Visibility |
| Normal, Darken, Multiply, Color Burn, Lighten, Screen, Color Dodge, Overlay, Soft Light, Hard Light, Difference, Exclusion, Hue, Saturation, Color, Luminosity | The same blend modes |
| Groups, open or closed, isolated or pass-through, nested to PhotoForge's depth limit | Groups, collapsed or expanded, isolated or pass-through |
| Layer masks and group masks, with their default value outside the mask's rectangle, disabled and inverted flags | Layer and group masks, cut to the layer's grid |
| Unicode layer names | Layer names (control characters removed, 120 characters) |
| 8-bit sRGB and 8-bit with an sRGB-equivalent profile | An 8-bit document |
| 16-bit and 32-bit, and 8-bit with another RGB profile | A high-precision (linear float) document, converted through the embedded profile; a 32-bit file with no profile is read as linear light |
| A file with no layers (or no layer with pixels) | One layer holding the flattened image |

## What is imported, but not as Photoshop shows it

Each of these appears in the report as **approximated**.

* **Text, smart objects, shape layers that store pixels, fill layers, linked files** are imported
  as the pixels Photoshop stored for them. They are no longer editable text, no longer a smart
  object with a source.
* **Vector masks** come across as the pixels they produced.
* **Clipping.** A layer clipped to the one beneath it is cut by that layer's coverage when it is
  imported ("baked"). The picture matches for a Normal-blended base; it is no longer a live
  relationship, so moving the base does not move the clipped layer.
* **A blend mode PhotoForge does not have** (Dissolve, Linear Burn, Darker Color, Linear Dodge,
  Lighter Color, Vivid Light, Linear Light, Pin Light, Hard Mix, Subtract, Divide) is imported
  as Normal and named.

## What is left out

Each of these appears in the report as **left out**.

* **Layer styles** (drop shadow, stroke, glow, bevel, overlays): the layer is shown without them.
* **Adjustment layers** (Curves, Levels, Hue/Saturation, and the rest): they hold no pixels, so
  the layers beneath are shown without them. This changes the picture; the report names each.
* **Shape and fill layers that Photoshop stored as a vector path and a fill, with no pixels.**
  PhotoForge does not read the path yet.
* **Artboards**: their contents are placed on the one canvas.

## What is refused, and what to do

The file is not opened, and the message says how to make one that will be.

* **CMYK, Lab, Indexed, Bitmap, Duotone, Multichannel**: choose *Image > Mode > RGB Color* in
  Photoshop and save a copy.
* **1-bit** documents: the same.
* A document wider or taller than PhotoForge's canvas limit (131,072 pixels), or needing more
  pixel memory than the resource manager allows (the message gives both numbers).

## How it is kept safe

A Photoshop file is a large, irregular format, and PhotoForge treats every length it declares
as a claim to be checked, not a size to allocate.

* Each length is compared with the bytes that remain *in its own section* before any buffer is
  made for it. A header that declares four gigabytes costs a comparison and an error.
* The number of layer records, resource blocks and additional-information blocks is capped;
  nesting is capped at PhotoForge's group depth; layers at 512.
* A channel's decoded size is computed from its declared dimensions with checked arithmetic and
  compared with the memory budget before its data is read; the data must fill exactly that size.
  A zlib stream that inflates past it is stopped at it.
* Nothing in a Photoshop file is executed, fetched or followed: linked files are not opened.

## How it was verified

This reader and importer are new code written against Adobe's published specification, and
agreement with a writer built from the same reading proves little. So they were also run
against **real files**:

* 315 PSD and PSB files from the test suite of the open-source *psd-tools* project (MIT
  licence; downloaded for this check and **not** committed): 8-, 16- and 32-bit; RGB, greyscale,
  CMYK, Lab, indexed, bitmap, duotone and multichannel; 28 with their layers in the 16- or
  32-bit document block; PSB; groups, masks, clipping, blend modes, effects, adjustment layers,
  text, smart objects, shape layers.
* **The reader** parsed all 315 and decoded 4,445 channels exactly to their declared sizes (32-bit
  samples checked to be finite and plausible).
* **The importer** opened 289 and refused 26 (the unsupported modes and depths), with 0 failures.
  Photoshop stores its own flattened image in each file, which is an independent picture of what
  the layers should look like, so each import was rendered by PhotoForge's compositor and
  compared with it. The 35 imports whose report said nothing was approximated or left out all
  match except one, whose stored flattened image is entirely white although its layer holds a
  clean triangle; the layer data was decoded independently to check that. Of the 220 imports
  that did report approximations, 115 still match closely, which is as expected: those are
  the files where the report names the reason.

Writing the real-file check found three defects the specification-based tests had not: the
order of the fields in a layer-mask block, the row table of a four-channel flattened image, and
that 32-bit files use zlib with prediction throughout (so refusing it would have refused every
32-bit file). They are fixed and covered.

To repeat the real-file checks on your own files, point `PHOTOFORGE_PSD_SAMPLES` at a file or
folder and run `cargo test --test psd_real_files --test psd_import_real -- --nocapture` from
`src-tauri`. Without the variable they do nothing.

## What is not done

PSD **export**; clipping as a live relationship; shape layers as editable shapes; text as
editable text; adjustment layers as PhotoForge adjustment layers; layer styles; CMYK and Lab;
the blend modes listed above; PSB over 131,072 pixels. Each is a recorded gap, not a claim.
