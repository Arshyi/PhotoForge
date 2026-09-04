# Phase 8 results — layers and non-destructive editing

PhotoForge 0.8.0 turns the editor from a primarily single-document destructive
tool into a layer-based, non-destructive editing system. It adds no cloud
service, telemetry, account, generative image feature, Stable Diffusion, Flux,
semantic object insertion or removal, mandatory neural inference, scripting
runtime, or executable plugin.

Baseline for this phase was commit `16e6afa` (0.7.1), verified clean before any
change.

**0.8.1** is a bug-fix release on top of the 0.8.0 implementation. A real-browser
interaction pass found three user-visible defects in the layer UI; because the
0.8.0 artifacts had already been built and hashed, the version was raised and the
binaries rebuilt so the shipped packages actually contain the fixes. No feature
or architecture changed in 0.8.1.

## 2026-09-04 continuation — 0.8.2 working source, not a packaged release

Claude's unfinished 0.8.2 changes are being continued in the working tree. The
0.8.1 artifacts, hashes, benchmark figures, browser observations, and installer
 results below are historical records: they do not validate the changed 0.8.2
source. No rebuilt 0.8.2 release bundle or new installer acceptance result is
claimed here. Current source-level gates are 722 Rust unit tests plus 39
IPC/integration tests passing, clean Rust formatting and Clippy, a successful
Rust release build, and a clean `npx tsc --noEmit`. The frontend Vite/Svelte
check and Vitest rerun is blocked in this sandbox by esbuild directory access;
an earlier escalated run passed 50 files and 814 tests before the latest
merge-safety tests were added. These results must not be read as packaged GUI
acceptance.

Verified continuation regressions currently cover:

- Opening or recovering a project establishes the backend document identity,
  and even a single-layer, zero-operation project renders through the layer
  compositor. Replacing the original pixel buffer, including flattening, cannot
  silently return to the originally opened image's preview.
- Save As holds the document stable while its dialog is pending. Open and
  native drag-drop are rejected while asynchronous layer creation owns the
  document. These are real-App integration tests with mocked Tauri calls, not
  packaged Windows GUI tests.
- Recovery keeps the source snapshot after restore and after a failed save.
  A successful save deletes only snapshots tracked for that working document,
  never all documents' recovery files.
- Imported schema-v2 workflows replay document operations and layer steps
  together. A mixed replay enters one Undo/Redo action; a failed pixel worker
  leaves the visible tree and document pipeline unchanged. Work is staged in a
  private tree and only published after success.
- Compositor fast paths and serial/parallel row bands honor cancellation.
  Nearest-neighbour masks now select the same texel as nearest-neighbour
  artwork. Regression coverage compares fast/general output across small and
  tall canvases, both sampling modes, and transparent/translucent/opaque pixels.
- Sampling commands resolve the current layer composite with document/revision
  guards, and mask-target shape/brush gestures map into layer-local mask space
  before committing one history entry. Colour-range and magic-wand sampling
  remain Selection-target operations by design.
- Merge Down and workflow `merge_down` now fail closed unless selected layers
  form a contiguous sibling range whose omitted backdrop cannot affect the
  result through a non-Normal blend, adjustment layer, or pass-through group.

The working source also adds the unlocked-pixel-layer transform panel and
on-canvas handles: numeric position, size, scale, rotation, proportion lock,
flips, reset, rasterize, and bilinear/nearest sampling. Transform math and
component tests are automated evidence; no Windows pointer/DPI acceptance is
implied. Current-layer sampling and mask-target routing are implemented and
covered by Rust IPC plus frontend lifecycle/component regressions; the blocked
frontend rerun and lack of packaged GUI automation are recorded above.

Planner support is a schema/validation capability, not automatic generation:
both backend planners still emit no layer steps. Imported/manually edited layer
steps can replay; recording captures document operations only. Batch deliberately
rejects any workflow containing layer steps before output work begins.

The native 73-item GUI/DPI matrix, elevated all-users MSI lifecycle, trusted
Authenticode signing, and complete-process-tree zero-network claim remain
unfulfilled. The historical binaries remain explicitly unsigned.

---

# Implementation record

Behavior described here is not blanket release acceptance. The dated
continuation above distinguishes current source fixes from the historical
validation and packaging evidence later in this document.

## Architecture

A new `src-tauri/src/layers` domain holds the layer tree, blend mathematics,
per-layer transforms, the deterministic compositor, the session pixel store, the
selection/layer-mask bridge, and the project container. It depends on `domain`
for `EditOperation`, on `mask` for coverage bitmaps, and on `image_processing`
to evaluate an adjustment layer. It has no Tauri, frontend, or filesystem
dependency beyond the project container.

The split that makes this work:

- **The frontend owns document state.** `LayerDocument` is pure data with no
  pixels, so cloning, undo, and serialization are cheap.
- **Rust owns pixels.** `LayerPixelStore` holds immutable, reference-counted
  buffers keyed by identifier.
- **Rust re-validates everything** at the trust boundary before any pixel work,
  iteratively rather than recursively.

