# Phase 13 text layers

A text layer stores the characters the user typed, the font they asked for, and
how it should be set. It stores no pixels. Reopening a project re-shapes the
text from those characters, so editing it after a save is the same operation as
editing it before one.

It also stores no glyph indices. A glyph index names a slot in one particular
font file on one particular machine; a project that stored them would reopen as
nonsense anywhere else. The integration gate asserts this against the serialised
JSON, not only against the restored value, because "it round trips" would be
equally true of a project that saved a bitmap and handed the same bitmap back.

## What a text layer stores

| Field | Meaning |
| --- | --- |
| `text` | The characters, UTF-8, up to 16 KiB |
| `fontFamily` | The family requested, or empty for the system default |
| `fontSize` | Document pixels, 0.5–2000 |
| `fontWeight` | CSS number, 100–900 |
| `italic` | Whether an italic face is requested |
| `align` | `start`, `center`, `end` or `justified` |
| `lineHeight` | Line advance as a multiple of the font size, 0.1–10 |
| `letterSpacing` | Document pixels, ±1000 |
| `originX` / `originY` | Where the block begins, in document coordinates |
| `wrapWidth` | Present for area text; absent for point text on one line |
| `fill`, `stroke`, `strokeStyle` | Paint, shared with shape layers |

`align` is `start`/`end` rather than `left`/`right` because the layout is
direction-aware: `start` is the left edge of a left-to-right paragraph and the
right edge of a right-to-left one.

## Shaping

Layout is cosmic-text's. It applies the Unicode bidirectional algorithm, selects
contextual forms, groups clusters, and falls back across faces for characters
the requested family cannot draw. Implementing that here would be a downgrade
dressed up as independence.

Two claims about international text are worth stating carefully, because both
are easy to assert falsely:

* **Right-to-left text is shaped, not merely rendered.** Persian "سلام" produces
  four glyphs laid out right to left with genuine contextual joining — the
  connected forms differ from the isolated ones, which is checked by comparing
  the outlines of the joined string against the same letters separated by
  spaces.
* **Mixed-direction text is reordered.** In "Hi سلام" the Latin sits left, the
  Persian sits right, and inside the Persian run the source byte offsets run
  *backwards* as x increases.

Both are tested against **source byte offsets**, not against the order glyphs
happen to be reported in. That order is a layout detail — cosmic-text reports a
pure right-to-left line in logical order and a mixed line in visual order — and
an assertion built on it would be testing the library's reporting convention
rather than the claim. Each glyph therefore carries the byte range it came from,
which caret placement, hit testing and selection need anyway.

What is **not** claimed: PhotoForge has no vertical writing modes, no ruby
annotation, no OpenType feature controls, and no justification beyond what
cosmic-text does. Complex scripts render through the shaper; they have not been
reviewed by native readers of each script.

## Outlines, not bitmaps

swash will rasterise a glyph, and hands back an 8-bit alpha mask. PhotoForge
takes the outline instead and draws it with the same float coverage rasteriser
vector shapes use. That keeps coverage in `f32` rather than quantised to 1/255,
and it means one rasteriser in the application with one region-independence
guarantee covering both text and shapes.

Quadratic segments in a glyph outline are raised to cubics exactly — the control
points sit two thirds of the way from each endpoint toward the quadratic's
control — so the path model has a single curve type.

Scaling is unhinted. Hinting snaps outlines to a pixel grid that exists at one
scale, and a text layer here can be transformed to any scale after the fact.

## Flattening happens in device space

Shapes flatten in document space at a fixed tolerance. Text does not: a glyph's
control points are mapped into device space **first**, and flattened there.

This is exact rather than an approximation, because PhotoForge's layer
transforms are affine — translate, scale, rotate, flip — and an affine map sends
a cubic's control polygon to the transformed cubic's control polygon. Mapping
four points is therefore the whole of transforming the curve, which is asserted
by a test comparing the enclosed area of both orders.

The benefit is that the 0.05-pixel tolerance is measured in the pixels actually
being drawn. Text is read at every zoom level, and flattening in document space
would coarsen the curves exactly when the user has zoomed in to look closely.

Region independence survives this, because the mapping and the tolerance both
depend only on the text and the transform, never on which rectangle asked. The
test asserts a region equals the same window of the whole frame **exactly**.

