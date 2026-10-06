# Phase 14 architecture audit

This audit was written before any Phase 14 implementation, from the live
repository rather than from the handoff. Where the two disagree, the
repository is recorded.

## 1. Baseline, as found

| Claim in the Phase 14 handoff | What the repository showed |
| --- | --- |
| `main` is `9a9afc4` | `main` is `bcef259`, two commits later: the window-close fix (`ae7c38a`) and the artifact rebuild (`bcef259`). Local, `origin/main` and `git ls-remote` agree. Working tree clean, `git diff --check` clean. |
| Rust: 1,182 tests | **1,182 passed, 0 failed** (`cargo test --offline --all-targets --all-features`). |
| Frontend: 899 tests / 59 files | **906 tests / 60 files.** The extra 7 tests / 1 file are `close.test.ts`, added with the close fix after the handoff. 901 passed; the 5 failures are the known timeout-marginal `App.lifecycle` cases, run here while cargo was loading the machine. |
| `npm run check` 443 files | 445 files, 0 errors, 0 warnings (the close fix added two files). |
| `photoforge.exe` hash begins `c7824463c251…` | Stale. The close fix changed the frontend and the binary was rebuilt; the shipped hash is `dd16370f9a8f…`. The handoff's size figures (10,171,365 / 45,270,528) are likewise the pre-fix bytes. |

## 2. Where the systems overlap

### D1. Four operation vocabularies, and a fifth for UI commands

| Representation | Where | Variants | Used by |
| --- | --- | --- | --- |
| `EditOperation` | `domain/models.rs` | 34 | The document pipeline and adjustment layers |
| `LayerWorkflowStep` | `layers/workflow.rs` | 10 | Layer workflows — validated in Rust, **executed in TypeScript** |
| `EditPlan` | `domain/planner.rs` | n/a | Rule planner and the optional Ollama planner |
| `Workflow` (global operation list) | `domain/`, `application/batch.rs` | n/a | Batch processing, via `pipeline_typed` |
| `LayerPanelAction` / `ShortcutIntent` / `handleLayerAction` | `src/lib`, `App.svelte` | string switch | Every user-initiated layer command |

None of them share identifiers. `LayerWorkflowStep::kind()` returns bare
snake-case strings (`"select_layer"`, `"merge_down"`) that are not namespaced,
not versioned and not owned, so a third party could not add one without
colliding. UI commands are dispatched through an untyped string switch.

This is the duplication Phase 14 must remove: a plugin, a macro, the planner and
a future CLI would each otherwise grow a sixth.

### D2. Document mutation lives in TypeScript; the authoritative validator is not on the commit path

The layer document is mutated by pure functions in `src/lib/layers/tree.ts` and
committed by `commitLayers` in `App.svelte`, which is called from **41 sites**.
Before committing it runs `validateDocument`, a **TypeScript** validator. The
Rust `LayerDocument::validate` — which also checks the smart-source graph,
recursion, mask dimensions and aggregate limits — is exposed as
`validate_layer_document`, but `commitLayers` does not call it; only
`SmartContentsEditor` does.

This is narrower than "Rust is never consulted", and the distinction matters.
Every Rust command that receives a document re-validates it on arrival (15 calls
in `commands/layers.rs`, 9 in `commands/smart.rs`, 2 in `commands/text.rs`), so
Rust never *acts* on an unvalidated document. The defect is timing: a document
the TypeScript validator accepts and Rust would reject is committed into
history, marks the project dirty and is shown to the user, and fails only at its
next render, save or export — after the undo entry that caused it is buried.

Consequences:

* Two validators of one invariant set, free to drift.
* Multi-step atomicity is by convention. Because documents are persistent
  values, a failed multi-step edit is discarded simply by not committing, which
  is correct — but nothing enforces it, and nothing states which operations are
  safe to compose.
* Operations that create pixel buffers (`merge`, `apply to layer`, `mask from
  selection`, `rasterize`) have a side effect in the Rust pixel store that a
  rollback does not undo. Orphans are reclaimed by a later
  `releaseUnreferencedPixels` sweep rather than by the transaction that made
  them.

### D3. Three history stacks interleaved by an event list