The existing stale-result protocol is preserved: layer renders take their own
request generation and bounded gate, recheck document and request identifiers
before encoding, and clone only `Arc` handles before moving CPU work to a
blocking worker. The session lock is never held during pixel work.

## Layer types

| Type | Status |
| --- | --- |
| Pixel layer | Implemented |
| Group layer | Implemented, nestable to depth 16 |
| Adjustment layer | Implemented, parametric, never baked |
| Text, vector, smart object, procedural, neural | **Not implemented**, by design |

Adding a future type means adding a `LayerContent` variant and a compositor
branch — the tree, mask model, transforms, history, and project container do not
change. No stub of an unimplemented type ships.

Every layer carries a stable random identifier, display name, type, visibility,
lock, opacity, blend mode, transform, optional mask, collapse state, creation
and modification timestamps, and bounded custom metadata. Nothing in PhotoForge
stores a layer reference by a mutable array position. Workflows may intentionally
resolve a unique display name, but reject ambiguous matches.

## Tree and invalid states

Reorder, move into and out of groups, move whole groups, duplicate, rename,
delete, visibility, lock, opacity, blend mode, collapse, group, and ungroup are
all implemented. Rejected and tested: self-parenting, a group moved into its own
descendant, duplicate identifiers (including a group whose children collide),
missing parents and layers, depth beyond 16, more than 512 layers, non-finite or
singular transforms, out-of-range opacity, blank or overlong names, mismatched
mask dimensions, and geometry operations in adjustment layers. A rejected
operation leaves the tree exactly as it was.

Cycles are additionally impossible by construction: the tree is by-value
nesting, so a layer cannot contain itself. Move is the only operation that could
introduce one, and it checks descendancy before detaching anything.

## Group behaviour

Both models are implemented and a group can be switched between them.

**Isolated** (the default) composites children onto their own transparent
buffer, and only the finished result is blended into the parent with the group's
mask, opacity, and blend mode. Two stacked opaque children in a 50% group read
as one 50% result, and an adjustment inside the group cannot reach the backdrop
beneath it.

**Pass-through** hands the children the accumulated backdrop instead, so an
adjustment inside the group also affects the layers below it. The group's own
opacity and mask then decide how much of the reworked backdrop replaces the
original; that mix happens in premultiplied space so differing coverage cannot
darken or lighten the seam. A pass-through group must use the Normal blend mode,
because pass-through *is* its blend behaviour — a second mode would be
ambiguous, and validation rejects it rather than silently picking a meaning.

Groups default to isolated, and a project written before pass-through existed
omits the field and restores as isolated, so no existing document changes
appearance. Tested: the two models genuinely differ for the same adjustment; a
fully open pass-through group renders identically to putting its children in the
root stack; opacity fades between the two backdrops; a mask limits where the
rework applies; pass-through never creates coverage over a transparent backdrop;
and nested pass-through groups compose through both levels.

PhotoForge still does not claim full Photoshop-compatible group semantics — only
that these two models behave as described here.

## Adjustment layers

An adjustment layer stores a validated `EditOperation` and re-evaluates it on
every render against the accumulated backdrop beneath it in its own group. It
never stores output pixels, and it is never silently rasterized.

Covered: exposure/brightness, contrast, levels, curves, HSL, temperature/tint,
selective colour, auto white balance, local contrast, sharpen, edge-aware
sharpen, denoise, deblock, mild deblur, uneven-lighting correction, blur,
black-and-white, and sepia.

Every one is editable after creation. Double-clicking an adjustment layer
reopens its controls: sliders for scalar parameters, numeric fields plus a gamma
slider for levels, per-band sliders for HSL, six controls for selective colour,
and a direct-manipulation curve editor with an RGB/R/G/B channel selector where
points are dragged, added by clicking the grid, removed with Alt-click or
Delete, and nudged with the arrow keys. The curve editor is fully keyboard
operable and its point maths is clamped to exactly the ranges Rust validates.

Adjustment layers support visibility, opacity, blend mode, mask, rename,
reorder, duplicate, group membership, and undo/redo.

Critically, an adjustment layer **changes colour without contributing
coverage**: backdrop alpha is preserved exactly and only colour channels move,
weighted by opacity and mask. Stacking them never thickens a transparent edge.

## Layer masks

Layer masks reuse the Phase 7 mask engine unchanged — a `MaskSnapshot`, the same
versioned, checksummed, run-length-encoded 8-bit coverage bitmap selections use.
**There is no second mask representation in PhotoForge.**

Implemented: create from selection, create white, create black, invert, disable,
delete, apply, replace from selection, load as selection, and
a mask thumbnail in the panel. Partial coverage is supported throughout.
Mask-target painting/routing is implemented in the working source for shape and
brush gestures. The stroke is mapped through the layer transform, respects
inverted masks, and commits atomically; colour-range and magic-wand sampling
remain Selection-target operations by design, after which the selection can be
turned into a mask.

A pixel layer's mask lives in that layer's own pixel space and is sampled at the
same layer-space coordinate as its pixels, so it travels with the layer through
translation, scale, rotation, and flips automatically. Group and adjustment
masks live in canvas space, which validation enforces.

