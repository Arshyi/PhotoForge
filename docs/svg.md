# SVG

PhotoForge writes the **shape layers** of a document as SVG, and reads a bounded subset of SVG as
shape layers. Writing comes first below; reading is [further down](#reading-svg).

## Writing SVG

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

Text as outlines; gradients (PhotoForge has none); masks as SVG masks; embedding pixel layers as
`<image>`.

## Reading SVG

**Open** an `.svg` (it is offered in the Open dialog, and a dropped `.svg` goes the same way) and
it becomes a new, unsaved document of **shape layers**, with a report of what the subset did not
cover. The file is not changed; **Save** asks where to put the PhotoForge project.

An SVG is text from a stranger. The reader is built so that opening one cannot reach the network,
run anything, or exhaust memory:

* **Nothing is fetched.** An `<image>` or any reference to an address or file is not followed, and
  is reported. There is no network code in this path at all.
* **Nothing is run.** `<script>`, event attributes and `<foreignObject>` are named and ignored.
* **No DTD, so no entity expansion.** The parser refuses a document type declaration outright (the
  "billion laughs" file is an error, not a slowdown), and stops at 200,000 nodes.
* **Bounded.** A file over 8 MiB, a canvas edge over 16,384 pixels, more than 512 shapes, a path
  over 20,000 segments, or groups nested past PhotoForge's limit are refused or flattened, and say
  so.
* **Bad numbers cannot break it.** A coordinate no float can hold, or a scale that compounds past
  anything finite, leaves that one shape out and says so; the rest imports.

### What is read

| In the SVG | In PhotoForge |
| --- | --- |
| `<svg>` width, height (px, pt, pc, mm, cm, in), `viewBox`, `preserveAspectRatio` | The canvas, with the view box fitted and centred (`none` stretches) |
| `<rect>`, `<circle>`, `<ellipse>`, `<line>` | A rectangle (corner radius kept), ellipse and line, kept parametric when the transform allows |
| `<polyline>`, `<polygon>`, `<path>` (every command: M L H V C S Q T A Z, relative and absolute) | A path; quadratics become cubics, arcs become cubics in quarter turns |
| `fill`, `stroke`, `fill-opacity`, `stroke-opacity`, `opacity`, `stroke-width`, `stroke-linecap`, `stroke-linejoin`, `stroke-miterlimit`, `fill-rule`, `color`/`currentColor`, `display`, `visibility`, as attributes or in `style="…"` | The shape's fill and stroke, in linear light; inherited from groups as CSS says; opacity on the layer |
| Colours: `#rgb`, `#rgba`, `#rrggbb`, `#rrggbbaa`, `rgb()`, `rgba()`, 40 common names, `transparent` | The same colour, decoded from sRGB |
| `transform`: `matrix`, `translate`, `scale`, `rotate` (about a point), `skewX`, `skewY` | A move or an axis-aligned scale is kept on the shape; a rotation or skew is **baked into the geometry** (the shape becomes its outline) |
| `<g>`, `<a>` | A group, with its opacity; its transform and style applied to the children |
| `<title>` and `id` | The layer name |

### What is left out, and named

`<text>` (an SVG's text needs fonts it does not carry), `<image>`, `<use>` (and so the
`<symbol>`s it would draw), nested `<svg>`, `<script>`, `<foreignObject>`, and elements the subset
does not read. Each says so in the report, with the element's `id` when it has one.

### What is read approximately, and named

* **A gradient or pattern fill** is replaced by the first colour of the gradient.
* **A dashed stroke** is a solid one (PhotoForge has no dashes).
* **`clip-path`, `mask`, `filter`**: the shape is imported without them.
* **A style sheet** (`<style>`): only styles written on the shapes themselves are applied.
* **Elliptical rounded corners** become round ones of the smaller radius.
* **A stroke under an uneven scale** is imported at the average of the two scales.
* **`preserveAspectRatio` with `slice`** is imported as `meet`.
* **An error in path data**: the path is drawn up to it, as SVG says to.
* **A colour name outside the 40** reads as no colour and the shape keeps its inherited paint.

### How reading was checked

* **Unit tests**: each of the rows above; the path grammar (every command, shorthands, exponents,
  flags written without separators, an error mid-path); arcs against the circle they describe;
  transform composition; colours; nesting and shape limits; a DTD, an entity bomb, and a file of
  bytes; every truncation and a changed byte of a sample, none of which panics.
* **A round trip**: what the exporter wrote, read back and rendered, against the original render.
* **Through the real command**: the name, the arguments, the document and report that come back,
  and the refusals.
* **230 real icons** (Feather, MIT; Bootstrap Icons, MIT; Simple Icons, CC0), each imported,
  rendered by PhotoForge and by `resvg`: **230 of 230 imported, none refused**, none reported as
  approximated or left out, a median mean difference of 0.2 of a level (0–255), 90th percentile
  0.9, worst 2.2.
* **Mutation checks**: ten breakages (a child transform applied after its group's, colours not
  decoded, a DTD allowed, `none` read as black, group opacity dropped, an arc's sweep inverted, the
  view box not centred, stroke widths not scaled, the shape limit off, a path error not reported)
  are each caught.

### What that comparison showed, which is not the importer's

The worst of the 230 (Simple Icons' EDEKA) has a path that stacks two nearly identical shapes.
PhotoForge's rasteriser sums partial coverage along edges and clamps (signed-area accumulation,
`docs/vector-layers.md`), so where two same-direction contours cover the *same* edge pixel
partially, the pixel comes out fuller than the union (0.73 + 0.72 reads as 1.0 where the true
union is 0.73). It is a property of how PhotoForge draws any path, not of reading SVG, it is a
fraction of a pixel on such edges only, and it is recorded as a known limitation, not fixed here.

### Not checked

The importer against SVGs that are not icons (maps, charts, drawings with filters and
gradients): they will mostly report. The interface in a real window.

### Not done

Text; `<use>`/`<symbol>`; gradients and patterns as themselves; CSS style sheets; dashes; clip
paths and masks; `.svgz`; an SVG's own colour profiles.
