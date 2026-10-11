# SVG

PhotoForge writes the **shape layers** of a document as SVG. (Reading SVG is the next increment;
this page will grow a section for it.)

Choose **Export**, then *SVG (shape layers)* in the file dialog, or give the file a `.svg` name.
The picture-format choices (profile, colour space, dither) and the metadata policy do not apply:
an SVG holds shapes, not a picture. The document on screen is not changed.

## What is written

| In PhotoForge | In the SVG |
| --- | --- |
| Rectangle (with corner radius), ellipse, line | `<rect rx ry>`, `<ellipse>`, `<line>` |
| Polygon, star, path | `<path d>` from the same commands the renderer flattens |
| Fill colour and alpha | `fill="#rrggbb"` and `fill-opacity`; fills only closed geometry, as the renderer does |
| Fill rule | `fill-rule="evenodd"` when it is even-odd |
| Stroke colour, alpha, width, cap, join, miter limit | `stroke`, `stroke-opacity`, `stroke-width`, `stroke-linecap`, `stroke-linejoin` (always said, because SVG's default is a mitre and PhotoForge's is round), `stroke-miterlimit` |
| Layer opacity, visibility | `opacity`, `display="none"` (hidden layers are kept, hidden) |
| Layer transform (move, scale, rotate, flip) | one `transform="matrix(…)"`, about the centre of the **canvas**, which is how the compositor maps shapes |
| Blend modes the W3C set has (16) | `style="mix-blend-mode:…"` |
| Groups | `<g>` with the same attributes, nested |
| Layer names | `<title>`, escaped |

Colours are the document's linear values encoded to sRGB; a colour outside sRGB is clipped and
the report says so.

## What is left out, and named

Each is in the export report; nothing is rasterised into the file behind the person's back.

* **Pixel layers, text layers, smart objects**: an SVG here holds shapes. (Text as outlines is
  possible and is not done.)
* **Adjustment layers**: they have no SVG form; the shapes beneath are drawn without them.
* **Layer masks**: the layer is drawn unmasked.
* **Edits over the whole document** (crop, rotation, resize, global adjustments).

## What is written approximately, and named

* **A blend mode the W3C set lacks** (Linear Burn, Vivid Light, Hard Mix and the other
  Photoshop-only ones) is written as Normal.
* **A layer clipped to the one beneath it** is written unclipped.
* **A pass-through group with an opacity or blend mode** is written as an isolated group.
* **A stroke on a layer scaled differently across and down**: PhotoForge draws the stroke at the
  average of the two scales, an SVG scales it with each.
* **Translucent shapes that overlap, in a high-precision document**: PhotoForge blends in linear
  light and an SVG viewer blends in sRGB, so the overlap looks different (darker, usually). A
  document of opaque shapes, or an 8-bit document, has no such difference and is not told so.

## How it was checked

* **Unit tests** of each rule above, including that the matrix reproduces the compositor's own
  `forward` map for a point and the corners of the canvas, with and without flips.
* **Through the real command**: the name, the arguments, the report, the refusals (not `.svg`, not
  absolute, no such folder, an invalid document leaves an existing file), and that nothing else is
  left beside the file.
* **Against an independent renderer.** Five shape documents (plain shapes, strokes of every cap
  and join, transforms and flips, opacity and groups, even-odd and multiply) were exported and
  rendered by `resvg`, and compared with PhotoForge's own render of the same documents. Pixels
  agree to a mean of 0.07 to 0.4 levels (premultiplied, 0 to 255), with the differences on
  anti-aliased edges, and one understood exception: the overlap of two translucent shapes
  (mean 5.4), which is the sRGB-against-linear difference above.
* **It found a defect in PhotoForge's own stroker**: a `bevel` join was built on one side of the
  path only, so every corner that turned the other way was notched (a bevelled rectangle lost
  its corners; 32 square pixels of 1,248). Fixed in `vector/stroke.rs`, with a test that fails
  without the fix.
* **Mutation checks**: eight breakages (the translation left out of the matrix, colours not
  encoded, open geometry filled, names not escaped, a bevel written as a mitre, hidden layers
  shown, the pass-through note never made, the stroker's bevel on one side) are each caught.

## Not checked

* The file in **Inkscape, Illustrator or a browser**. `resvg` is one renderer; a browser's differs
  in detail.
* The interface in a real window (component tests only).

## Not done

Reading SVG; text as outlines; gradients (PhotoForge has none); masks as SVG masks; embedding
pixel layers as `<image>`.
