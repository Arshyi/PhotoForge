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

Covered: exposure/brightness, contrast, levels, HSL, temperature/tint, auto
white balance, local contrast, sharpen, edge-aware sharpen, denoise, deblock,
mild deblur, uneven-lighting correction, blur, black-and-white, and sepia.

Adjustment layers support visibility, opacity, blend mode, mask, rename,
reorder, duplicate, group membership, and undo/redo. Double-clicking one reopens
its controls.

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

Rendering is deterministic — single-threaded, fixed order, no parallel reduction
— and this is asserted directly, including across a 50-layer document.

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

Advanced actions live in contextual rows for the selected layer rather than
being always visible, so the panel does not become overloaded. The existing
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
| Rust | 481 | **646** | 165 |
| Frontend | 415 | **545** | 130 |
| **Total** | 896 | **1,191** | 295 |

Of these, 165 Rust tests and 126 frontend tests are layer-specific. All previous
tests still pass unchanged; no test, lint rule, or type check was weakened.

Backend coverage includes layer creation and deletion, stable identifiers,
ordering, nested groups, cycle rejection, opacity, visibility, locking,
transforms, all sixteen blend modes, alpha compositing, masks, adjustment
layers, merge, flatten, project serialization, project corruption and truncation,
migration and version rejection, archive traversal rejection, oversized
allocation rejection, renderer determinism, fast-path equivalence, cache
behaviour, and undo/redo.

Frontend coverage includes panel rendering, layer selection, thumbnails, drag
reorder (above, below, into a group, onto itself, and from a locked layer),
groups, visibility, lock state, opacity, blend selector, layer masks, adjustment
definitions, the active editing target, undo/redo labels and coalescing, tree
validation, and the simple-document fast-path predicate.

Two drag-and-drop tests initially passed for the wrong reason — jsdom does not
deliver `clientY` through `fireEvent`, so the geometry fell through a NaN
comparison. They were rewritten to construct the event explicitly and now assert
the visible drop hint as well as the resulting call.

## Validation performed

| Check | Result |
| --- | --- |
| `cargo fmt --check` | Clean |
| `cargo clippy --all-targets --all-features -- -D warnings` | Clean |
| `cargo test --all-targets --all-features` | 646 passed, 0 failed |
| `npm test` | 545 passed, 0 failed |
| `npm run check` (svelte-check) | 402 files, 0 errors, 0 warnings |
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
| 4000×3000, 1 layer | 16.5 ms | 2.7 ms |
| 6000×4000, 1 layer | 33.0 ms | 2.7 ms |
| 1920×1080, 1 layer | 2.7 ms | 1.9 ms |

Visibility toggle at preview scale: 1.1–1.2 ms. Reorder at preview scale:
2.0–2.7 ms. These take a fast path for whole-pixel placement with no mask, full
opacity, and Normal blending, which is asserted byte-identical to the general
path.

### Translucent and blended layers (the hard case)

Deliberately adversarial: every layer above the first at 78% alpha with Multiply
blending, so nothing can take the fast path.

| Scenario | Full render | Preview render | Opacity change | Flatten |
| --- | --- | --- | --- | --- |
| 4000×3000, 10 layers | 4,331 ms | 694 ms | 699 ms | 4,311 ms |
| 6000×4000, 10 layers | 8,619 ms | 615 ms | 617 ms | 8,630 ms |
| 1920×1080, 10 layers | 745 ms | 518 ms | 517 ms | 744 ms |
| 1920×1080, 50 layers | 3,960 ms | 2,745 ms | 2,749 ms | 3,945 ms |
| 1920×1080, 100 layers | 7,881 ms | 5,490 ms | 5,516 ms | 7,920 ms |

Other measurements: nested groups with three adjustment layers at 1920×1080,
579 ms full render; ten layers each with a full-resolution mask, 1,186 ms;
project save of a ten-layer 1920×1080 document, 146 ms producing 23.3 MB;
project load, 212 ms; registering a 6000×4000 buffer including preview
generation, 68 ms for 102.8 MB of store memory.

**Honest assessment.** Interactive editing of documents with a handful of opaque
layers is comfortably fast. Documents with many *translucent or blended* layers
are not: at roughly 55 ms per 1920×1080 blended layer, a 50-layer document takes
about 2.7 s per preview and a 100-layer document about 5.5 s. That is too slow
to feel interactive, and it is a real limitation of this release rather than a
measurement artefact. The cause is a per-pixel scalar compositing loop with no
tiling, no dirty-region tracking, no caching of unchanged group composites, and
no parallelism. Each of those is a viable improvement for a later phase; none of
them were implemented here.

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
directory, matching existing repository policy. Hashes independently recomputed
and verified after writing the manifest.

