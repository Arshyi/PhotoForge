# Phase 13 vector shape layers

A shape layer stores geometry. It does not store pixels, and it is not a pixel
layer that remembers what it used to be. Scaling one to 800% draws the shape
again at 800%; there is no source raster, so there is nothing to resample and
nothing to accumulate damage across repeated transforms.

## What a shape stores

`ShapeGeometry` keeps a shape's meaning rather than its coordinates. A rectangle
is a rectangle with a width, a height and a corner radius, not four points; a
star keeps its point count and its two radii. Changing a corner radius later is
therefore editing a parameter, not reconstructing geometry from its remains.

| Geometry | Parameters |
| --- | --- |
| `Rectangle` | `x`, `y`, `width`, `height`, `cornerRadius` |
| `Ellipse` | `cx`, `cy`, `rx`, `ry` |
| `Line` | `x1`, `y1`, `x2`, `y2` |
| `Polygon` | `cx`, `cy`, `radius`, `sides`, `rotationDegrees` |
| `Star` | `cx`, `cy`, `outerRadius`, `innerRadius`, `points`, `rotationDegrees` |
| `Path` | an explicit command list, for the pen tool and for converted shapes |

Paths hold `MoveTo`, `LineTo`, `CubicTo` and `Close`. Quadratics are converted
to cubics as a path is built, so the renderer has one curve type rather than
two. A path is capped at 20,000 commands and every coordinate at ±1,000,000:
project files are untrusted, and both flattening cost and accumulator width grow
with those numbers.

Fill and stroke are independent and both optional. A shape with neither draws
nothing, which is a legitimate state while the user is deciding.

## Why the rasteriser is PhotoForge's own

Coverage is `f32` throughout, computed by signed-area accumulation. The obvious
alternative was tiny-skia, and it was rejected on a specific ground rather than
on preference: its mask type is 8-bit — its own documentation calls it "just a
simple 8bit alpha mask" — so every antialiased edge would have been quantised to
1/255 before reaching a pipeline whose entire identity is high-precision linear
float. Text uses the same rasteriser, so there is one implementation of coverage
in the application and one place where its guarantees hold.

## Region independence

The renderer is tiled, so a shape is drawn many times into different rectangles,
and every one of them must agree with the whole frame exactly. Two decisions
make that true:

* **The accumulator spans the path, never the region.** Sizing it to the region
  would mean clamping edges that lie outside — and clamping an edge *moves* it.
  An earlier version did exactly that, and a region boundary that happened to
  fall on an ellipse's edge disagreed with the full-frame render by 0.003.
* **Each row's `x` is evaluated from the line equation, not advanced
  incrementally.** Advancing accumulates a different rounding history depending
  on where iteration started. That produced a 6e-6 disagreement, which is
  invisible and still a seam.

Both are covered by tests asserting *exact* equality between a region and the
same window of the whole frame, not equality within a tolerance.

Curve flattening for shapes uses a fixed 0.05-pixel tolerance in document space.
Fixed rather than derived from the transform, because a tile that flattened more
coarsely than its neighbour would seam down the middle of a curve.

## Strokes

A stroke is converted to a fill: each segment becomes a quad, joins and caps
become discs or triangles, and the union is filled under the non-zero rule.
Everything is wound consistently before that union. It has to be — an earlier
version wound the discs opposite to the quads, so non-zero winding cancelled
them and round caps contributed exactly nothing.

Stroke width is measured in device pixels and derived from the mapping into
device space, so a shape scaled by its layer transform gets a stroke scaled with
it. **A single width cannot express an anisotropic scale**, so a shape scaled
1.0 × 3.0 gets the average of the two axes rather than an elliptical pen. This
is a real limitation, not an approximation that converges: a stroked circle
scaled anisotropically has a stroke of uniform width where a true anisotropic
pen would vary it.

Miter joins fall back to bevel past the miter limit, which is what stops a
nearly-doubled-back joint growing a spike the length of the document. The
layer's influence bounds include the miter margin, so a spike that does occur is
inside the region the cache invalidates.

## How a shape reaches the canvas

`render_shape` rasterises directly into the rectangle being rendered and returns
colour plus coverage. That goes through `composite_coverage`, shared with text,
which applies the layer's mask, opacity and blend mode exactly as for any other
layer. A shape has no buffer of its own, so its mask lives in canvas space, like
a group's or an adjustment's.

The legacy 8-bit renderer refuses shape layers rather than compositing them.
Antialiased coverage is float, and pushing it through an 8-bit backdrop would
discard that silently; a document containing a shape is a 0.13.0 document and
uses the high-precision renderer.

## Caching

A shape's cache digest is its serialised content — there is no external buffer
to fingerprint, so the description is both necessary and sufficient. Its
influence region is its transformed bounds including the stroke and miter
margin, so moving a shape invalidates the tiles it left and the tiles it arrived
at, and nothing else.

Extending the random layer-tree property tests to generate shapes exposed a
cache bug that predated them. A **pass-through group** has no buffer of its own:
its children composite straight onto the canvas, and `cross_fade` uses the
group's transform only to sample the group's mask. The cache mapped the
children's region through that transform anyway, so an edit invalidated tiles
the group does not paint and left the ones it does paint stale. It had been
reachable for as long as groups have had transforms, and needed a child small
enough to leave the mapped region to show it — until vector layers, every
generated child covered the whole canvas.

## Limits

| Constant | Value | Why |
| --- | --- | --- |
| `MAX_PATH_COMMANDS` | 20,000 | Flattening cost grows with the command count |
| `MAX_COORDINATE` | 1,000,000 | Bounds the accumulator width |
| `MAX_POLYGON_SIDES` | 512 | Beyond this a polygon is a circle with more work |
| `MAX_STROKE_WIDTH` | 4,096 | Generated geometry is proportional to the width |
| `FLATTEN_TOLERANCE` | 0.05 px | Fixed, so tiles agree |

## What is not implemented

* **No SVG or PDF import or export.** Nothing in PhotoForge reads or writes
  either format, and nothing claims vector-preserving export.
* **No boolean operations** between shapes.
* **No gradient or pattern fills.** Fill and stroke are solid colours.
* **No per-shape dash patterns.**
* **No pen tool or on-canvas handles yet.** Geometry is authored through the
  document model; direct manipulation is not built.
