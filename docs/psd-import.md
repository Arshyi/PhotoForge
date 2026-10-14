# Opening Photoshop documents (PSD and PSB)

PhotoForge opens `.psd` and `.psb` files as layered documents (writing them is described in
[psd-export.md](psd-export.md)). The file is never changed; the result is a new, unsaved document
that you save as a PhotoForge project. Every time a file is opened a **report** lists what was approximated and what was left
out, because nothing is dropped silently.

Open one with **Open** (it is listed in the file dialog) or by dropping it on the window;
there is no separate menu.

## What comes across exactly

| In the file | In PhotoForge |
| --- | --- |
| Pixel layers, their position (including off-canvas parts) and their transparency | Pixel layers with the same pixels, placed by translation |
| Opacity, and fill opacity (the two are multiplied) | Layer opacity |
| Visibility | Visibility |
| Normal, Darken, Multiply, Color Burn, Linear Burn, Darker Color, Lighten, Screen, Color Dodge, Linear Dodge (Add), Lighter Color, Overlay, Soft Light, Hard Light, Vivid Light, Linear Light, Pin Light, Hard Mix, Difference, Exclusion, Subtract, Divide, Hue, Saturation, Color, Luminosity | The same blend modes (26) |
| Groups, open or closed, isolated or pass-through, nested to PhotoForge's depth limit | Groups, collapsed or expanded, isolated or pass-through |
| Layer masks and group masks, with their default value outside the mask's rectangle, disabled and inverted flags | Layer and group masks, cut to the layer's grid |
| Clipping masks: a layer clipped to the one beneath it | The same relation: the layer stays clipped, drawn only where its base is, composited with the base's blend mode and opacity as one group |
| Invert adjustment layers | An adjustment layer (a curve that inverts), exactly |
| Levels adjustment layers with master settings only | An adjustment layer using PhotoForge's Levels, the same arithmetic |
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
* **Curves, Levels with settings for individual channels, and Exposure** are imported as adjustment
  layers holding a curve. Photoshop draws a smooth curve through a curve's points; PhotoForge joins
  its points with straight lines, so the smooth curve is sampled at up to 32 points. Against
  Photoshop's own flattened pictures the difference averages 0.1 to 0.5 of a level for Curves and
  Levels and about 1.5 for Exposure (see [phase-15-1d-results.md](phase-15-1d-results.md)). A channel's
  curve or levels is applied before the composite one, as Photoshop does.
* **Clipping, where it cannot be kept whole.** A clipped layer whose base could not be imported
  (a fill or adjustment layer with no pixels) has nothing to attach to and is imported unclipped;
  a pass-through group that is the base of a clipped layer is imported as an isolated group; and
  a base that has "Blend Clipped Layers as Group" switched off is composited as one group anyway.
  Each is named in the report. Ordinary clipping is not reported, because it is not approximated.
* **Dissolve**, the one blend mode PhotoForge does not have (a random dither whose pattern cannot
  be reproduced), is imported as Normal and named.
* **A group with a fill opacity below 100% whose layers blend with what is beneath** (or that is
  pass-through) is imported with the fill applied as opacity. Photoshop does something slightly
  different there; on the one file made for the case the difference averages 3 levels.

## What is left out

Each of these appears in the report as **left out**.

* **Layer styles** (drop shadow, stroke, glow, bevel, overlays): the layer is shown without them.
* **Adjustment layers other than those above** (Brightness/Contrast, Hue/Saturation, Color
  Balance, Posterize, Threshold, Gradient Map, Selective Color, Channel Mixer, Photo Filter, Black
  & White, Vibrance, Color Lookup; and Exposure in a greyscale document): they hold no pixels, so the
  layers beneath are shown without them. This changes the picture; the report names each.
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
* **Only the channels that are drawn are decoded**: the colour channels (three, or one in a
  grayscale file) and transparency. A spot or alpha channel beyond them is a plane as large as the
  layer and a file can carry dozens; it is neither read nor counted, so a layer's memory is what
  the estimate said it is (before Phase 15.11 a file with many extra channels could take up to about
  fourteen times the estimate).
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

Shape layers as editable shapes; text as editable text; the adjustment layers listed above as
left out; layer styles; CMYK and Lab; Dissolve; PSB over 131,072 pixels. Each is a recorded gap,
not a claim. (Export is done, and has its own list: [psd-export.md](psd-export.md).)
