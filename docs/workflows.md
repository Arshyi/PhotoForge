# Workflows

## Phase 9 RAW development boundary

RAW development parameters are not ordinary encoded-space edits. The Phase 9
checkpoint exposes both a validated, serialisable `raw_development` operation
for deterministic raster previews and a `RawDevelopmentDocument` contract for
the future source-backed RAW project. Workflow replay may apply the operation
to an existing raster source; it must still fail closed rather than silently
pretend to decode a camera RAW until a vetted decoder and source-backed project
extension land. See
[raw-development.md](raw-development.md) and
[phase-9-results.md](phase-9-results.md).

Workflows are reusable, local, typed edit pipelines introduced in PhotoForge 0.6.0 and extended with immutable mask snapshots in 0.7.0. Recording a workflow copies the current operation list; it never stores source image pixels or source paths.

## 2026-09-04 continuation — 0.8.2 working source

The layer schema predates this continuation, but schema support was not proof
that the full application replayed layer steps. The current working source now
wires imported/manually edited layer workflows through the real application,
stages pixel-worker results before publishing changes, and records mixed
document/layer edits as one Undo/Redo action. App integration regressions use
mocked Tauri calls and cover mixed schema-v2 import/replay and rollback after a
worker failure. Current source-level gates are 746 Rust unit tests plus 39
IPC/integration tests, clean Rust formatting and Clippy, a successful Rust
release build, and a clean `npx tsc --noEmit`. The frontend Vite/Svelte check
and Vitest rerun is blocked in this sandbox by esbuild directory access; an
earlier escalated run passed 50 files and 814 tests before the latest
merge-safety tests were added.

These changes are not yet a rebuilt 0.8.2 release. Historical 0.8.1 packages,
browser passes, installer results, and benchmarks do not validate this source.
No native Windows GUI/DPI, elevated MSI, or production-signing acceptance is
claimed here.

## Workflows and layers

A workflow's `operations` still apply to the document pipeline, which in a
layered document runs on the finished composite. Replaying one therefore
produces the same visible result it always did.

### Schema versions

Phase 8 introduces workflow schema version **2**, which adds an optional
`layerSteps` list. Version 1 files carry none, still load and replay exactly as
before without rewriting their source files. New exports use schema 2 even
when the workflow was imported from version 1. A version 1 document that
contains layer steps is rejected as a mismatch rather than accepted silently.

### Layer steps

| Step | Effect |
| --- | --- |
| `select_layer` | Makes a layer active |
| `set_visibility` | Shows or hides a layer |
| `set_opacity` | Sets layer opacity |
| `set_blend_mode` | Sets the layer blend mode |
| `create_adjustment_layer` | Adds a parametric adjustment layer |
| `apply_to_layer` | Applies operations destructively to one pixel layer |
| `create_mask_from_selection` | Masks a layer with the current selection |
| `merge_down` | Merges a layer into the one beneath it |
| `flatten` | Flattens every visible layer |
| `export_composite` | Exports the composite and document operations to a user-chosen destination; final step only |

### Selectors, and why replay cannot target the wrong layer

A step names its layer through a deterministic selector: an exact identifier,
the active layer, the layer a previous step created, a unique name, the bottom,
or the top of the stack.

Replay fails closed. A selector that cannot be resolved, a name matching more
than one layer, a step needing a pixel layer that resolved to a group, or a
merge with nothing beneath it all abort the replay. Structural preflight checks
the sequence before worker execution. Each step then resolves against its
staged intermediate tree, so a layer created or merged by an earlier step is
handled in order. The visible document and pipeline change only after successful
completion; failed pixel work cannot publish an earlier opacity or global edit.
Created immutable buffers that are no longer referenced are released.

Merge steps additionally require a contiguous sibling range. If an omitted
backdrop could affect a selected merge through a non-Normal blend, adjustment
layer, or pass-through group, preflight rejects the step before worker or export
side effects. The frontend and Rust IPC boundary enforce the same fail-closed
rule.

`export_composite` may appear once, only as the final step. The application
asks the user for the output path; the workflow does not carry a path. Export
is an external file write and is not undone by document Undo/Redo.

`apply_to_layer` and `create_adjustment_layer` reject geometry operations,
because crop, rotation, straighten, perspective, and lens correction reshape the
canvas and belong to the document pipeline rather than to one layer.

### Batch

Ordinary image batches behave exactly as in 0.7.1. A saved `.photoforge`
project is additionally accepted as a batch input: batch renders its visible
composite, applies the project's own document pipeline, then the workflow's
operations, and exports the result. The project file is only ever read — never
rewritten — and a corrupt project fails that one item rather than the run.
Any workflow containing a layer step is deliberately rejected for batch preview
and batch execution before outputs or a batch log are written. Batch support
for project inputs does not mean layer-step batch replay is implemented.