## Blend modes

All sixteen requested modes are implemented: Normal, Multiply, Screen, Overlay,
Darken, Lighten, Color Dodge, Color Burn, Soft Light, Hard Light, Difference,
Exclusion, Hue, Saturation, Color, and Luminosity. Each formula is documented in
[compositing.md](compositing.md) and asserted against hand-computed values.

Numerically: inputs are clamped before any arithmetic so NaN and infinity cannot
propagate; a brute-force test drives every mode with non-finite and
out-of-range operands; every singular endpoint of dodge and burn is handled and
tested; neutral operands are asserted to be identities; Overlay and Hard Light
are asserted to be transposes; identifiers round-trip through JSON and an
unknown mode is rejected rather than silently treated as Normal.

## Compositing and alpha

Straight (unassociated) alpha at rest; premultiplied only inside a bilinear
sample and inside the composite formula, then converted straight back. The
composite is the W3C `source-over` formula extended by a blend function, with a
fully transparent result returning transparent black rather than dividing by
zero. Bilinear sampling interpolates premultiplied and treats out-of-bounds
neighbours as transparent, which is what prevents halos.

Verified with numerical tests: opaque over opaque, translucent over opaque,
opaque over transparent, translucent over transparent, transparent over
transparent, a fully transparent source never changing the backdrop in any mode,
a transparent backdrop never tinting the source in any mode, masks multiplied by
opacity, group opacity, and nested group transparency.

Rendering keeps a fixed layer order. Expensive per-row work may be split into
disjoint row bands across at most eight scoped worker threads, with no shared row
and no parallel reduction. Output therefore stays deterministic regardless of
scheduling; this is asserted directly across a 50-layer document and on the
parallel path.

**8-bit accumulation is real and documented**: stacking many translucent layers
converges one step short of opaque (alpha 254 after fifty layers at 25%
coverage). The test asserts this rather than hiding it.

## Colour handling, stated honestly

Blending happens on **sRGB-encoded, non-linear 8-bit values** — the same
encoding every previous PhotoForge operation uses. This is **not** physically
linear-light compositing, and PhotoForge is **not** colour managed. No ICC
rewrite was attempted. A future colour phase can add a linear working space
because every blend runs through one function.

## Transforms

Translate, scale, rotate, flip horizontal, and flip vertical, stored rather than
applied, so repeated edits accumulate no resampling loss. `Reset Transform` and
`Rasterize` are both available. Forward and inverse mappings are asserted to be
mutual for every component combination. Non-finite, zero-scale, out-of-range,
and oversized transforms are rejected.

Per-layer transforms are entirely separate from document crop, straighten,
perspective, and lens correction, which still reshape the whole canvas as in
Phase 6 and 7.

## Canvas model

Document canvas, layer dimensions, layer transform, and viewport are separate.
Layers smaller than, larger than, partially outside, and entirely outside the
canvas are each tested; the renderer computes each layer's bounds, clips to the
canvas, and skips layers that miss entirely.

## Layers panel

Thumbnail, name, type icon and label, visibility, lock, mask thumbnail, group
indentation, expand/collapse, selection state, opacity, and blend mode. Drag and
drop reorder with above/below/inside drop bands and a visible drop hint, plus
keyboard-accessible move buttons. Create pixel layer, group, adjustment layer,
and place-image-as-layer. Duplicate, delete, rename, group, ungroup, merge down,
flatten, reset transform, rasterize, and the full mask action set.

Ctrl-, Cmd-, or Shift-clicking adds a layer to the selection so several can be
grouped at once; a selection that would span different parents is trimmed back,
because only siblings can be grouped. Advanced actions live in contextual rows
for the selected layer rather than being always visible, so the panel does not
become overloaded. The existing
PhotoForge visual identity is preserved — the panel uses the same tokens,
spacing, and control shapes as the Selection workspace.

## Thumbnails

Real rendered thumbnails with preserved aspect ratio, a checkerboard behind
transparency, a type badge for adjustment layers, and a separate mask thumbnail.
Rendering is lazy and cached in a bounded 96-entry least-recently-used cache
keyed by everything that can change a thumbnail. Renaming, locking, or selecting
a layer does not invalidate it; a group's key covers every descendant. Thumbnails
render from preview-scale buffers, never at full resolution.

## Active editing target

A three-way selector — Layer, Mask, Selection — with a sentence stating what the
next stroke will change. The Mask target is highlighted in a warning colour and
is only available when the selected layer has a mask. Selecting an adjustment
layer moves the target off Layer automatically.

## Existing tool integration

The **Adjustments go to** selector defaults to *The whole document (as before)*,
and in that mode every existing control behaves exactly as in 0.7.1. The other
options are *The selected layer, applied directly* and *A new adjustment layer*.
Returning a slider to its default always goes to the document pipeline, so a
control still works as its own reset.

A document that is still one plain full-canvas pixel layer backed by the exact
originally opened pixel buffer keeps using the original render/export path.
Loaded projects and replacement buffers use the compositor even if their trees
look equally simple. A test asserts compositing such
a document reproduces the opened pixels byte for byte, and that the document
pipeline on top of it matches the destructive path exactly.

