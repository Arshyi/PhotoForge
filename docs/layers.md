# Layers

PhotoForge 0.8.0 turns the editor into a layer-based, non-destructive system.
This document describes the layer model, what each layer type does, how existing
tools interact with layers, and what is deliberately out of scope.

For the rendering mathematics see [compositing.md](compositing.md); for the
on-disk format see [project-format.md](project-format.md).

## 2026-09-04 continuation — 0.8.2 working source

This guide includes current unbuilt 0.8.2 source changes continued from Claude's
work. The available 0.8.1 packages and earlier benchmark/browser records do not
contain or verify all of these changes. The current source-level evidence is
722 Rust unit tests plus 39 IPC/integration tests passing, clean Rust formatting
and Clippy, a successful Rust release build, and a clean `npx tsc --noEmit`.
The frontend Vite/Svelte check and Vitest rerun is currently blocked in this
sandbox by esbuild's directory-access denial; an earlier escalated run passed
50 files and 814 tests before the latest merge-safety tests were added.
That evidence is source-level, not packaged-app acceptance. Native Windows
GUI/DPI acceptance, elevated MSI testing, and trusted signing are not claimed.

Automated real-App tests with mocked Tauri calls now cover correct backend
project identities, zero-operation project compositing, replacement-buffer
previews, Save As and asynchronous-layer operation locks, scoped recovery
cleanup, and atomic layer-aware workflow replay/Undo/Redo. These are integration
regressions, not hands-on packaged-app evidence. Current-layer sampling now
resolves the active document's immutable composite with document/revision
guards, and mask-target shape/brush gestures map into pixel-layer mask space
before committing one history entry. Rust IPC tests and frontend lifecycle/
component tests cover the stale and routing paths; no native GUI claim follows.

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
blend mode, optional mask, and collapse state, plus a choice between **isolated**
and **pass-through** compositing — see [compositing.md](compositing.md#groups).
Isolated is the default and is what every pre-0.8.0 project restores as.

### Adjustment layer

Parametric, non-destructive colour and tone. The layer stores an `EditOperation`
and re-evaluates it on every render; it never stores baked pixels. Changing a
parameter recomputes the result.

Supported adjustments: exposure/brightness, contrast, levels, curves, HSL,
temperature/tint, selective colour, auto white balance, local contrast, sharpen,
edge-aware sharpen, denoise, deblock, mild deblur, uneven-lighting correction,
blur, black-and-white, and sepia — the restoration operations from Phases 2-3
included.

Every one of them is editable after the fact. Double-clicking an adjustment
layer reopens its controls: sliders for scalar parameters, numeric fields plus a
gamma slider for levels, per-band sliders for HSL, and a direct-manipulation
curve editor with an RGB/R/G/B channel selector where points can be dragged,
added by clicking the grid, removed with Alt-click or Delete, and nudged with
the arrow keys.

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

Identifiers are random and stable. The UI, history, and project format preserve
those identifiers rather than treating mutable array positions as identity.
Workflows may also resolve a unique layer name or a relative selector; ambiguous
names and missing targets fail closed.

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
layer's pixels), replace from selection, and load as selection. Mask-target
painting/routing is implemented in the working source for shape and brush
gestures. The stroke is mapped through the layer transform, respects inverted
masks, and commits atomically; colour-range and magic-wand sampling remain
Selection-target operations by design, after which the selection can be turned
into a mask.

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

The 0.8.2 working source exposes numeric Center X/Y, Width/Height, Scale X/Y,
Rotation, and Keep proportions controls, plus an on-canvas move/resize/rotate
box for an unlocked pixel layer. A gesture stages a preview and commits one
history change when completed; cancellation discards its staged transform.
Controls expose off-canvas bounds, flips, reset, and rasterization.

`Ctrl+T` toggles transform mode. While it is active, arrow keys nudge and Shift
uses the larger nudge, Enter finishes, and Escape cancels. `Ctrl+Shift+H` and
`Ctrl+Shift+U` flip horizontally and vertically; `Ctrl+Shift+R` resets. Text
fields keep their own typing/arrow behavior. These routes have automated
coverage, not native Windows pointer/DPI acceptance.

Bilinear remains the backward-compatible default; nearest-neighbour preserves
hard edges. The sampling choice is saved in the transform and applied to both
artwork and its mask. Regression tests check their texel alignment and preserve
byte-equivalence for whole-pixel placements. Groups and adjustment layers do
not expose these interactive pixel-transform controls.

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
| Generated Guided and Ollama plans | Document operations; current generators emit no layer steps |
| Imported/manually edited layer workflows | Staged layer tree plus document pipeline, committed together |
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

A document that is still one plain full-canvas pixel layer backed by the exact
originally opened pixel buffer keeps the original render/export path. A loaded
project or a replaced/flattened buffer uses the compositor even when it also has
one plain layer, so it cannot fall back to stale original pixels. Automated
tests assert the pixel-equivalent fast path and the correct lifecycle routing.

## Merge and flatten

- **Merge Down** merges the selected layer into the one beneath it.
- **Merge Selected** is available by grouping and then flattening that group.
- **Flatten Image** replaces every layer with one composited background layer.

All three are destructive document operations and all three are undoable. Merged
results are canvas-sized with an identity transform, because a merged layer no
longer has the individual placements of the layers that produced it. Merging
preserves the document's own bottom-to-top order rather than the order
identifiers were listed in, so a merge can never reorder pixels. Merge Down also
requires a contiguous sibling range and fails closed when an omitted backdrop
could affect the result through a non-Normal blend, adjustment layer, or
pass-through group; the same guard runs in the frontend workflow and Rust IPC
boundary.

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

## Autosave and recovery

While a document has unsaved changes, PhotoForge writes a bounded recovery
snapshot to `%LOCALAPPDATA%\PhotoForge\recovery` every 90 seconds. Snapshots
use the `.photoforge-recovery` extension so they can never be mistaken for, or
overwrite, a project the user saved; at most three are kept, oldest pruned
first; and each is written atomically.

A snapshot is an ordinary project container plus a small sidecar recording where
it came from, so restoring one goes through exactly the same validated,
checksummed reader a project does - a corrupt snapshot is rejected rather than
half-read.

On startup, if a snapshot is present, PhotoForge offers to recover it. Recovered
work is marked unsaved until the user saves it somewhere they chose. Restoring
or failing a save does not delete the source snapshot. A successful save clears
only snapshots tracked for that working document, not unrelated documents'
snapshots. Nothing is uploaded and no cloud autosave exists.

## Layer-aware workflows

Workflows can carry layer steps at schema version 2: select layer, set
visibility, set opacity, set blend mode, create adjustment layer, apply
operations to a layer, create a mask from the selection, merge down, flatten,
and export the composite. Version 1 files carry none and still load without
rewriting their source files; new exports use version 2.

Steps name layers through deterministic selectors - an exact identifier, the
active layer, the layer a previous step created, a unique name, the bottom, or
the top. A selector that cannot be resolved, or a name that matches more than
one layer, fails the replay rather than retargeting silently, and the whole
workflow is structurally checked before worker execution. Steps then resolve
against staged intermediate trees; visible state is published only after the
replay succeeds. Global operations and layer changes enter one Undo/Redo action.
Failed worker operations leave the visible tree and document pipeline unchanged.

The workflow library supports layer-aware import/export and a validated
layer-step JSON editor. Recording captures only document operations; it does
not automatically record layer actions. Export is allowed once as the final
layer step and asks the user for a destination. Batch intentionally rejects
workflows containing layer steps before it writes outputs.

Planner schema validation supports layer steps and restricts prospective
planner selectors to the active/last-created layer, rejecting merge, flatten,
export, and fabricated identifiers. Both current backend planners still emit
empty layer-step lists. Automatic natural-language layer-action generation is
not implemented; validation/application support is not generation support.

## Known limitations, including the 0.8.2 continuation

- **Blending is not linear-light and PhotoForge is not colour managed.** See
  [compositing.md](compositing.md#colour-space-honestly).
- **Compositing is CPU-only.** There is no GPU acceleration in this release.
- **Many translucent or blended layers have recorded latency.** See
  [phase-8-results.md](phase-8-results.md) for historical measurements; changed
  0.8.2 source has not yet been benchmarked as a release.
- **No PSD support.** PhotoForge cannot read or write Photoshop documents.
- Text, vector, smart-object, procedural, and neural layers are not implemented.
- Multiple selection is limited to siblings: Ctrl-, Cmd-, or Shift-clicking adds
  a layer to the selection, and a selection that would span different parents is
  trimmed back, because only siblings can be grouped.
- No complete native Windows GUI/DPI matrix, elevated all-users MSI lifecycle,
  trusted signing, or process-tree zero-network verification is claimed.