### Planner scope

The plan schema and validators understand restricted layer steps, and the
frontend can apply them. Prospective planner output may target only the active
layer or a layer it just created; destructive merge/flatten/export proposals
and fabricated identifiers are rejected. Both current backend generators still
emit empty layer-step lists. Automatic natural-language layer-action generation
is not implemented and must not be inferred from those validation tests.

## Library and editor

The workflow library supports save, rename, duplicate, delete, favorite, search,
folders, JSON import/export, and deterministic replay. The editor can reorder,
delete, duplicate, insert through JSON, and adjust typed operation parameters.
Its separate layer-step JSON editor validates changes before saving them.
Recording copies only the current document operation list; layer actions are
not automatically recorded. Applying a mixed workflow commits document
operations and layer changes as one shared Undo/Redo action.

The built-in library is stored in the application WebView's local storage under
a versioned key and is bounded to 250 workflows. A workflow contains at most 200
document operations and 100 layer steps, and must contain at least one of
either. Local storage read failures fall back to an empty library without
affecting image editing.

## Versioned JSON

Exports use this envelope:

```json
{
  "schemaVersion": 2,
  "workflow": {
    "id": "restore-old-scan",
    "name": "Restore Old Scan",
    "description": "",
    "folder": "Restoration",
    "favorite": true,
    "operations": [
      { "type": "crop", "x": 0, "y": 0, "width": 1, "height": 1, "aspect_ratio": "original", "overlay": "rule_of_thirds" },
      { "type": "auto_white_balance", "strength": 0.5 },
      { "type": "levels", "input_black": 4, "input_white": 248, "gamma": 1.05, "output_black": 0, "output_white": 255 }
    ],
    "layerSteps": [],
    "createdAt": "2026-07-20T00:00:00.000Z",
    "updatedAt": "2026-07-20T00:00:00.000Z"
  }
}
```

The Rust import boundary caps files at 64 MiB, validates the schema version, validates every operation and parameter, and rejects unknown operation types. The larger ceiling permits bounded embedded masks while remaining below the standalone mask engine's allocation ceiling. Unknown envelope fields are ignored for forward compatibility, but unknown schema versions are rejected rather than guessed. Export checks the serialized size and uses a temporary sibling file followed by a rename.

## Masked operations

A Phase 7 workflow may wrap a mask-capable operation in a `masked` operation:

```json
{
  "type": "masked",
  "operation": { "type": "brightness", "amount": 0.18 },
  "mask": {
    "version": 1,
    "width": 2,
    "height": 2,
    "encoding": "base64_u8",
    "data": "AP+A/w",
    "checksum": "fnv1a64:d865707bf628386d"
  },
  "invert": false,
  "mask_id": "subject"
}
```

The embedded snapshot is immutable and self-contained, so replay is independent of the currently selected named mask. Import validates its dimensions, encoding, decompressed length, checksum, and wrapped operation. Nested masked operations and geometry-changing masked operations are rejected. `decontaminate_colors` is also rejected at the workflow boundary unless it is wrapped in an embedded mask; its accepted strength is `0…1`, radius is `1…32`, and omitted wire settings default safely to disabled, strength `0.5`, and radius `4`. Version 0.7.1 requires each snapshot to match its exact full-resolution pipeline stage. Preview alone creates an ephemeral bilinear copy at the corresponding bounded-preview stage; the stored workflow is not modified.

When the current edit pipeline changes geometry, 0.7.1 identifies persistent embedded masked operations by semantic operation signature and stage. Their snapshots participate in the same all-or-error geometry transaction as active and named masks. A crop, quarter-turn rotation, horizontal reflection, straighten, perspective, or lens correction inserted before a masked adjustment therefore remaps that immutable coverage to its new stage before the edit is committed. Lens coverage follows distortion in the safe `-0.16…1` range; vignetting and per-channel chromatic-aberration offsets do not move scalar coverage. If a snapshot cannot be reconciled, a transform is invalid or non-invertible, the document changes, or any result is missing, the whole geometry commit fails closed.

Current workflow exports use envelope schema 2. Phase 6 global workflows and
Phase 7 masked schema-1 workflows require no source-file migration, and loading
does not rewrite them. Unsupported future envelope versions, stale/malformed
embedded snapshots, and incompatible stage dimensions are rejected rather than
silently applying an adjustment globally.

Workflow JSON is data only. PhotoForge never evaluates scripts, loads plugins, follows paths from the workflow, or executes external programs. A mask snapshot is coverage data, not a source-image copy.