Selection tools, refinement, named masks, mask import/export, crop, straighten,
perspective, lens correction, guided planning, and Ollama planning are unchanged.

## Autosave and recovery

While a document has unsaved changes, a bounded recovery snapshot is written to
the local `%LOCALAPPDATA%\PhotoForge\recovery` folder every 90 seconds. Snapshots
use their own `.photoforge-recovery` extension so they can never be mistaken for
or overwrite a saved project; at most three are kept, oldest pruned first; each
is written atomically; and the user's project file is never touched.

A snapshot is an ordinary project container plus a sidecar recording its origin,
so restoring goes through the same validated, checksummed reader a project does
— a corrupt snapshot is rejected, not half-read. On startup a snapshot triggers a
recovery offer; recovered work stays marked unsaved until the user saves it
where they chose. A successful save clears only snapshots tracked for that
document; failed saves retain them. Nothing is uploaded.

## Layer-aware workflows

Workflow schema version 2 adds layer steps: select layer, set visibility, set
opacity, set blend mode, create adjustment layer, apply operations to a layer,
create a mask from the selection, merge down, flatten, and export the composite.
Version 1 files carry none and still load without rewriting their source file.
New exports use schema 2, including exports of imported version 1 workflows. A
version 1 document carrying layer steps is rejected as a mismatch.

Steps name layers through deterministic selectors: an exact identifier, the
active layer, the layer a previous step created, a unique name, the bottom, or
the top. A selector that cannot resolve — or a name matching more than one layer
— fails the replay rather than retargeting. Structural preflight precedes worker
execution, selectors are resolved against each staged intermediate tree, and
visible state is committed only after successful completion. A pixel-worker
failure leaves document operations and layers unchanged. Export is allowed
once, as the final step, and obtains its destination from the user rather than
from workflow data.

The library imports/exports layer-aware JSON and exposes a validated layer-step
JSON editor. Recording still captures only the document operation list; layer
actions are not automatically recorded. Mixed operations/layer replay is one
shared Undo/Redo action.

## Planner integration

`EditPlan` gained an optional layer-step list, and its validator restricts
prospective planner output to selectors
they cannot fabricate — only the active layer and the layer the plan just
created — and may not propose merge, flatten, or export. **This is enforced by
validation, not convention**: a plan containing an identifier selector is
rejected with `invalid_plan`. The brief's own example (create a curves
adjustment layer, mask it from the selection, reduce opacity to 60%) validates;
an invented identifier does not. Ollama still receives no image, mask, or layer
data. Both current backend generators still construct empty `layer_steps`:
automatic natural-language generation of layer actions is not implemented.
Schema validation and frontend application support must not be described as
proof that either planner currently proposes these actions.

## Batch

Ordinary image batches are unchanged. A saved `.photoforge` project is now
accepted as a batch input: batch renders its visible composite, applies the
project's own document pipeline, then the workflow's operations, and exports the
result. The project file is only ever read — a test asserts it is byte-identical
afterwards — and a corrupt project fails that one item rather than the run.
Workflows containing any layer step are intentionally rejected for batch
preview/run, before outputs or a batch log are written, rather than silently
dropping those steps.

## Undo and redo

Layer changes have their own undo stack on the same shared history timeline as
edit and selection history, so one Ctrl+Z undoes the most recent action of any
kind. Every required operation is undoable: create, delete, rename, duplicate,
reorder, group, ungroup, visibility, lock, opacity, blend mode, transform,
create mask, edit mask, delete mask, apply mask, create adjustment layer, change
adjustment parameters, merge, and flatten.

History stores whole trees but never full-document snapshots of pixels: the tree
carries no pixels, and untouched subtrees are shared by reference between
entries, which a test asserts directly. A continuous slider drag and a
drag-and-drop reorder each coalesce into one logical undo step. Selecting a
different layer never enters history.

Pixel buffers stay alive while any document in the undo or redo stacks
references them, so an undone merge can still be redone; everything else is
released as editing proceeds.

## Project format

`.photoforge`, a bounded PhotoForge container (magic `PFORGE\r\n`, format
version 1) with a JSON manifest plus `layers/` and `masks/` PNG entries. It
stores the canvas, the full tree, names, visibility, locks, opacity, blend
modes, transforms, masks with their flags, adjustment parameters, metadata,
timestamps, the document operation pipeline, and both version numbers. It never
stores only flattened pixels.

ZIP was deliberately not used: a general archive format brings decompression
bombs and traversal along with it, while this container has no general
decompression stage at all — every payload is a PNG whose dimensions are
declared, checked, and decoded through the existing bounded decoder. It also
adds no new dependency. The honest trade-off is that a `.photoforge` file is not
readable by a ZIP tool.

Security: entry names validated against traversal, absolute roots, backslashes,
empty and dotted segments, and a restricted character set; duplicate names
rejected; every structural field bounds-checked before allocation; FNV-1a-64
checksums on each payload and on the whole file; trailing data rejected;
`deny_unknown_fields` on the manifest; bounded at 1 GiB file, 32 MiB manifest,
4,096 entries, 256 MiB per entry.

