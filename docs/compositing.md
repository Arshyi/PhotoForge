# Compositing

PhotoForge 0.8.0 renders a layered document with a deterministic compositor in
`src-tauri/src/layers`. This document states exactly what that compositor does,
including where it is deliberately simpler than a full colour-managed renderer.

## Pipeline

```text
Document
  ↓
Layer tree traversal (bottom to top, depth first)
  ↓
Per-layer transform          translate, scale, rotate, flip
  ↓
Layer effect / adjustment    adjustment layers re-evaluate their parameters
  ↓
Layer mask                   8-bit coverage, sampled in layer space
  ↓
Opacity                      multiplied into coverage
  ↓
Blend mode                   W3C blend function B(Cb, Cs)
  ↓
Composite                    source-over with straight alpha
  ↓
Clip to canvas → display or export
```

The document-level operation pipeline (global adjustments plus crop, straighten,
perspective, and lens correction) runs **after** compositing, on the finished
canvas. Per-layer transforms position content inside the canvas; document
geometry reshapes the canvas itself. The two never interact.

## State separation

| Concern | Owner | Notes |
| --- | --- | --- |
| Document state | Frontend `LayerDocument` | Pure data: identifiers, geometry, parameters, mask coverage. No pixels. |
| Layer pixel buffers | Rust `LayerPixelStore` | Immutable `Arc<RgbaImage>` values keyed by identifier. |
| Render graph | `layers::composite` | Rebuilt per render; holds no state between renders. |
| Preview cache | `LayerPixelStore` preview buffers, `ThumbnailCache` | Bounded, key-invalidated. |
| Final export render | `export_layer_composite` | Full-resolution, never reuses preview data. |

The frontend DOM and canvas are never the source of truth. Every pixel decision
is made in Rust; the webview only displays a PNG data URL that Rust produced.

## Colour space, honestly

Blending happens on **sRGB-encoded, non-linear 8-bit values**, the same encoding
PhotoForge has always stored and the same one its Phase 1–7.1 operations work
in. This is what most 8-bit editors do and it keeps 0.8.0 consistent with every
existing PhotoForge result.

It is **not** physically linear-light compositing. A 50% blend of black and
white produces sRGB 128, not the linear-light midpoint. PhotoForge does not
claim linear compositing, scene-referred rendering, or ICC colour management.
A future colour-management phase can add a linear working space; nothing in the
layer model prevents it, because every blend runs through one function.

## Alpha model

Layers store **straight (unassociated) alpha**: colour channels are independent
of the alpha channel, exactly as PNG stores them. Compositing converts to
premultiplied form only where it is mathematically required — inside a single
bilinear sample and inside the composite formula — and converts straight back.
Nothing at rest is premultiplied.

The composite formula is the W3C one, extended by a blend function:

```text
ao = as + ab * (1 - as)
Co = ((1 - ab) * as * Cs + ab * as * B(Cb, Cs) + (1 - as) * ab * Cb) / ao
```

A fully transparent result returns transparent black rather than dividing by
zero. Bilinear resampling interpolates in premultiplied space and treats
out-of-bounds neighbours as transparent, which is what keeps transparent edges
free of dark or light halos.

### Verified alpha cases

Each of these has a numerical test in `layers::blend` or `layers::composite`:

- opaque over opaque
- translucent over opaque
- opaque over transparent
- translucent over transparent
- transparent over transparent
- a fully transparent source leaving the backdrop untouched, in every mode
- a transparent backdrop leaving the source colour untouched, in every mode
- mask coverage multiplied by layer opacity
- group opacity applied to a finished group rather than to each child
- nested group transparency

### 8-bit accumulation

Every layer's result is quantized back to 8 bits before the next layer
composites onto it. Stacking many translucent layers therefore converges one
step short of fully opaque: fifty layers at 25% effective coverage settle at
alpha 254, not 255. This is inherent to an 8-bit compositor and is asserted in
the test suite rather than hidden.

## Blend modes

Sixteen modes are implemented. The twelve separable modes evaluate one channel
at a time; the four non-separable modes mix all three channels.

| Mode | `B(Cb, Cs)` |
| --- | --- |
| Normal | `Cs` |
| Multiply | `Cb · Cs` |
| Screen | `Cb + Cs − Cb·Cs` |
| Overlay | `HardLight(Cs, Cb)` |
| Darken | `min(Cb, Cs)` |
| Lighten | `max(Cb, Cs)` |
| Color Dodge | `0` if `Cb = 0`; `1` if `Cs = 1`; else `min(1, Cb / (1 − Cs))` |
| Color Burn | `1` if `Cb = 1`; `0` if `Cs = 0`; else `1 − min(1, (1 − Cb) / Cs)` |
| Soft Light | `Cb − (1 − 2Cs)·Cb·(1 − Cb)` for `Cs ≤ 0.5`; else `Cb + (2Cs − 1)·(D(Cb) − Cb)` |
| Hard Light | `Multiply(Cb, 2Cs)` for `Cs ≤ 0.5`; else `Screen(Cb, 2Cs − 1)` |
| Difference | `|Cb − Cs|` |
| Exclusion | `Cb + Cs − 2·Cb·Cs` |
| Hue | `SetLum(SetSat(Cs, Sat(Cb)), Lum(Cb))` |
| Saturation | `SetLum(SetSat(Cb, Sat(Cs)), Lum(Cb))` |
| Color | `SetLum(Cs, Lum(Cb))` |
| Luminosity | `SetLum(Cb, Lum(Cs))` |

