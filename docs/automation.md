# Automation

There is **one way a layer document changes**: a typed operation from the registry,
sent to the transaction engine, which validates it, admits its resources, runs it as an
all-or-nothing transaction, checks the document's invariants and hands back one
undoable result that the renderer and cache then use. The Layers panel, a plugin's
command, a macro, a workflow's layer steps and the guided-edit planner's layer steps
all use it, and there is no second layer-editing engine for them or for a future
command line.

There is also **one evaluator for the pixel math of an edit** (`EditOperation`), which
the editor's own edit stack, adjustment layers, plugin filters, the tiled renderer and
batch processing all call. Batch processing has no editing path of its own: it renders
each file through that evaluator and never touches a layer tree.

What is *not* unified is stated plainly in [phase-14-results.md](phase-14-results.md):
the single-image edit stack (the sliders for a photo that is not a layered document) is
still changed by the interface's own code, not through the registry, and the three
history stacks (edits, selections, layers) are still interleaved by an event list.

This page describes that path and the structured automation built on it. There is
**no scripting**: no Python, JavaScript, shell, expression language, loop or variable
anywhere in it.

## The operation registry

`list_operations` returns every operation that may edit a document, as the backend
publishes them. There are 22:

| Category | Operations |
| --- | --- |
| Layer (15) | `core.layer.select`, `set_visible`, `set_locked`, `set_opacity`, `set_blend_mode`, `rename`, `set_collapsed`, `move`, `delete`, `duplicate`, `group`, `ungroup`, `reset_transform`, `add_group`, `mask_from_selection` |
| Adjustment (1) | `core.layer.add_adjustment` |
| Pixels (3) | `core.layer.add_pixel`, `apply_edit`, `merge_down` |
| Document (1) | `core.document.flatten` |
| Plugin (2) | `core.plugin.apply_filter`, `core.plugin.add_adjustment` |

Each has an id in the `core.` namespace, a version, a title, a summary, **typed
parameters**, whether it changes pixels, how it treats locked layers, whether a planner
may use it, and whether it needs a selection. The registry is pinned by a fixture
(`src-tauri/tests/fixtures/operation_registry.json`) that the interface's own checks
read, so what the editor offers cannot drift from what the backend accepts.

A parameter is one of a closed set of kinds — a layer selector, a list of selectors,
text of a bounded length, a boolean, a number or integer in a range, a blend mode, an
edit or list of edits — and a call with a missing, surplus or wrongly typed parameter
is refused before anything runs.

Layers are named by a **selector**: by id, by name, the selected layer, the top or
bottom layer, or "the layer the last step created". Selectors resolve against the
document *as it is when the step runs*, after the steps before it.

## The transaction engine

A request is a document, a label, a list of calls, an origin, and optionally an
expected revision and a selection mask. The engine:

1. **Plans.** It works on a private copy and does a **dry run** of every step first:
   every selector is resolved, every lock, kind and invariant checked, the whole
   request admitted against the memory budget. Nothing real has been touched.
2. **Runs.** It runs the steps for real against the same private copy, with a journal
   of every pixel buffer it creates or changes.
3. **Validates.** The document validator — the same one that guards loading a
   project — runs after every step.
4. **Commits or rolls back.** If any step fails, every buffer in the journal is
   discarded and the document you started with is untouched: no layer, no pixel buffer
   and no history entry is left behind. If all succeed, the result is one new document.

Committing that result is **one entry in Undo**, named for the request. The engine
reports, per step, whether it ran or was skipped, which layers it created and which
pixel buffers it registered.

**Origin** — `user`, `automation`, `planner`, `plugin`, `batch` — decides what is
*allowed*, never how it is *done*. A planner may only use operations marked planner-safe
and may name layers only relative to the selection; a plugin must carry its own id
and only operations its manifest named; an operation that destroys or protects work
respects locked layers for every origin but a person. `expected_revision` makes a
request fail if the document moved on since it was planned.

The interface's layer-tree functions are TypeScript, for instant feedback; a set of
**parity vectors** (`src-tauri/tests/fixtures/operations_parity.json`) pins them to the
engine, so the two cannot quietly disagree.

## Conditions

A step may carry one **condition**, and is skipped when it does not hold. That is the
only decision automation can make. A condition is one of:

* a layer exists (named by a selector);
* the document has at least *n* layers (1–512);
* the selected layer is of a kind (`pixel`, `group`, `adjustment`, `shape`, `text`,
  `smart_object`);
* the document has a precision (`linear_srgb_f32` or `legacy_srgb8`);
* **not** of one of those, nested at most two deep.

There is no else, no loop, no variable, no arithmetic, no expression. And it is
asked **of the document as it is when the step is reached**, after the earlier steps
have run — not once at the start — so "if there is a layer called Sky, dim it" cannot
dim a layer an earlier step deleted. The dry run and the real run ask the same
question of the same working copy and so skip the same steps.

`plan_transaction` runs the dry run alone and returns the per-step report — which
steps would run and which would be skipped, or the exact reason the request cannot
work — **without registering a single pixel buffer**. Its refusal and the refusal of
running are the same text.

## Macros

A **macro** is a named list of steps. Each step is a registered operation, its
parameters, an on/off switch, an optional condition and an optional note. It is data:
JSON a person can read, and nothing in it is code.

**Automation…** in the command palette opens the editor, where a macro can be made,
renamed, duplicated, deleted (asking twice), exported and imported, and where each step
can be turned off, moved, copied, removed and edited with controls drawn from the
registry's own description of the operation. The editor checks the macro as it is
edited, against the same registry, and names the step and the problem
("Step 3 (Set layer opacity): opacity must be between 0 and 1"). The backend checks
again when it runs, and is the authority.