Saving is atomic — temporary sibling file, flushed, renamed — and a test asserts
a failed save leaves the previous project byte-identical with no temporary file
left behind.

## Import, export, merge, migration

Opening PNG, JPEG, and WebP produces a document with one background pixel layer,
preserving alpha, orientation handling, and metadata exactly as before. Source
files are never modified. Importing another image into an existing document as a
new layer is implemented.

Export renders the visible composite — all visible layers, masks, adjustments,
blend modes, clipped to the canvas, with expected alpha — then applies the
document pipeline. Exporting a flattened image does not flatten the project.

Merge Down, Merge Selected (via group then flatten), and Flatten Image are
implemented, destructive, and undoable. Merges preserve the document's own
bottom-to-top order rather than the order identifiers were listed in.

Legacy 0.7.x images, workflow files, mask files, and selection sessions all load
unchanged; none are wrapped, rewritten, or migrated. Future format and schema
versions are rejected safely with typed errors.

## Keyboard shortcuts

`Ctrl+Shift+N` new layer, `Ctrl+J` duplicate, `Ctrl+G` group, `Ctrl+Shift+G`
ungroup, `Delete` delete the selected layer. Every existing binding was checked
first; none of these were previously assigned, and Ctrl+Z/Y, Ctrl+O/S, Ctrl+A/D,
Ctrl+Shift+I, and the single-letter tool keys are untouched.

## Historical 0.8.0/0.8.1 automated test record

The following counts belong to the earlier implementation/release record. They
are not current 0.8.2 totals. Current continuation gates are recorded at the
top of this document; the historical counts remain here for comparison only.

| Suite | Before (0.7.1) | After (0.8.0) | Added |
| --- | --- | --- | --- |
| Rust | 481 | **700** | 219 |
| Frontend | 415 | **607** | 192 |
| **Total** | 896 | **1,307** | 411 |

Of these, 211 Rust tests and 188 frontend tests are layer-specific. All previous
tests still pass unchanged; no test, lint rule, or type check was weakened.

Backend coverage includes layer creation and deletion, stable identifiers,
ordering, nested groups, cycle rejection, opacity, visibility, locking,
transforms, all sixteen blend modes, alpha compositing, masks, adjustment
layers, merge, flatten, project serialization, project corruption and truncation,
migration and version rejection, archive traversal rejection, oversized
allocation rejection, renderer determinism, fast-path equivalence, parallel-band
determinism, both group compositing models, cache behaviour, undo/redo, recovery snapshots (round trip,
pruning, corruption, extension safety, leaving the source project untouched),
layer workflow selectors and fail-closed resolution, planner-safety enforcement,
workflow schema 1/2 compatibility, and batch project composites.

Frontend coverage includes panel rendering, layer selection, thumbnails, drag
reorder (above, below, into a group, onto itself, and from a locked layer),
groups, visibility, lock state, opacity, blend selector, layer masks, adjustment
definitions, the active editing target, undo/redo labels and coalescing, tree
validation, the simple-document fast-path predicate, curve point maths (moving,
adding, removing, sampling, validity), nested selective-colour fields, layer
workflow execution and fail-closed replay, and multiple layer selection.

Two drag-and-drop tests initially passed for the wrong reason — jsdom does not
deliver `clientY` through `fireEvent`, so the geometry fell through a NaN
comparison. They were rewritten to construct the event explicitly and now assert
the visible drop hint as well as the resulting call.

## Historical 0.8.0/0.8.1 validation performed

| Check | Result |
| --- | --- |
| `cargo fmt --check` | Clean |
| `cargo clippy --all-targets --all-features -- -D warnings` | Clean |
| `cargo test --all-targets --all-features` | 700 passed, 0 failed |
| `npm test` | 607 passed, 0 failed |
| `npm run check` (svelte-check) | 410 files, 0 errors, 0 warnings |
| `npm run build` | Succeeded |
| `cargo tauri build` | Succeeded, both bundles produced |
| `cargo audit` | 0 vulnerabilities; 17 pre-existing unmaintained-dependency warnings, unchanged from 0.7.1 |
| `npm audit --omit=dev` | 0 vulnerabilities |
| Secret scan | No secrets in the diff |
| Machine-path scan | No absolute machine paths in the diff |
| `git diff --check` | Clean |

Phase 8 adds **no new Rust or npm dependency**.

## Historical 0.8.0/0.8.1 performance

The prior run recorded these wall-clock measurements with
`cargo run --release --example layer_benchmark`. They have not been rerun for
the changed 0.8.2 compositor and must not be presented as current performance.

### Opaque layers (the common case)

| Scenario | Full render | Preview render |
| --- | --- | --- |
| 4000x3000, 1 layer | 16.3 ms | 2.7 ms |
| 6000x4000, 1 layer | 33.7 ms | 2.4 ms |
| 1920x1080, 1 layer | 3.1 ms | 2.1 ms |

Visibility toggle at preview scale: ~1.0-1.2 ms. Reorder: ~1.9-3.0 ms. These
take a fast path for whole-pixel placement with no mask, full opacity, and
Normal blending, asserted byte-identical to the general path.