## Fill rule

Glyph fills always use the non-zero rule. Font outlines are drawn with counters
wound against their contours, which is what non-zero reads; even-odd would fill
the inside of an "o" and hollow out the overlap where a script face joins two
strokes.

## Missing fonts

A requested family that this machine does not have is **reported, never
repaired**. The text draws in the system's default sans-serif, and the project
goes on asking for the original face, so opening it somewhere that has the font
restores the intended setting rather than a record of one machine's gap.

The substitute is deliberately the generic sans-serif rather than a named face:
naming one would make the substitution look like a choice the author made.

`inspect_document_fonts` reports every family a document asks for, whether it is
installed, and which layers ask for it — including layers nested inside groups.
The panel says plainly that line breaks and spacing under a substitute are not
the ones the text was set with.

## Performance

Measured on the development machine, release build:

| Operation | Cost |
| --- | --- |
| Font discovery (226 families, warm OS cache) | 15 ms release, 43 ms debug |
| Font discovery, first run of a session | several hundred ms |
| `family_is_available` | below measurement resolution |
| Shaping a 50-glyph wrapped paragraph, cold | 4.74 ms |
| The same, served from the cache | 0.0001 ms |
| Rasterising into one 256-pixel tile | 0.54 ms |
| Rasterising a full 1920×1080 frame | 14.15 ms |

Both caches exist because of those numbers. Discovery is behind a `OnceLock`
because `family_is_available` runs on every render. Shaped results are behind a
64-entry cache keyed by every input the layout depends on, because a 1920×1080
frame in 256-pixel tiles is thirty-five rectangles: without it, one text layer
would re-shape thirty-five times per frame — about 165 ms of duplicated work —
and those thirty-five renders happen on several threads that would queue behind
one lock to compute identical answers.

A cache hit is the same answer shaping would have produced, which is asserted by
comparing a hit against a fresh shape glyph by glyph, and the key is asserted to
change for every field independently.

## How text reaches the canvas

`render_text` rasterises into the rectangle being rendered and returns colour
plus coverage. That goes through `composite_coverage`, shared with shapes, which
applies the layer's mask, opacity and blend mode like any other layer. A text
layer has no buffer of its own, so its mask lives in canvas space, as a group's
or an adjustment's does.

The legacy 8-bit renderer refuses text layers for the same reason it refuses
shapes: antialiased glyph coverage is float, and pushing it through an 8-bit
backdrop would discard that silently.

## Caching and invalidation

A text layer's cache digest is its serialised content — the characters, the font
requested and the setting. The glyphs it resolves to are derived from those, so
digesting the request covers the result.

Its influence region comes from the **ink**, measured from glyph outlines rather
than from line boxes. Ink routinely leaves the line box: descenders, accents,
italic overhang and swashes all do, and a bound taken from the line box would
clip them. When bounding fails — shaping can fail — the answer is the whole
canvas, never a smaller region that would let a stale tile survive.

## Rasterizing

`rasterize_semantic_layer` is the only path from characters to pixels, and it
runs because the user asked for it. Nothing in the render path rasterises a text
layer to make some other operation convenient. A gate asserts that rendering a
document repeatedly, tiled and full-frame, leaves the document byte-identical
and gives the text layer no pixel buffer.

Rasterizing bakes placement, because the result is a canvas-sized buffer with an
identity transform. It does not bake opacity, blend mode or visibility: those
stay editable on the replacement layer rather than being applied twice.

## Fonts are not bundled

PhotoForge reads the fonts already installed on the machine and ships none of
its own. Copying a face out of Windows into an installer would be redistributing
someone else's licensed work. Nothing here downloads a font or touches the
network.

## What is not implemented

* **No full caret, hit-testing, or selection UI.** The canvas editor supports
  click-to-place and a bounded modal text edit; `caret_position` is available
  for future precise caret and selection interaction. Text can also be edited
  through the panel.
* **No text-on-a-path, no warped text.**
* **No OpenType feature or variable-axis controls.**
* **No vertical writing modes.**
* **No font embedding in the project.** A project names the font it wants; it
  does not carry it.
* **No paragraph styles or character styles.** Every setting is per layer.
