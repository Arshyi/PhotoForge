# Phase 15.0 — one editing path for the Layers panel

Phase 15.0 is the first increment of Phase 15 (interoperability). It changes no file format. Its job is
the prerequisite named in [phase-14-results.md](phase-14-results.md#still-incomplete-or-unverified): the Layers
panel's own edits used to be made by TypeScript tree functions that were *agreed with* the Rust engine by
25 parity vectors. Two implementations agreed by test are not one implementation. Now the panel asks the
engine, and what it shows is what the engine returned.

Status: **implemented and verified as listed below; not committed, not released.** Version is still 0.14.0.

## Completed

* **The panel's edits are the engine's.** Show/hide, lock, collapse, rename, opacity, blend mode, move
  (including drag and drop), new group, new pixel layer, place image, duplicate, delete, group, ungroup,
  reset transform, pass-through, add / enable / invert / delete a layer mask, new adjustment layer and
  changing an adjustment are each sent as one transaction, as origin `user`
  ([`panelOps.ts`](../src/lib/layers/panelOps.ts)). The engine validates after every step and the
  document that returns is committed unchanged — the TypeScript validator is no longer on that path.
* **Seven new registered operations** (22 → 29): `core.layer.set_pass_through`, `mask_add`,
  `mask_set_enabled`, `mask_set_inverted`, `mask_remove`, `update_adjustment`, `place_pixels`.
  `place_pixels` names a buffer the session already holds, so it is **user-only**: the engine refuses it
  for a macro, plugin, planner or batch, a plugin manifest that names it is refused at install, and the
  macro editor does not offer it.
* **Edits queue.** A second edit made while one is with the engine is built from the first one's result and
  applied in order, not refused. Selecting a layer waits its turn too. Opening a file, undo and macros wait
  until the queue is empty.
* **Drags are drawn live and committed once.** An opacity drag or an adjustment slider shows its value at
  once as a proposal that never enters history; the engine makes the change when the slider is released
  or pauses for 400 ms.
* **History memory is preserved.** The engine's document is new all the way down; `shareUnchanged`
  gives every layer that did not change back its old identity, so the undo stack still shares unchanged
  layers (and their masks) between entries.
* **Documentation** updated: [automation.md](automation.md) (the 29 operations and the honest
  what-goes-through-the-engine table), [plugin-api.md](plugin-api.md), and the module comment in
  `operations/mod.rs`.

## Verification

All on this machine (Windows 11 Pro, Rust 1.97.1), dev profile with dependency debug info reduced to line
tables by a per-invocation `--config` (no repository file was changed to do so), in a build directory
outside the repository.

| Gate | Result |
| --- | --- |
| `cargo fmt --check` | clean |
| `cargo clippy --all-targets --all-features -- -D warnings` | clean |
| `cargo test --all-targets --all-features --no-fail-fast` | **1,460 passed**, 0 failed, 40 test binaries. The same command on an export of `main` (bdbfe7f) with the same settings: 1,448 passed, 39 binaries. The 12 added are this phase's new Rust tests; none was removed |
| No GPU / no inference / no plugins (`--no-default-features …`) | clippy clean; **1,453 / 1,452 / 1,421 passed**, 0 failed |
| `npx tsc --noEmit`, `npm run check` | clean; 507 files, 0 errors, 0 warnings |
| `npm test` | 84 files, **1,257 passed**, 0 failed (Phase 14: 81 files, 1,205; 52 added, none removed) |
| `npm run build` | built |

New tests: `operations_engine.rs` (+10, covering the seven operations), `operations_panel.rs` (2),
`panelOps.test.ts` (36), `sharing.test.ts` (6), `App.panelengine.test.ts` (10).

**Across the language boundary.** `panelOps.test.ts` writes every plan the panel can send (29, with a
document each is valid on) to `src-tauri/tests/fixtures/panel_plans.json`; `operations_panel.rs` runs each
through the real engine as the person at the keyboard, requires it to be accepted and to change the
document, and checks what a dozen of them must do. A mis-spelt parameter fails there with the engine's own
message.

**Each gate was shown able to fail**, by breaking the thing it protects, watching the intended test fail, and
restoring the file byte-for-byte:

| Broken | Caught by |
| --- | --- |
| commit a locally computed document instead of the engine's | 5 `App.panelengine` tests |
| remove the queue | "queues a quick second edit behind the first" |
| commit a drag on every step | the two opacity-drag tests |
| pass-through leaves a blend mode behind | `pass_through_is_set_and_returns_the_blend_mode_to_normal` |
| drop the user-only rule | `only_a_person_may_place_pixels` |
| drop the buffer-size check | `placing_pixels_checks_the_buffer_exists_and_is_the_size_claimed` |
| send `MULTIPLY` for a blend mode | `every_plan_the_panel_sends_is_accepted…` and `the_plans_do_what…` |

**In the real application window.** A debug build of the app (WebView2 and the real Rust backend, driven
over the browser debugging port, with the frontend served from the dev URL as `tauri dev` does) passed
24 of 24 checks: every action above reached the engine as one `user` transaction; a three-step opacity drag
sent nothing until release and then exactly one transaction; 19 edits were undone in 19 steps and redone in
19; a burst of 8 clicks produced 4 transactions in order with no refusal. Observed from outside through
the DevTools network log, because WebView2 does not let a page wrap its own `invoke`. **This is a debug
build, not the packaged release**; the image was opened by delivering the window's drop event rather than
through the native file dialog.

## Measured cost

An edit is now a round trip carrying the whole layer tree (not pixels). One visibility toggle, debug build,
medians of 7:

| Layers | Request | Over the IPC | Click to screen |
| ---: | ---: | ---: | ---: |
| 50 | 23 KiB | 5 ms | 47 ms |
| 100 | 46 KiB | 9 ms | 42 ms |
| 200 | 92 KiB | 132 ms | 177 ms |
| 400 | 184 KiB | 406 ms | 482 ms |

("Click to screen" is polled every 40 ms, so values near 40 ms mean "within one poll".) The engine alone,
timed separately in the same profile, takes **1.5 / 3.0 / 5.9 / 11.9 ms** for the same four sizes, so almost
all of the time at 200 and 400 layers is **outside the engine** — serialisation, the WebView2 IPC, or the
page doing other work — and **I did not isolate which**. It is a debug build; a release build was not
measured. Before the old path, these edits were instantaneous; at a few hundred layers they now are not.

## Behaviour that changed

* Panel edits are an asynchronous round trip and queue in order; opening a file, undo or a macro, tried while
  an edit is in flight, shows the existing "Wait for the current file or layer operation to finish."
* A drag commits when released or after a 400 ms pause, not on every tick.
* Refusals are the engine's text (for example "Step 1 (core.layer.move) failed: …") rather than the old
  local sentences.
* The Undo labels are unchanged. `reset transform` and pass-through are still not written into a recorded
  macro, as before.

## Still incomplete

* **Not routed through the engine** (documented in automation.md): the panel's *From selection*,
  *To selection*, *Apply mask*, *Rasterize*, *Convert to smart object*, flip, the transform box, text /
  shape / smart-object editing, RAW development and painting a mask. `core.layer.mask_from_selection`
  exists as an operation, but the panel button does not use it. Its lock policy is `Always` while the
  button has always worked on a locked layer, so routing it is a decision, not a refactor.
* **The TypeScript tree functions still exist** and still serve the text / shape / transform paths, the
  smart-contents editor, workflow steps, and the dev-only browser harness (which mounts the real panel
  with its own callbacks and does not exercise the engine). The 25 parity vectors still hold them to the
  engine.
* **Latency at several hundred layers** (above) is unexplained and unmeasured on a release build.
* Repeatedly duplicating one layer grows its name ("… copy copy copy") until the 120-character limit
  refuses it. That limit and the refusal are older than this phase; it was only noticed by it.
* Packaged-release validation, display scaling at 125 / 150 / 200 %, a screen reader, and the full
  native GUI matrix were **not run**. Nothing is signed.

## Deferred

PSD / PSB import (15.1) and everything after it in the
Phase 15 brief (kept outside the repository, pending its owner's review): its fidelity report, the bounded readers,
fuzzing, TIFF / EXR / SVG / metadata, and the clipping-mask decision.