### Translucent and blended layers (the hard case)

Deliberately adversarial: every layer above the first at 78% alpha with Multiply
blending, so nothing can take the fast path.

| Scenario | Full render | Preview render | Opacity change | Flatten |
| --- | --- | --- | --- | --- |
| 4000x3000, 10 layers | 683 ms | 107 ms | 117 ms | 694 ms |
| 6000x4000, 10 layers | 1,349 ms | 96 ms | 98 ms | 1,373 ms |
| 1920x1080, 10 layers | 128 ms | 83 ms | 87 ms | 126 ms |
| 1920x1080, 50 layers | 643 ms | 440 ms | 443 ms | 638 ms |
| 1920x1080, 100 layers | 1,277 ms | 904 ms | 898 ms | 1,290 ms |

Other measurements: nested groups with three adjustment layers at 1920x1080,
257 ms full render; ten layers each with a full-resolution mask, 227 ms; project
save of a ten-layer 1920x1080 document, 144 ms producing 23.3 MB; project load,
213 ms; registering a 6000x4000 buffer including preview generation, 66 ms for
102.8 MB of store memory.

### What changed and what it cost

Two optimisations, both asserted to leave output byte-identical:

1. **An exact fast path** for opaque whole-pixel placement, which is what a
   background layer and a moved layer normally are. A 6000x4000 single-layer
   full render went from 964 ms to 34 ms.
2. **Deterministic row-band parallelism** across up to eight threads. Bands are
   disjoint slices of the destination and nothing is reduced, so the result does
   not depend on how work was divided or scheduled — a dedicated test renders a
   tall masked, blended, adjusted document five times and requires identical
   bytes. A 100-layer preview went from 5,490 ms to 904 ms.

**Historical assessment.** In that measured build, a 10-layer preview updated
in under 110 ms even at 24 MP, a 50-layer
document in ~440 ms, and a 100-layer document in ~900 ms. The last of those is
noticeable rather than instant. The remaining cost is a per-pixel scalar loop
with no tiling, no dirty-region tracking, and no caching of unchanged group
composites; each is a real improvement available to a later phase, and none is
implemented here. There is no GPU acceleration.

## Memory

Immutable reference-counted buffers, shared between layers, so duplicating a
layer costs a reference rather than a pixel copy. Copy-on-write on every pixel
edit. Buffers released when unreachable from the document and history. The store
is bounded at 1 GiB and 1,024 buffers and fails the edit rather than the
process. Opening an image or project releases every previous buffer. Thumbnails
use a bounded 96-entry LRU cache. History stores no pixels and shares untouched
subtrees by reference.

## Threading and cancellation

Every layer command moves CPU work to a blocking worker via `spawn_blocking` and
clones only `Arc` handles first, so the UI thread and the session lock are never
held during pixel work. Renders take a bounded gate and recheck document and
request identifiers before encoding, so a stale async result cannot overwrite
newer document state. The compositor accepts a cancellation flag and checks it
per layer and per row.

## Large-document and malformed-input safety

Protected and tested: enormous layer dimensions, hundreds of layers, recursive
group depth, integer overflow in dimension arithmetic, memory exhaustion,
malformed project files, corrupt mask references, missing pixel buffers, invalid
blend modes, invalid opacity, NaN transforms, singular transforms, and malformed
adjustment parameters. Validation traversal is iterative; compositor recursion
is bounded by a depth limit checked both at validation and at render time. Deeply
nested JSON is rejected by the parser before validation is reached.

No project file can trigger network access, executable loading, script
execution, plugin loading, or a shell command.

## Historical 0.8.0/0.8.1 real-browser interaction validation

Performed after the 0.8.0 implementation as a validation and hardening pass. It
is a distinct level of evidence from the jsdom suite and from packaged desktop
testing, and the three must not be conflated.

**Real-browser testing is not equivalent to packaged PhotoForge testing at
Windows 100%, 125%, 150%, and 200% display scaling.**

### How it was done

| | |
| --- | --- |
| Browser | Chromium 148.0.7778.280, `devicePixelRatio` 1 |
| Served by | The project's own Vite dev server |
| Under test | The real `LayersPanel`, `AdjustmentLayerDialog`, `CurveEditor`, `MaskThumbnail`, and `SliderControl` components, driving the real `layers/tree`, `layers/history`, and `layers/adjustments` modules |
| Harness | `harness.html` + `src/harness/`, not a Vite build input, so it never reaches `dist/` (verified) |
| Input | Genuine pointer clicks, click-drags, hovers, modifier-clicks, typing, and key presses dispatched to the page |

The whole layer UI stack has no Tauri import, so it mounts and runs unmodified.
Backend-only operations — merge, flatten, rasterize, apply mask,
mask-from-selection, project save and load, export, place-image — were **not
simulated**; the harness records them as unavailable. Mask fixtures are real
`MaskSnapshot` values built and checksummed in TypeScript by the application's
own `decodedCoverageChecksum`, so mask rendering exercised the ordinary path.

### Exercised

