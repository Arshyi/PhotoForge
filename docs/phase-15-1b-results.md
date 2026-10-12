# Phase 15.1b — live clipping

An increment between [15.1](phase-15-1-results.md) (opening Photoshop files) and 15.2 (writing
them). 15.1 imported a clipped layer by baking the base's coverage into its alpha, which matched
the picture for a Normal-blended base and gave up the relation: moving the base did not move the
clipped layer. This increment adds the relation to the layer model, to all three renderers, to the
operation engine and to the Layers panel, and has the Photoshop importer use it.

Status: **implemented and verified as listed; not committed, not released.** The version is still
0.14.0. It is on top of 15.1 on branch `phase-15-1-psd-import`.

## Completed

* **A `clipped` flag on every layer.** Not written when false, so a document that uses no clipping
  serialises exactly as before and a project written earlier loads unchanged.
* **The semantics Photoshop calls "Blend Clipped Layers as Group"**, written down in
  [compositing.md](compositing.md#clipping): a base and the layers clipped to it are composited as
  one unit, each clipped layer *atop* what is already there (it moves colour and leaves coverage
  alone), the whole with the base's blend mode and opacity. A half-transparent base stays
  half-transparent however opaque the layer clipped to it is; an adjustment layer clipped to a
  layer changes that layer and nothing else.
* **All three renderers** — the 8-bit one, the float full-frame one and the tiled one — and the
  tile cache, whose keys now name a base that is drawn through its clipping group's buffer.
* **One operation, `core.layer.set_clipped`** (30 registered operations; the registry snapshot, the
  panel-plan fixture and the operation-parity fixture are regenerated), with the structural rules
  that keep a clipping group whole: moving a base carries what is clipped to it; deleting one
  promotes the first layer clipped to it; a copy of a base goes above its group; grouping releases
  the lowest layer of the new group; the bottom layer of any stack is never clipped.
* **The Layers panel**: a "Clip to layer below" box for the selected layer (disabled, with the
  reason, for the bottom layer), a bar and a "Clipped" label on clipped rows, and a recorded macro
  step.
* **A defect in the panel that was there before**, found while reading how a drop becomes a move
  index: dropping a layer onto a row *above* it in a stack of three or more landed it one place too
  high (dropping `a` above `c` in `a b c d` gave `b c d a`). The index is now counted against the
  stack with the dragged layer — and anything clipped to it — taken out, as the engine counts it.
* **The Photoshop importer keeps the relation.** A clipped layer is imported clipped with its pixels
  untouched; a group can be a base; a base with "Blend Clipped Layers as Group" off, a pass-through
  group that is a base, and a clipped layer whose base could not be imported are each named in the
  report.

## Verification

Same machine and settings as 15.1 (dev profile, dependency debug info reduced per invocation).

| Gate | Result |
| --- | --- |
| `cargo fmt --check` | clean |
| `cargo clippy --all-targets --all-features -- -D warnings`, and the same with no GPU / no inference / no plugins | clean (all four) |
| `cargo test --all-targets --all-features --no-fail-fast` | **1,541 passed** on the tree that held this increment alone (15.1 ended at 1,495); **1,561** once 15.1c and 15.1d were on top |
| No GPU / no inference / no plugins | **1,533 / 1,532 / 1,501 passed** on that tree; **1,554 / 1,553 / 1,522** on the final one — the whole matrix was re-run there, 0 failed in every configuration |
| `npx tsc --noEmit`, `npm run check` | clean; 510 files, 0 errors, 0 warnings |
| `npx vitest run --maxWorkers=4` | 85 files, **1,285 passed**, 0 failed (15.1 ended at 1,264) |
| `npm run build` | built |

Two things about that frontend line. With the default worker count the full suite once failed one
test (`LayersPanelStress`: "renames and deletes a layer buried deep in the stack", 5.2 s against a
5 s limit) and passed on the next run; alone it takes 1.4–1.8 s with the 15.1 panel and 1.6–1.8 s
with this one, so the failure is load, not this change. And vitest reports one *unhandled error*,
from `CommandPalette.test.ts`'s test that makes a command throw on purpose; that file and the
component are not touched here.

New Rust tests: 11 in `clip.rs` (grouping, and the three renderers' behaviour for opaque, half
transparent, blended, adjusted, hidden, grouped and orphaned cases), the random-tree check of the
tiled renderer against the float one with clipping at every level, 14 through the real engine, 8 in
the importer, one for the resource estimate. New frontend tests: 3 panel plan, 10 tree, 4 panel
component, 3 panel drop-index, and 10 vectors in the TypeScript/Rust parity file.

Each renderer check was shown able to fail by breaking what it protects — turning *atop* into
*over* in both renderers (6 tests fail), never using the group's buffer (12 fail), and applying the
base's blend mode and opacity to the base alone (1 fails) — and restoring it. The cache check failed
first, on a stale tile one bit different from a fresh render, which is why a base's cache digest now
says it is drawn through a buffer.

**Against real files** (the same 316 psd-tools fixtures, not committed): 290 imported, 26 refused,
0 failed. Imports that reported nothing approximated or left out: **36, of which 35 are close to
Photoshop's own flattened image** (15.1: 35 and 34); imports with approximations: 219, **118 still
close** (15.1: 220 and 115). No file got worse in those counts.

## What is not done

* **"Blend Clipped Layers as Group" switched off** — the layers clipped to a base blending with the
  backdrop individually — is not modelled. Photoshop's default is on; the importer reports a base
  that has it off.
* **An adjustment layer as a base.** It has no coverage of its own, so layers clipped to one paint
  as ordinary layers.
* **The panel's drag-and-drop was not exercised against a real window** for a clipping group. The
  engine's rules for a moved base and the panel's index arithmetic are tested; a manual pass with the
  packaged application is still owed.