| Artifact | Size | SHA-256 |
| --- | --- | --- |
| `PhotoForge-portable.exe` | 15,995,392 bytes | `02cd2971c269a3c8e0d5d219768d6021ff2fd191d2ceaf4c815c4d2ee3a2063e` |
| `PhotoForge_0.8.0_x64-setup.exe` | 3,596,723 bytes | `b6f6a2ef210a1e60d86d6539843460e458f5f00f6db424c61e879acbef228559` |
| `PhotoForge_0.8.0_x64_en-US.msi` | 5,320,704 bytes | `54908e96572107541080fb0aed87b3864348a1d21ed20cc838437ae8603dda6e` |

`SHA256SUMS.txt` contains exactly these three entries. All three binaries report
`FileVersion` and `ProductVersion` 0.8.0 with `ProductName` PhotoForge; no stale
0.7.1 metadata remains. `Get-AuthenticodeSignature` reports **NotSigned** for all
three: no legitimate signing identity exists, and no self-signed substitute was
used.

## Packaging validation

The portable executable was launched and observed: it started, stayed running,
reported `Responding: True`, presented a main window titled `PhotoForge`, used
about 40 MB working set, and spawned the expected `msedgewebview2.exe` child. It
was then terminated cleanly.

---

# Not completed

Stated plainly, without hedging.

## Not implemented

1. **Pass-through groups.** Groups are isolated only.
2. **Curves and selective-colour adjustment-layer editors.** The compositor,
   project format, validation, and rendering handle both correctly and they can
   be created programmatically, but the 0.8.0 dialog edits scalar adjustments,
   levels, and HSL only.
3. **Autosave and crash recovery.** Unsaved layer changes are tracked and the
   user is prompted before an action would discard them, but no periodic
   recovery snapshot is written and no automatic restore after abnormal exit
   exists.
4. **Layer-aware workflow steps.** Workflows remain document-pipeline
   recordings, unchanged at schema version 1. Select-layer, apply-to-layer,
   create-adjustment-layer, and set-blend-mode steps do not exist. The design
   constraints for adding them are recorded in `workflows.md`.
5. **Planner layer awareness.** The Ollama and rule planners still emit only
   document-pipeline plans. They cannot fabricate layer identifiers because they
   have no layer vocabulary at all.
6. **Batch processing of project files.** Ordinary image batches are unchanged;
   `.photoforge` files are not accepted by batch.
7. **Multi-layer selection in the panel.** One layer is selectable at a time.
   Multi-select grouping exists in the tree API and is tested, but the panel does
   not expose it.
8. **Text, vector, smart-object, procedural, and neural layers.** Designed for,
   not built.
9. **GPU acceleration.** Compositing is CPU-only and single-threaded.
10. **Colour management.** No ICC handling; blending is in encoded sRGB.
11. **PSD compatibility.** PhotoForge cannot read or write Photoshop documents.

## Not verified

12. **The manual GUI matrix was not performed.** No interactive desktop
    automation was available in this environment. Creating, deleting,
    duplicating, reordering, dragging into and out of groups, collapsing,
    renaming, toggling visibility, changing opacity and blend mode, masks,
    adjustment layers, transforms, merge, flatten, undo/redo, save, reload, and
    export were **not** exercised by hand through the running GUI, and were
    **not** tested at 100%, 125%, 150%, or 200% display scaling. What was
    verified is process-level: the portable binary starts, responds, and shows
    its main window. Everything else rests on the 1,191 automated tests, which
    cover the panel's rendering and callbacks in jsdom but not real pointer
    input, real rendering, or DPI behaviour.
13. **NSIS and MSI install, launch, uninstall, and residue checks were not
    performed.** Both installers build and hash correctly, but installing them
    requires a UAC consent boundary that was not crossed. UAC was not bypassed or
    automated.
14. **No zero-network claim is made.** As in 0.7.1, the embedded WebView2
    runtime performs its own diagnostics that the embedding application does not
    fully control. PhotoForge application code makes no network request; the
    complete WebView2 process tree is not claimed to be silent.

## Known performance limitation

15. **Documents with many translucent or blended layers are slow** — roughly
    2.7 s per preview at 50 layers and 5.5 s at 100 layers, 1920×1080. Measured
    figures are in the performance section above. Tiling, dirty regions, cached
    group composites, and parallelism are all unimplemented.
