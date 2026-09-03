# Phase 8 results — layers and non-destructive editing

PhotoForge 0.8.0 turns the editor from a primarily single-document destructive
tool into a layer-based, non-destructive editing system. It adds no cloud
service, telemetry, account, generative image feature, Stable Diffusion, Flux,
semantic object insertion or removal, mandatory neural inference, scripting
runtime, or executable plugin.

Baseline for this phase was commit `16e6afa` (0.7.1), verified clean before any
change.

---

# Completed

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
addresses a layer by array position or display name.

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

Groups are **isolated**: children composite onto their own transparent buffer,
and only the finished result is blended into the parent with the group's mask,
opacity, and blend mode. Two stacked opaque children in a 50% group read as one
50% result.

**Pass-through groups are not implemented.** An adjustment layer inside a group
affects only the layers below it within that group. This is the simpler correct
model and 0.8.0 commits to it. PhotoForge does not claim Photoshop-compatible
group semantics.

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
delete, apply, replace from selection, load as selection, edit by painting, and
a mask thumbnail in the panel. Partial coverage is supported throughout.

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

A document that is still one plain full-canvas pixel layer also keeps using the
original 0.7.1 render and export path entirely. A test asserts compositing such
a document reproduces the opened pixels byte for byte, and that the document
pipeline on top of it matches the destructive path exactly.

Selection tools, refinement, named masks, mask import/export, crop, straighten,
perspective, lens correction, guided planning, and Ollama planning are unchanged.

## Autosave and recovery

While a document has unsaved layer changes, a bounded recovery snapshot is
written to the local `PhotoForge
ecovery` folder every 90 seconds. Snapshots
use their own `.photoforge-recovery` extension so they can never be mistaken for
or overwrite a saved project; at most three are kept, oldest pruned first; each
is written atomically; and the user's project file is never touched.

A snapshot is an ordinary project container plus a sidecar recording its origin,
so restoring goes through the same validated, checksummed reader a project does
— a corrupt snapshot is rejected, not half-read. On startup a snapshot triggers a
recovery offer; recovered work stays marked unsaved until the user saves it
where they chose; saving clears the snapshots. Nothing is uploaded.

## Layer-aware workflows

Workflow schema version 2 adds layer steps: select layer, set visibility, set
opacity, set blend mode, create adjustment layer, apply operations to a layer,
create a mask from the selection, merge down, flatten, and export the composite.
Version 1 files carry none, still load unchanged, and are not rewritten. A
version 1 document carrying layer steps is rejected as a mismatch.

Steps name layers through deterministic selectors: an exact identifier, the
active layer, the layer a previous step created, a unique name, the bottom, or
the top. A selector that cannot resolve — or a name matching more than one layer
— fails the replay rather than retargeting, and the whole workflow is resolved
before any of it is applied, so a replay is never half-applied.

## Planner integration

`EditPlan` gained an optional layer plan. Planners are restricted to selectors
they cannot fabricate — only the active layer and the layer the plan just
created — and may not propose merge, flatten, or export. **This is enforced by
validation, not convention**: a plan containing an identifier selector is
rejected with `invalid_plan`. The brief's own example (create a curves
adjustment layer, mask it from the selection, reduce opacity to 60%) validates;
an invented identifier does not. Ollama still receives no image, mask, or layer
data.

## Batch

Ordinary image batches are unchanged. A saved `.photoforge` project is now
accepted as a batch input: batch renders its visible composite, applies the
project's own document pipeline, then the workflow's operations, and exports the
result. The project file is only ever read — a test asserts it is byte-identical
afterwards — and a corrupt project fails that one item rather than the run.

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

## Automated tests

| Suite | Before (0.7.1) | After (0.8.0) | Added |
| --- | --- | --- | --- |
| Rust | 481 | **688** | 207 |
| Frontend | 415 | **593** | 178 |
| **Total** | 896 | **1,281** | 385 |

Of these, 199 Rust tests and 174 frontend tests are layer-specific. All previous
tests still pass unchanged; no test, lint rule, or type check was weakened.