Selection, additive multi-selection via Ctrl/Cmd/Shift-click, visibility, lock,
group expand and collapse, nested groups to depth 6 and an attempted depth 20,
indentation and `aria-level`, layer creation (pixel, group, adjustment),
duplicate, delete, rename (open, type, Enter, Escape), opacity by pointer drag,
blend-mode selection, pass-through and isolated group switching, reorder by the
move controls, undo and redo, adjustment dialog open and close, adjustment type
switching, curve editor point add/drag/endpoint/clamping, 53-row panel
scrolling, and thumbnail rendering.

Drag and drop: Chromium produced genuine `dragstart`/`dragover`/`drop` events
from synthesized mouse drags for **some** gestures and not others, so native
HTML5 drag is only intermittently drivable under automation. Where it fired it
was correct: dropping a layer into a group moved it there, and dragging a group
onto its own descendant was rejected with the cycle notice, left the tree
byte-identical, and created no history entry. The move controls exercise the
same `onreorder` path deterministically and were used to confirm reorder,
undo, and redo.

### Viewport sizes

1920×1080, 1536×864, 1366×768, 1280×720, 1024×768, 800×600, and a narrow
420×900. At every size: no horizontal page overflow, the panel stayed inside the
viewport, no control fell past an edge, no control collapsed below 8 px, no
layer name truncated, and the layer list scrolled correctly.

### Browser zoom

Browser zoom changes the CSS-pixel viewport, so the zoom levels were covered by
their equivalent viewports on a 1920×1080 window: 100% → 1920×1080, 125% →
1536×864, 150% → 1280×720, 200% → 960×540. All were clean. **This is browser
zoom coverage, not Windows display scaling**, and it does not reproduce
fractional device-pixel rasterisation.

### Bugs found and fixed

1. **The panel reported the wrong layer count.** The header counted the rows
   currently on screen, so collapsing a group changed "6 layers" to "3 layers"
   even though no layer had been removed. Now counts the document.
2. **The rename field was never focused.** It rendered unfocused, so typing
   straight after pressing Rename went nowhere and neither Enter nor Escape
   reached its key handler — rename was effectively unusable without an extra
   click. Now focuses on open.
3. **Curve points at the grid edges could not be grabbed.** With an unpadded
   `0 0 100 100` viewBox the point circles sat half outside the SVG and
   `overflow: hidden` clipped them; `elementFromPoint` at an endpoint's centre
   returned the container instead of the circle. Both endpoints were therefore
   permanently ungrabbable, and any point dragged to full black or full white
   became stuck there. The viewBox now carries a margin wider than the point
   radius. Verified afterwards by dragging an endpoint to 50% output with a real
   pointer while its input stayed anchored at 0.

Each fix has a jsdom regression test. The clipping test asserts the viewBox
margin rather than the clipping itself, because jsdom has no layout and cannot
reproduce it — the defect was only observable in a real browser.

### Confirmed correct, not changed

Pointer coordinates mapped exactly: a click at 40% input / 70% output produced
0.404 / 0.699, and an opacity drag to 25% and 75% produced 23% and 77%. No drag
offset error at any viewport. Curve point ordering stayed strictly increasing
when a point was dragged past its neighbours. No stuck drag state after
releasing outside the control. Selection changes correctly stayed out of undo
history while edits entered it. Slider drags coalesced into one undo step.
Collapsing a group while its child was selected kept the selection valid. The
console stayed clean throughout: no errors, warnings, or rejected promises.

### Not a Phase 8 defect

Below 700 px the application's own stylesheet hides the entire inspector
`aside`, including the Layers panel. That is a pre-existing Phase 1–7 responsive
rule, not a layers bug; the harness reproduces the panel at narrow widths by not
using an `aside`.

### Limitations of this pass

- Global keyboard shortcuts (Ctrl+Shift+N, Ctrl+J, Ctrl+G, Ctrl+Shift+G, Delete)
  and their suppression while typing live in `App.svelte`'s window handler,
  which requires Tauri. They were **not** exercised in the browser.
- Layer masks were exercised with TypeScript-built fixtures; the backend mask
  commands were not.
- There is no interactive translate/scale/rotate UI in 0.8.0 — the panel offers
  only Reset transform and Rasterize — so per-layer transform gestures could not
  be tested.
- Native HTML5 drag fired only intermittently under automation.
- Select-all-on-rename could not be confirmed, because the automation's typing
  re-collapses the selection; only the focus fix is verified.

## Historical 0.8.1 release artifacts

Built from the earlier **0.8.1** source, not the current working tree, and stored
in the ignored `release/` directory. The prior release record reports these
hashes and their independent comparison; they are not 0.8.2 artifact hashes.

| Artifact | Size | SHA-256 |
| --- | --- | --- |
| `PhotoForge-portable.exe` | 16,552,448 bytes | `45c09ddef7166d9232ee6cf19c3a7c6523957aff2a32ea7c46e138b584ca94a9` |
| `PhotoForge_0.8.1_x64-setup.exe` | 3,706,580 bytes | `ad037cd540f8695e182dfe3ec410f2d410c5fa0ab02c1afb47b1988d79263769` |
| `PhotoForge_0.8.1_x64_en-US.msi` | 5,476,352 bytes | `0487ea2a9c5b6a0dbef95f3ba712a284b1116282278176a82b35f7ad7cc6c58a` |