`EditHistory`, `SelectionHistory` and `LayerHistory` are separate classes, and a
`historyEvents` / `redoEvents` list in `App.svelte` records which stack to pop.
`LayerHistory` stores whole document trees, which is affordable because the tree
carries no pixels. A grouped transaction therefore maps to exactly one
`LayerHistory.commit`, which is the property Stage C and AC rely on.

### D4. One compile-time constant is the whole resource policy

`resources::MAX_WORKING_IMAGE_BYTES` is `1_073_741_824`. From it:

* `MAX_WORKING_PIXELS = bytes / 16` = **67,108,864 pixels**, a hard ceiling
  that does not depend on the machine.
* It is re-exported as the ceiling in six places: `color.rs`
  (`MAX_FLOAT_PIXELS`), `image_io.rs` (`MAX_PIXELS`, `MAX_DECODED_BYTES`),
  `layers/model.rs` (`MAX_CANVAS_PIXELS`), `layers/project.rs`
  (`MAX_ENTRY_BYTES`), `raw.rs` (`RAW_MAX_PIXELS`).
* `MAX_DIMENSION = 20_000` is **declared twice** — in `resources.rs` and again
  in `image_io.rs`.
* `layers/store.rs` has its own independent `MAX_STORE_BYTES` of 1 GiB, and
  `MAX_JOB_BYTES` is a further 4 GiB.

Nothing consults physical or available memory. A 128 GB machine and an 8 GB
machine apply identical ceilings. Several of these also conflate two different
questions: whether a file is **structurally valid** (a property of the format,
which must hold regardless of machine) and whether it is **admissible under the
current budget** (a property of the machine and what is already open).

### D5. Two plugin concepts

`domain/plugins.rs` defines a `PluginManifest` v1 with types `Planner` and
`RestorationEngine`, scanned and validated by `scan_plugins` and
`validate_plugin_manifest`. It is metadata discovery only: nothing is ever
loaded or executed, and there is no runtime. The Phase 14 `.photoforge-plugin`
format is a different thing with a different manifest, so the two must be
reconciled rather than left as lookalikes.

### D6. Presets and workflows

There is a single flat `presets: Preset[]` list of adjustment presets in
`operations.ts`. Workflows are stored in `localStorage` and as files through
`workflow_io.rs`. Shortcut bindings already have conflict detection and
persistence (`workspace.ts`). There is **no command palette** and no registry
that commands are drawn from.

## 3. Image admission as found

`load_image` reads dimensions from the header first (`image::image_dimensions`),
validates them, estimates cost, and only then decodes. That ordering is right
and is kept: a hostile header is refused before any pixel allocation.

The decode itself is full-frame (`DynamicImage::from_decoder`) for PNG, JPEG and
WebP. The 16-byte-per-pixel float working copy is what makes the 67.1 MP ceiling
bind; the transient 8-bit decode costs 3–4 bytes per pixel. So an image can be
decodable but not admissible as a conventional float document, and the code does
not currently distinguish the two: both surface as `ImageTooLarge`.

The DNG decoder (`raw/dng.rs`) takes the whole file as `&[u8]` and decodes every
strip or tile, but its segment table is explicit, so decoding only the segments
that intersect a window is structurally possible. Per-format ROI capability is
recorded in `docs/oversized-images.md` as it is established, not assumed here.

## 4. What is sound and is reused

* The tiled renderer is region-addressable (Phase 11) and `layers::linear`
  remains the correctness oracle. Rendering a *canvas* region exists; decoding a
  *source* region does not.
* The tile cache keys per layer with influence regions, and a plugin layer can
  join that scheme.
* `resources::acquire_job` already serialises full-frame workers, including
  batch, with cancellable admission.
* The project container is versioned, checksummed and bounded.
* `Result`/`AppError` is used throughout and carries structured limits.

## 5. Phase 14 consequences

1. The operation registry and transaction boundary belong **where mutation
   happens and where the authoritative validator lives** — in Rust, with the
   frontend calling into it — otherwise batch, plugins and a future CLI could not
   share it.
2. The commit path must run the Rust validator. Doing so closes D2 without
   rewriting the 41 call sites; those migrate to the registry incrementally and
   the audit records which have.
3. Resource limits become a runtime policy fed by the machine, with the two
   questions (valid / admissible) separated.
4. Plugin filters need a renderer node of their own, with locality declared, so
   that tiling never treats a global algorithm as local.