`D(Cb) = ((16·Cb − 12)·Cb + 4)·Cb` for `Cb ≤ 0.25`, otherwise `√Cb`.
`Lum(C) = 0.3R + 0.59G + 0.11B`, and `SetLum` clips back into gamut using the
W3C `ClipColor` procedure.

### Numerical guarantees

- Inputs are clamped to `0…1` before any arithmetic, so NaN, infinity, and
  out-of-range channels cannot propagate. A brute-force test drives every mode
  with non-finite and out-of-range operands and asserts finite in-range output.
- Every singular endpoint of Color Dodge and Color Burn is handled explicitly
  and tested.
- Neutral operands are identities: multiply by white, screen with black,
  overlay/soft-light/hard-light with 0.5, difference and exclusion with black.
- Overlay and Hard Light are asserted to be transposes of one another.
- Mode identifiers are unique and round-trip through JSON; an unknown mode is
  rejected at deserialization rather than silently treated as Normal.

## Groups

Groups are **isolated**. Children composite onto a transparent buffer of their
own, and only the finished group result is blended into the parent using the
group's own mask, opacity, and blend mode. This is why two stacked opaque
children inside a 50% group read as a single 50% result rather than two
separately faded layers.

**Pass-through groups are not implemented.** An adjustment layer inside a group
affects only the layers below it *within that group*; it cannot reach the
backdrop outside. This is the simpler of the two correct models, and the one
0.8.0 commits to. PhotoForge does not claim Photoshop-compatible group
semantics.

## Adjustment layers

An adjustment layer stores an `EditOperation` — the same validated type the
destructive pipeline uses — and never stores output pixels. On every render it
re-evaluates that operation against the accumulated backdrop beneath it inside
its own group.

An adjustment layer changes colour without contributing coverage: the backdrop's
alpha is preserved exactly and only the colour channels move, weighted by the
layer's opacity and mask. This is why stacking adjustment layers never lightens,
darkens, or thickens a transparent edge.

Operations that change canvas dimensions (crop, rotate, straighten, perspective,
lens correction, horizontal reflection) cannot be adjustment layers and are
rejected both at validation and at render time. `decontaminate_colors` requires
an explicit selection and is likewise rejected.

## Transforms and sampling

Each layer's transform maps its own pixel grid into document space: flip, then
scale, then rotation around the layer centre, then translation. Pixels are not
resampled until render time, so repeated edits accumulate no resampling loss.

Sampling uses one of two paths, which are asserted to be byte-identical for the
cases both can handle:

- **Fast path** — a whole-pixel position with no mask, full opacity, and Normal
  blending is plain source-over. Fully transparent source pixels are skipped and
  fully opaque ones are copied directly.
- **General path** — everything else inverse-maps each destination pixel into
  layer space and samples bilinearly in premultiplied space. Whole-pixel
  coordinates still take an exact-copy branch, so an identity or integer
  translation is lossless either way.

Layer masks are stored at the layer's own pixel dimensions and are sampled at
the same layer-space coordinate as the pixels, so a mask travels with its layer
through translation, scale, rotation, and flips for free. Group and adjustment
layer masks live in canvas space instead, and validation enforces that.

## Determinism

A render traverses layers in a fixed order. Per-pixel work may be split into
disjoint row bands across at most eight scoped worker threads, but no row is
written twice and no parallel reduction occurs. The result therefore does not
depend on scheduling. Byte-identical repeated output is asserted directly,
including across a 50-layer document and a tall masked, blended, adjusted
document that exercises the parallel path.

## Preview scale

A preview resolves smaller pixel buffers and scales layer translations by
exactly the same ratio, so a preview is a faithful miniature of the export
rather than a differently composed image. Scale, rotation, and flips are
relative to each layer's own dimensions and are therefore already scale-free.

Radius-based operations (blur, sharpen, local contrast, denoise) are **not**
rescaled for preview. A preview of those adjustments is an approximation of the
full-resolution result, which is the behaviour PhotoForge has had since Phase 3;
0.8.0 does not change it.

## Fixtures

`layers::composite` contains deterministic numerical fixtures rather than
screenshot comparisons: red over blue, 50% red over blue, multiply, screen,
overlay, darken, lighten, difference, a masked layer, group opacity, an
adjustment plus mask, nested groups, and transparent edges. Each asserts exact
or ±1 channel values against hand-computed expectations.