`SHA256SUMS.txt` contains exactly these three entries and all three re-verified
as MATCH. The portable executable and NSIS installer report `ProductVersion`
0.8.1; the superseded 0.8.0 bundles were removed from `release/` so the manifest
describes exactly what ships. `Get-AuthenticodeSignature` reports
**NotSigned** for all three: no legitimate signing identity exists, and no
self-signed substitute was used.

## Historical 0.8.1 packaging validation

**Portable.** The rebuilt executable started, stayed running, reported
`Responding: True`, presented a main window titled `PhotoForge`, used about
42 MB working set, and spawned the expected `msedgewebview2.exe` child. It was
terminated cleanly, and the local recovery folder was confirmed empty
afterwards — nothing left behind by a session with no unsaved work.

**NSIS, full per-user lifecycle.** Performed end to end without crossing a UAC
boundary, because the installer supports a current-user install:

| Step | Result |
| --- | --- |
| Silent install (`/S /CURRENTUSER`) | Exit code 0 |
| Installed files | `photoforge.exe` (16,552,448 bytes) and `uninstall.exe` in `%LOCALAPPDATA%\PhotoForge` |
| Registration | `HKCU` uninstall entry: name `PhotoForge`, version `0.8.1`, correct install location and uninstall string |
| Start Menu | `PhotoForge.lnk` created, resolving to the installed executable |
| Installed version metadata | `ProductName` PhotoForge, `ProductVersion` 0.8.1 |
| Launch | Started, `Responding: True`, window titled `PhotoForge`, ~30 MB working set, terminated cleanly |
| Silent uninstall (`/S /CURRENTUSER`) | Exit code 0 |
| Residue | Install directory removed, Start Menu shortcut removed, `HKCU` uninstall key removed, no stray process, no configuration or recovery files left |

One honest detail: the executable inside the NSIS package hashes
`d25d3509cc49cd9af08fdc39043c7095b9c17defdc85d7d4673805baaeffdb62`, which is
**not** the same as the standalone portable binary. That is expected — Tauri
patches the executable with NSIS bundle-type information before packaging it —
but it means the installed binary is a variant of the portable one rather than
a byte-identical copy, and the manifest hashes cover the shipped artifacts, not
the executable extracted from inside them.

**MSI: still not exercised.** The MSI is an all-users package whose install
requires elevation. UAC was not bypassed or automated, so its install, launch,
uninstall, and residue behaviour remain unverified.

---

# Remaining limitations and verification gaps

Stated plainly, without hedging.

## Not implemented

1. **Text, vector, smart-object, procedural, and neural layers.** Designed for,
   not built.
2. **GPU acceleration.** Compositing is CPU-only. The current CPU renderer may
   use deterministic row bands across at most eight threads.
3. **Colour management.** No ICC handling; blending is in encoded sRGB.
4. **PSD compatibility.** PhotoForge cannot read or write Photoshop documents.

## Not verified

5. **The packaged desktop GUI matrix was not performed.** A real-browser
    interaction pass was completed and is recorded above, but it is a different
    level of evidence: it drives the components in Chromium, not the packaged
    application in its WebView2 window, and viewport size is not Windows display
    scaling. No interactive desktop automation was available. Creating,
    deleting,
    duplicating, reordering, dragging into and out of groups, collapsing,
    renaming, toggling visibility, changing opacity and blend mode, masks,
    adjustment layers, the curve editor, transforms, merge, flatten, undo/redo,
    save, reload, recovery, and export were **not** exercised by hand through
    the running GUI, and were **not** tested at 100%, 125%, 150%, or 200%
    display scaling. What was verified of the packaged build is process-level:
    it starts, responds, shows its main window, and installs and uninstalls
    cleanly. Real pointer, drag, focus, and layout behaviour is covered by the
    browser pass; **Windows DPI behaviour is covered by neither.**
6. **The MSI lifecycle was not exercised.** The MSI builds and hashes
    correctly, but it is an all-users package whose installation requires
    elevation, and UAC was not bypassed or automated. The NSIS installer's full
    per-user lifecycle *was* verified and is recorded above.
7. **No zero-network claim is made.** Earlier observation recorded two
    Microsoft TLS connections from WebView2 and no observed socket from the
    PhotoForge Rust process. This is scoped observation, not proof that the
    complete process tree is network-silent. Optional local Ollama use must also
    be distinguished from offline editing.
8. **Production Authenticode signing remains unavailable.** No trusted signing
    identity/service has been supplied; the historical artifacts are unsigned,
    and no self-signed substitute is claimed as production signing.

## Known performance limitation

9. **Documents with many translucent or blended layers have recorded visible
   latency** — the historical build measured roughly 440 ms per preview at 50
   layers and 904 ms at 100 layers, 1920x1080. Current 0.8.2 timings are pending.
   Tiling, dirty regions, and cached group
   composites remain unimplemented; bounded row-band parallelism is implemented.
