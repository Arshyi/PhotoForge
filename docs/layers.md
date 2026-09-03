# Layers

PhotoForge 0.8.0 turns the editor into a layer-based, non-destructive system.
This document describes the layer model, what each layer type does, how existing
tools interact with layers, and what is deliberately out of scope.

For the rendering mathematics see [compositing.md](compositing.md); for the
on-disk format see [project-format.md](project-format.md).

## The document model

```text
LayerDocument
├── schemaVersion      1
├── canvasWidth/Height document canvas, independent of any layer's size
├── layers[]           root stack; index 0 is the BOTTOM
└── activeLayerId      stable identifier, never an index
```

Four things are kept strictly separate:

| Concept | Meaning |
| --- | --- |
| Document canvas | The output size. Only document geometry changes it. |
| Layer dimensions | Each layer's own pixel grid. It need not match the canvas. |
| Layer transform | Where that grid sits in the canvas. Non-destructive. |
| Viewport | Zoom and pan. Presentation only; never affects rendering. |

Layers may be smaller than the canvas, larger than it, or partially or wholly
outside it. The renderer computes each layer's bounding box, clips it to the
canvas, and skips layers that miss entirely.

## Layer types

### Pixel layer

Raster content. The layer holds a `pixelId` referring to an immutable buffer in
the session pixel store, plus that buffer's dimensions. The tree itself never
carries pixels, which is what makes cloning, undo, and serialization cheap.

### Group layer