Backend coverage includes layer creation and deletion, stable identifiers,
ordering, nested groups, cycle rejection, opacity, visibility, locking,
transforms, all sixteen blend modes, alpha compositing, masks, adjustment
layers, merge, flatten, project serialization, project corruption and truncation,
migration and version rejection, archive traversal rejection, oversized
allocation rejection, renderer determinism, fast-path equivalence, parallel-band
determinism, cache behaviour, undo/redo, recovery snapshots (round trip,
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

## Validation performed

| Check | Result |
| --- | --- |
| `cargo fmt --check` | Clean |
| `cargo clippy --all-targets --all-features -- -D warnings` | Clean |
| `cargo test --all-targets --all-features` | 688 passed, 0 failed |
| `npm test` | 593 passed, 0 failed |
| `npm run check` (svelte-check) | 406 files, 0 errors, 0 warnings |
| `npm run build` | Succeeded |
| `cargo tauri build` | Succeeded, both bundles produced |
| `cargo audit` | 0 vulnerabilities; 17 pre-existing unmaintained-dependency warnings, unchanged from 0.7.1 |
| `npm audit --omit=dev` | 0 vulnerabilities |
| Secret scan | No secrets in the diff |
| Machine-path scan | No absolute machine paths in the diff |
| `git diff --check` | Clean |

Phase 8 adds **no new Rust or npm dependency**.

## Performance

Measured with `cargo run --release --example layer_benchmark` on this machine.
Every figure is a wall-clock measurement of the same compositor the application
uses. Nothing is estimated.

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

**Honest assessment.** Interactive editing is now comfortable across the range
tested: a 10-layer preview updates in under 110 ms even at 24 MP, a 50-layer
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

## Release artifacts

Built from this tree at version 0.8.0 and stored in the ignored `release/`
directory, matching existing repository policy. Hashes were written to the
manifest and then independently recomputed and compared.

| Artifact | Size | SHA-256 |
| --- | --- | --- |
| `PhotoForge-portable.exe` | 16,500,736 bytes | `6dc805c2304580332b5748eafba4631368d8b7d74489c6165e286da5066e8c32` |
| `PhotoForge_0.8.0_x64-setup.exe` | 3,685,835 bytes | `df57a7fe8a305e1fc5db6da7519a586bd1a4dbae6958e1d6ad50582373db8d4a` |
| `PhotoForge_0.8.0_x64_en-US.msi` | 5,443,584 bytes | `5287405d35490d473092fb1b64ef1d4c2996fc7128aced5b9d6835aefae78e9e` |

`SHA256SUMS.txt` contains exactly these three entries and all three re-verified
as MATCH. The portable executable and NSIS installer report `ProductVersion`
0.8.0; no stale 0.7.1 metadata remains. `Get-AuthenticodeSignature` reports
**NotSigned** for all three: no legitimate signing identity exists, and no
self-signed substitute was used.

## Packaging validation

The rebuilt portable executable was launched and observed: it started, stayed
running, reported `Responding: True`, presented a main window titled
`PhotoForge`, used about 42 MB working set, and spawned the expected
`msedgewebview2.exe` child. It was then terminated cleanly, and the local
recovery folder was confirmed empty afterwards — nothing was left behind by a
session with no unsaved work.

---

# Remaining limitations and verification gaps

Stated plainly, without hedging.

## Not implemented

1. **Pass-through groups.** Groups are isolated only.
2. **Text, vector, smart-object, procedural, and neural layers.** Designed for,
   not built.
3. **GPU acceleration.** Compositing is CPU-only. The current CPU renderer may
   use deterministic row bands across at most eight threads.
4. **Colour management.** No ICC handling; blending is in encoded sRGB.
5. **PSD compatibility.** PhotoForge cannot read or write Photoshop documents.

## Not verified

6. **The manual GUI matrix was not performed.** No interactive desktop
    automation was available in this environment. Creating, deleting,
    duplicating, reordering, dragging into and out of groups, collapsing,
    renaming, toggling visibility, changing opacity and blend mode, masks,
    adjustment layers, the curve editor, transforms, merge, flatten, undo/redo,
    save, reload, recovery, and export were **not** exercised by hand through
    the running GUI, and were **not** tested at 100%, 125%, 150%, or 200%
    display scaling. What was verified is process-level: the portable binary
    starts, responds, and shows its main window. Everything else rests on the
    automated suite, which covers the panel's rendering and callbacks in jsdom
    but not real pointer input, real rendering, or DPI behaviour.
7. **NSIS and MSI install, launch, uninstall, and residue checks were not
    performed.** Both installers build and hash correctly, but installing them
    requires a UAC consent boundary that was not crossed. UAC was not bypassed or
    automated.
8. **No zero-network claim is made.** As in 0.7.1, the embedded WebView2
    runtime performs its own diagnostics that the embedding application does not
    fully control. PhotoForge application code makes no network request; the
    complete WebView2 process tree is not claimed to be silent.

## Known performance limitation

9. **Documents with many translucent or blended layers still have visible
   latency** — roughly 440 ms per preview at 50 layers and 904 ms at 100 layers,
   1920x1080, on the measured machine. Tiling, dirty regions, and cached group
   composites remain unimplemented; bounded row-band parallelism is implemented.