* **Check** plans the macro and shows beside each step whether it would run or be
  skipped. It changes nothing. Edit the macro and the answer is withdrawn rather than
  left beside steps it no longer describes.
* **Run** sends the enabled steps as **one transaction** with origin `automation`: it
  is atomic, validated after every step, and **one Undo reverses all of it**. It says how
  many steps ran and how many were skipped, and if every step was skipped it says
  nothing was changed and adds no history entry.
* **Plugins a macro uses are listed**, each *ready*, *turned off*, *not installed*,
  or unavailable with the reason; **Run is unavailable until each can run**, and says so.
* Every saved macro with something to run is also a command in the palette,
  *Run macro: …*, which runs it exactly as Run does.

### Recording

*Record a macro…* closes the editor, shows a banner (*Recording a macro* with a count,
**Stop recording** and **Discard**), and writes down what you do in the Layers panel as
registered operations: show or hide, lock, collapse, opacity, blend mode, rename, move,
group, ungroup, duplicate, delete, merge down, flatten, new group, new pixel layer, new
adjustment layer, and applying an adjustment directly to a layer.

Recording is **deliberately narrow** and says what it left out. An action with no
registered operation — editing text, placing an image, developing RAW, painting, Undo
and Redo — is not guessed at: it is counted in the banner and listed above the new
macro when recording stops ("Editing text cannot be recorded, so it is not in this
macro."). A continuous gesture such as an opacity drag becomes one step holding the last
value, not one per frame.

A layer is named in the way most likely to mean the same thing later: by **name**, if no
other layer shares it, which replays on any document with such a layer; otherwise by
**identifier**, which replays on this document only, and the recording says so once.

### Files

*Export…* writes a macro as a `photoforge-macro` JSON document (schema version 1) and
*Import…* reads one **as a new macro** — it never overwrites the one it came from, and
nothing in it runs. File access goes through two backend commands that read and write
**only** macro documents, no larger than 1,000,000 bytes, to an absolute `.json` path in
a folder that exists, replacing an existing file atomically. They are not a general way
to read or write files. A file with an unknown field in a step, an unknown condition
kind, or a step count over 100 is refused with "the macro in the file is malformed", not
repaired. Imported steps are checked like any others when they run.

## What goes through the engine today, and what does not

Stated plainly, because "one path" is a claim that should be checkable.

| Change | Path |
| --- | --- |
| Macros, workflow and planner **layer steps**, plugin filters, plugin commands | The transaction engine, at commit time |
| Merge down, flatten | The transaction engine (`core.layer.merge_down`, `core.document.flatten`) |
| Show/hide, lock, collapse, select, opacity, blend mode, rename, move, new group, duplicate, delete, group, ungroup, reset transform, new adjustment layer | The interface's own tree functions, so a drag of the opacity slider is instant, **pinned to the engine** by 25 shared parity vectors over these 15 operations (`tests/fixtures/operations_parity.json`, run against both implementations), and checked by the same document validator the engine uses on save and load. Each has a registered operation that does the same thing, which is what a macro runs |
| New pixel layer | The backend registers the buffer (`create_layer_pixels`) and the interface inserts the layer. `core.layer.add_pixel` does both inside the engine. **Not covered by a parity vector** |
| Text, shape and smart-object editing, RAW development, transforms, painting a mask | The interface and their own commands. **No registered operation exists for them**, so a macro cannot contain them and the recorder says so |
| The single-image edit stack (sliders on a photo that is not a layered document) | The interface; unchanged by this phase |

The middle row is the one that matters. It is two implementations of the same operations,
agreed by test and not by construction. If they disagree on a case the vectors do not
cover, a macro would do something different from the panel. Moving those actions onto the
engine (one round trip per commit, with a local preview for gestures) is the way to make
the claim true by construction; it was not done in this phase.

## Workflows, the planner and batch

The older **workflows** (the Flows tab in the professional workspace: a list of global
edits plus layer steps) are kept. Their layer steps are converted to registered operations
and run through the same engine, as one transaction, so they cannot behave differently
from the same steps in a macro. Their saved files are unchanged.

The guided-edit **planner** (rule-based, and the optional local Ollama one) produces a
plan of typed edits and layer steps. Its **layer steps** run through the transaction
engine with origin `planner`: planner-safe operations only, layers named relatively to the
selection, never by identifier or name. Its global edits go to the edit stack like any
other edit. A plan is shown, checked and applied; it has no path of its own to a layer
document.

**Batch processing** applies a workflow's global edits to a folder of files through the
same evaluator, writes the results, and **never edits a layer tree**. A workflow that has
layer steps is refused outright ("layer-aware workflow steps are not supported by batch
processing") rather than having them ignored. A plugin filter works in a batch: it is the
same function found the same way. An 8-bit source (a JPEG, a plain PNG) is **promoted to
linear float once, deliberately**, for a workflow that contains a plugin filter, because the
legacy 8-bit renderer will not run one; a workflow without one keeps the precision it always
had. If the plugin is missing, off, damaged or a different version, **each file fails with
that reason and nothing is written for it** — a file is never passed through unfiltered.
(`tests/plugin_pipeline.rs`, two batch tests.)

## What this does not do

* No macro **language**. Conditions are five questions and `not`.
* No macro can **loop**, call another macro, or read a file's contents.
* No macro can run **plugin code except through a filter** the plugin declared, with the
  limits in [plugin-api.md](plugin-api.md).
* Recording does not capture **everything** — it says what it missed.
* A macro made on one document **may not run on another**: its steps may name layers the
  other has not. The engine refuses such a run as a whole, with the step and the reason,
  and changes nothing.
* There is **no command-line interface** yet. The registry and engine are built so one
  could be added without a second editing path; none is claimed.