A named, nestable container. Groups have their own visibility, lock, opacity,
blend mode, optional mask, and collapse state. Group compositing is **isolated**
— see [compositing.md](compositing.md#groups).

### Adjustment layer

Parametric, non-destructive colour and tone. The layer stores an `EditOperation`
and re-evaluates it on every render; it never stores baked pixels. Changing a
parameter recomputes the result.

Supported adjustments include exposure/brightness, contrast, levels, HSL,
temperature/tint, auto white balance, local contrast, sharpen, edge-aware
sharpen, denoise, deblock, mild deblur, uneven-lighting correction, blur,
black-and-white, and sepia — the restoration operations from Phases 2–3
included.

### Types designed for but not implemented

Text, vector, smart-object/linked, procedural, and neural layers are **not**
implemented in 0.8.0. The model is shaped so that adding one means adding a
`LayerContent` variant and a compositor branch, without changing the tree, the
mask model, transforms, history, or the project container. No stub or partial
implementation of them ships.

## Layer properties

Every layer carries: a stable identifier, display name, type, visibility, lock,
opacity, blend mode, transform, optional mask, collapse state, creation and
modification timestamps, and bounded custom metadata. Parent and ordering come
from the layer's position in the tree.

Identifiers are random and stable. Nothing in PhotoForge addresses a layer by
array position or display name — not the UI, not history, not workflows, not the
project format.

## Tree operations and invalid states

Supported: reorder, move into and out of groups, move whole groups, duplicate,
rename, delete, visibility, lock, opacity, blend mode, collapse, and grouping or
ungrouping siblings.

The following are rejected rather than tolerated:

- a layer parented to itself
- a group moved into one of its own descendants (a cycle)
- duplicate identifiers, including a group whose children collide with existing
  layers
- a missing parent or a missing layer
- nesting deeper than 16 or more than 512 layers in one document
- a non-finite, zero, or out-of-range transform
- an opacity outside 0…1, or a blank or overlong name
- a mask whose dimensions do not match its layer's space
- an adjustment layer holding a geometry operation

A rejected operation leaves the tree exactly as it was. Validation runs
iteratively with an explicit stack, so a hostile or malformed tree cannot
exhaust the stack before its depth is checked, and it runs again in Rust at the
trust boundary on every command.

## Layer masks

Layer masks reuse the Phase 7 mask engine unchanged. A mask is a `MaskSnapshot`
— the same versioned, checksummed, run-length-encoded 8-bit coverage bitmap that
selections use. **There is no second mask representation in PhotoForge.**

A mask supports partial coverage, an enabled flag, and an inverted flag.
Supported operations: create from the active selection, create white (reveal
all), create black (hide all), invert, disable, delete, apply (bake into the
layer's pixels), replace from selection, load as selection, and edit by
painting.

A pixel layer's mask lives in that layer's own pixel space, so it moves, scales,
rotates, and flips with the layer automatically. Group and adjustment layer
masks live in canvas space. Converting between a canvas selection and a layer
mask maps through the layer transform in both directions; a non-invertible
transform fails closed rather than producing garbage.

## Layer transforms

Each pixel layer supports translate, scale, rotate, flip horizontal, and flip
vertical. The transform is stored, not applied, so repeated edits accumulate no
resampling loss. `Reset Transform` returns to identity; `Rasterize` bakes the
current transform into a new canvas-sized buffer and returns the layer to
identity.

Per-layer transforms are entirely separate from document crop, straighten,
perspective, and lens correction, which continue to reshape the whole canvas
exactly as they did in Phase 6 and 7.

## The active editing target

PhotoForge makes the target of the next brush stroke explicit. The Layers panel
carries a three-way selector and a sentence stating what will change:

- **Layer** — tools change the selected layer's pixels
- **Mask** — tools change the selected layer's mask (highlighted, because
  painting into a mask when you meant pixels is the classic layer-editor
  mistake)
- **Selection** — tools change the document selection and no layer

The Mask target is only available when the selected layer has a mask, and
selecting an adjustment layer moves the target off Layer automatically.

## How existing tools map onto layers

| Tool group | Operates on |
| --- | --- |
| Selection tools (rectangle, ellipse, lasso, brush, wand, colour range) | The document selection, in canvas space |
| Selection refinement, named masks, mask import/export | The document selection |
| Crop, straighten, perspective, lens correction, rotate, reflect | Document geometry, applied to the finished composite |
| Global adjustments (brightness, contrast, saturation, gamma, blur, sharpen, restoration) | Routed by the **Adjustments go to** selector |
| Curves, levels, HSL, selective colour in the Professional workspace | The document pipeline, as before |
| Guided and Ollama plans | The document pipeline, as before |
| Export | The visible composite |

### Preserving existing behaviour

The **Adjustments go to** selector defaults to **The whole document (as
before)**, and in that mode every existing control behaves exactly as it did in
0.7.1. Nothing about an existing workflow changes unless the user explicitly
chooses another target:

- **The selected layer, applied directly** — bakes the adjustment into that
  pixel layer, producing a new immutable buffer.
- **A new adjustment layer** — creates a non-destructive layer instead.

Returning a slider to its default always goes to the document pipeline, so a
control still works as its own reset regardless of the selected target.

A document that is still one plain full-canvas pixel layer also keeps using the
original 0.7.1 render and export path entirely. A test asserts that compositing
such a document reproduces the opened pixels byte for byte, and that the
document pipeline on top of it matches the destructive path exactly.

## Merge and flatten

- **Merge Down** merges the selected layer into the one beneath it.
- **Merge Selected** is available by grouping and then flattening that group.
- **Flatten Image** replaces every layer with one composited background layer.

All three are destructive document operations and all three are undoable. Merged
results are canvas-sized with an identity transform, because a merged layer no
longer has the individual placements of the layers that produced it. Merging
preserves the document's own bottom-to-top order rather than the order
identifiers were listed in, so a merge can never reorder pixels.

## Undo and redo

Layer changes have their own undo stack, tracked on the same shared history
timeline as edit and selection history, so one Ctrl+Z always undoes the most
recent action of any kind.

Every listed operation is undoable: create, delete, rename, duplicate, reorder,
group, ungroup, visibility, lock, opacity, blend mode, transform, create mask,
edit mask, delete mask, apply mask, create adjustment layer, change adjustment
parameters, merge, and flatten.

History stores whole document trees rather than diffs. This is affordable
because the tree carries no pixels and because untouched subtrees are shared by
reference between entries — editing one layer never deep-copies the rest. A
continuous slider drag and a drag-and-drop reorder each coalesce into a single
logical undo step through a coalescing key. Trivial metadata changes therefore
cost a few hundred bytes, not a document snapshot.

Selecting a different layer is a view change, not an edit: it never enters
history and never marks the project as modified.

## Memory model

- Pixel buffers are immutable and reference-counted. Several layers may share
  one buffer, so **duplicating a layer costs one reference, not a pixel copy**.
- Any edit that changes pixels registers a new buffer instead of overwriting the
  old one — copy-on-write, which is what makes undo cheap.
- Buffers stay alive while any document in the undo or redo stacks still
  references them, so an undone merge can still be redone. Everything else is
  released.
- Opening an image or loading a project rebinds the store and drops every
  previous buffer, so editing images in sequence cannot accumulate memory.
- The store is bounded at 1 GiB total and 1,024 distinct buffers. Exceeding
  either fails the edit rather than the process.
- Preview buffers are generated once per buffer at the canvas's preview scale.
- Thumbnails use a bounded 96-entry least-recently-used cache keyed by
  everything that can change a thumbnail. Renaming, locking, or selecting a
  layer does not invalidate it.

## Limits

| Limit | Value |
| --- | --- |
| Layers per document | 512 |
| Group nesting depth | 16 |
| Distinct pixel buffers | 1,024 |
| Total pixel memory | 1 GiB |
| Canvas and layer dimensions | 20,000 px per side, 40 MP |
| Layer name | 120 characters |
| Layer metadata | 16 entries, 512 characters each |

## Known limitations in 0.8.0

- **Pass-through groups are not implemented.** Groups are isolated.
- **Curves and selective colour have no adjustment-layer parameter editor.** The
  compositor, project format, and workflows handle them correctly, and they can
  be created programmatically, but the 0.8.0 dialog edits scalar adjustments,
  levels, and HSL only.
- **Blending is not linear-light and PhotoForge is not colour managed.** See
  [compositing.md](compositing.md#colour-space-honestly).
- **Compositing is CPU-only.** There is no GPU acceleration in this release.
- **Many translucent or blended layers are slow.** See
  [phase-8-results.md](phase-8-results.md) for measured figures.
- **No PSD support.** PhotoForge cannot read or write Photoshop documents.
- Text, vector, smart-object, procedural, and neural layers are not implemented.
- Multiple-layer selection in the panel is limited to one layer at a time;
  grouping operates on the selected layer, and multi-select grouping is
  available through the tree API but not yet through the panel.
