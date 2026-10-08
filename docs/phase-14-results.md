# Phase 14 results — PhotoForge 0.14.0

Extensibility, resource management, oversized-image admission, safe automation and a
plugin architecture. Everything below was produced or measured in this checkout; where a
thing was not done, that is said in the section that would otherwise have claimed it.
Written from the repository, not from the handoff: where they disagreed, the repository
is recorded (see [phase-14-architecture-audit.md](phase-14-architecture-audit.md)).

Starting commit `bcef259` (0.13.0, rebuilt artifacts). Ending commit: see
[Git](#git).

## What was built

| Area | Where | Document |
| --- | --- | --- |
| Operation registry (22 operations) and transaction engine | `src-tauri/src/operations` | [automation.md](automation.md) |
| Conditions on steps, and planning | `operations/condition.rs`, `plan_transaction` | [automation.md](automation.md) |
| Macros: editor, recorder, files, palette commands | `src/lib/automation`, `AutomationDialog.svelte`, `infrastructure/macro_io.rs` | [automation.md](automation.md) |
| Resource manager and memory budget (Settings, Memory) | `src-tauri/src/resources`, `ResourceSettings.svelte` | [resource-management.md](resource-management.md) |
| Oversized-image admission, region and reduced-copy opening, source-backed documents | `src-tauri/src/source`, `OversizedSourceDialog.svelte`, `RegionSelector.svelte` | [oversized-images.md](oversized-images.md) |
| Plugin runtime (Wasmtime), package reader, store, manager | `src-tauri/src/plugins`, `PluginManager.svelte` | [plugins.md](plugins.md), [plugin-api.md](plugin-api.md), [plugin-package.md](plugin-package.md), [plugin-security.md](plugin-security.md), [plugin-runtime-decision.md](plugin-runtime-decision.md) |
| Plugin filters as render nodes (tiling, masks, cache) | `EditOperation::PluginFilter`, `layers/tiles.rs`, `image_processing/high_precision.rs` | [plugins.md](plugins.md) |
| Command palette, plugin commands, panels, tools | `src/lib/commands`, `CommandPalette.svelte`, `PluginsPanel.svelte` | [plugins.md](plugins.md) |
| Dependencies | `src-tauri/Cargo.toml` | [dependency-decisions-phase-14.md](dependency-decisions-phase-14.md) |

Six example plugins are in `plugins/examples/` (solarize, channel swap, border, box blur,
shapes, inspector) and 25 deliberately hostile modules in `plugins/adversarial/`. Neither is
shipped in any installer.

## Audit findings, and what became of them

The audit's six findings, and the state of each at the end:

| Finding | State |
| --- | --- |
| **D1.** Four operation vocabularies plus an untyped UI switch | **Reduced, not removed.** Layer-document changes have one vocabulary (the registry); workflow layer steps convert to it; `EditOperation` stays the one evaluator for pixel math. `EditPlan`, `Workflow` and `LayerPanelAction` still exist as types. |
| **D2.** Document mutation in TypeScript; the authoritative validator not on the commit path | **Partly.** Transactions mutate in Rust and run the authoritative validator after every step. The Layers panel's simple actions still apply through TypeScript tree functions, pinned to the engine by 25 shared parity vectors, and validated by the TypeScript validator at commit. See [automation.md](automation.md#what-goes-through-the-engine-today-and-what-does-not). |
| **D3.** Three history stacks interleaved by an event list | **Unchanged.** A transaction is one undo entry on the layer stack. |
| **D4.** One compile-time constant is the whole resource policy | **Resolved.** [resource-management.md](resource-management.md). |
| **D5.** Two plugin concepts | **Reconciled by documentation and naming.** The component manifest (discovered, never run) and the plugin package (sandboxed, installed) are different formats told apart by their first field; the older documents now say so. |
| **D6.** Presets and workflows | **Macros added; workflows kept.** Workflow layer steps run through the same engine. Named parameter presets for plugin filters were **not** built; the last-used values are remembered per filter. |

## Verification

All on this machine, Windows 11 Pro 10.0.26200, Intel Core i7-12850HX (16 cores / 24
threads), 128 GB RAM, NVIDIA RTX A5500 Laptop GPU, Rust 1.97.1.

| Gate | Result |
| --- | --- |
| `cargo fmt --check` | clean |
| `cargo clippy --offline --all-targets --all-features -- -D warnings` | clean |
| `cargo test --offline --all-targets --all-features --no-fail-fast` | **1,445 passed**, 0 failed, 39 test binaries |
| `npx tsc --noEmit` | clean |
| `npm run check` (svelte-check) | 502 files, 0 errors, 0 warnings |
| `npm test` | 81 files, **1,205 tests passed**, 0 failed |
| `npm run build` | built |
| No GPU (`--no-default-features --features custom-protocol,inference,plugins`) | clippy clean; **1,441 passed**, 0 failed |
| No inference (`…custom-protocol,gpu,plugins`) | clippy clean; **1,440 passed**, 0 failed |
| No plugins (`…custom-protocol,gpu,inference`) | clippy clean; **1,409 passed**, 0 failed (the plugin suites are compiled out with the engine) |

For the reduced builds, `cargo tree` confirms the engine is really absent: the no-plugins
tree contains no `wasmtime`, `cranelift` or `wasmparser`, the no-GPU tree no `wgpu`, the
no-inference tree no `tract`. Each was linted and tested; **none was packaged**.

Baselines at the start were 1,182 Rust tests and 906 frontend tests in 60 files. Nothing was
deleted to reach the new totals except three TypeScript replay-executor tests for code that
moved to Rust, replaced by nine frontend tests and the Rust engine suite.

Gates were run on a quiet machine. An earlier frontend run, made while cargo was compiling
in the background, failed 15 of the heaviest App tests on **timeouts**; that was a
property of the load, not of the product, and was not counted as either a pass or a fail
— the run was repeated on a quiet machine and the result above is that one.

Each safety gate was shown to be able to fail, by breaking the thing it protects and
watching it fail, then restoring it: the macro checks, the recorder, the dialogs' run
blocking, the focus wiring, the macro-file guard, the batch promotion, the package reader's
overlap check, and the planner's pricing (below).

### Evidence for the ten success cases

| # | Case | Evidence |
| --- | --- | --- |
| 1 | A normal image opens normally | `resources::admission::tests::a_normal_image_is_admitted_without_fuss`; the whole-open rows of `tests/estimate_accuracy.rs`; every App open test |
| 2 | An oversized image opens by preview and region, saves, and reopens keeping its source view | `source::open::tests::an_oversized_png_is_refused_whole_and_opens_by_region`; `OversizedSourceDialog.test.ts` and `RegionSelector.test.ts`; `layers::project::tests::a_source_view_survives_a_project_round_trip_and_a_missing_source_changes_nothing`; for a camera RAW, `tests/raw_flow.rs::a_region_backed_project_survives_a_save_and_reload` |
| 3 | A region is the same pixels as the same part of the whole | PNG, JPEG, WebP: the `…identical_to_the_crop` tests in `src/source`; DNG: `tests/raw_region.rs` (19 rectangles × strips / 8×8 / 16×16 tiles × 3 white balances, bit for bit) |
| 4 | Resource safety against hostile dimensions | `resources::admission` tests (zero, implausible, impossible-expansion headers → `Unsafe`); `source::probe`; `tests/resource_policy.rs` |
| 5 | A basic plugin works end to end, with Undo and save/reopen | `tests/plugin_ipc.rs::a_plugin_command_draws_on_a_document_as_one_undoable_step_and_survives_a_save`; `src/App.plugins.test.ts` |
| 6 | A hostile plugin is contained | `tests/plugin_runtime.rs` against the 25 fixtures in `plugins/adversarial/` (21 tests); `tests/plugin_package.rs` (12) |
| 7 | A missing plugin is visible and never silently replaced | `tests/plugin_pipeline.rs` (`every_way_a_plugin_can_be_unavailable…`, `a_document_with_a_missing_plugin_says_which…`), the batch test, `App.plugins.test.ts` |
| 8 | Automation uses the same registry and undoes correctly | `tests/operations_engine.rs` (24), `operations_conditions.rs` (7), `src/App.automation.test.ts` (14) |
| 9 | A large machine gets a larger budget | `the_same_source_is_judged_by_the_machine_it_is_on`; the admission matrix below |
| 10 | A small machine is not over-allocated | the same; `tests/resource_policy.rs` with fixed probes; the matrix below. **Simulated, not run on small hardware** |

## Architecture as delivered

**Operation registry and transaction engine.** 22 registered operations with typed
parameters, whether they change pixels, their lock policy, whether a planner may use them.
A request becomes a transaction on a private copy: a dry run first, then a real run with a
journal of every pixel buffer; the authoritative document validator runs after each step; a
failure discards the journal and leaves nothing behind. Origin (`user`, `automation`,
`planner`, `plugin`, `batch`) decides what is allowed, not how it is done. A condition on a
step is asked of the document as it is when the step is reached.

**Resource manager.** One budget from the machine (installed and free memory, less a
reserve of 2 GiB or 15%, 85% of the smaller figure, within 512 MiB and 90% of installed),
or a figure the person sets in Settings → Memory, clamped to what the machine can
support, with a 4 GiB fallback when it cannot be measured. Every ceiling is a fraction of it.
Limits are a snapshot refreshed at document open and on an explicit change; a lower budget
applies from the next document. Structural limits (2³¹ pixels, 131,072 per edge) never
move. Video memory is not measured, and the page says so.

**Admission and oversized images.** A planner decides *whole*, *whole with a warning*,
*region*, *reduced copy*, *insufficient* (momentary or budget) or *unsafe*, from the header
and what the decoder can honestly do. Region decoding by format: PNG true row streaming;
interlaced PNG, JPEG and WebP whole-frame then crop (named as such and priced as such); DNG
true segment decoding, bit-identical to a crop of a whole development. Reduced copies:
streamed for PNG, DCT-scaled for JPEG, whole-frame for WebP, none for DNG. The document
remembers its source by SHA-256 and view; a missing source changes nothing. **Out-of-core is
not offered** and every report says why.

**Plugin runtime.** Wasmtime 49.0.2, core modules only, **no WASI**. One import
(`photoforge.log`). A fresh instance for every call; memory capped by a `ResourceLimiter`
(manifest ask, ceiling 2 GiB, at most a quarter of the budget); work capped by fuel
(10 M + 20,000 per pixel); wall-clock capped by an epoch (20 s + 2.5 s per megapixel, 15 min
ceiling, 10 ms resolution, which is also how cancel works); NaN canonicalised, relaxed SIMD
and threads absent. Output must be the right size, finite, alpha in range, or it is refused.

**Capabilities.** Five exist: `filter.pixels`, `document.read`, `document.operations`,
`ui.panel`, `ui.tool`. Default authority is none. Anything in `filesystem`, `network`,
`process`, `shell`, `registry`, `env`, `clipboard`, `device`, `native`, `net`, `fs` does not
exist and a manifest that asks for it is refused. Granted at install, checked on every use.

**Package.** A ZIP read by a purpose-built reader as hostile input: closed file list,
stored and deflate only, overlap, bomb and CRC checks, 48 MiB limit. Identity is a content
hash; updates install beside the old version; installs are all-or-nothing after a
self-test that runs each filter twice and whole-versus-tiled. **Not signed**, and the
installer says so.

**Filters and tiling.** A filter declares `pointwise`, `local` (radius ≤ 256) or `global`;
the renderer gives a `local` filter its margin, runs a `pointwise` one in tiles, and runs a
`global` one as one call, refusing before any pixel is copied if it will not fit. The
declaration is checked by the install self-test; a self-test samples and cannot prove.

**UI.** Plugin manager, filter dialog (bake into a layer or add a live adjustment layer),
command dialog, panels over a closed set of document facts, tools as menu entries running
commands, command palette (`core.*`, `plugin:<id>:<command>`, `macro:<id>`), macro editor,
memory page.

**Persistence and missing plugins.** A plugin filter in a project records plugin, exact
version, content hash, locality and parameters; the plugin itself is not stored. A project
opens and saves with a missing plugin's reference intact; the layer is hidden in the preview
and badged; export, merge, flatten and rasterize refuse, naming the plugin. A different
version is never substituted. A legacy 8-bit document refuses a plugin filter.

**Cache.** The operation, including the plugin's hash and parameters, is part of the render
cache key; a test shows one version or setting cannot serve another.

**Batch and planner.** Batch renders through the same evaluator, never edits a layer tree,
rejects layer steps outright, and runs plugin filters (promoting an 8-bit source to linear
float once, deliberately); a missing plugin fails each file with its reason and writes
nothing. The planner's layer steps run through the engine as origin `planner`.

## Measurements

Release build, this machine, one scenario per process. Run-to-run variance at 24 MP and
above is large on this machine — even the native filter swung between 755 and 1,190 ms at
45 MP — so minimum and median are both given and the **minimum is the better estimate of
capability**.

### A plugin filter against the native one

Pointwise (one arithmetic step per channel) — the difference is the sandbox and the copy in
and out. Seven runs each (ms, min / median):

| Size | Native contrast | Plugin solarize | Plugin ÷ native (min) |
| --- | --- | --- | --- |
| 12 MP | 247 / 261 | 512 / 528 | 2.1× |
| 24 MP | 407 / 415 | 999 / 1,722 | 2.5× |
| 45 MP | 755 / 1,190 | 1,899 / 3,585 | 2.5× |

Neighbourhood — **different algorithms**, so this shows what each costs, not that the plugin
does the same work. Three runs each (ms, min / median). The plugin is a hand-written
WebAssembly text example that sums the box naively; a plugin written for speed would do
better, and these are not a statement about WebAssembly's ceiling.

| Size | Native Gaussian (σ 2) | Plugin box blur, radius 1 | Plugin box blur, radius 4 |
| --- | --- | --- | --- |
| 12 MP | 267 / 280 | 833 / 856 | 6,756 / 10,308 |
| 24 MP | 540 / 562 | 1,655 / 2,939 | 20,194 / 25,412 |
| 45 MP | 1,050 / 1,063 | 4,321 / 9,726 | 46,942 / 48,272 |

Tiles run one after another on one thread. Peak working set for the plugin runs was within 3% of the native pointwise run (e.g. 1,403 MiB against 1,393 MiB at 45 MP): the sandbox's
cost is time, not memory.

### Installed plugins at start-up

100 copies of a small plugin, measured by re-opening the store: opening the store 0.1 ms
(none), 4.0 ms (10), 20.5 ms (100); listing them, which re-reads, re-validates and compiles
each, 21.7 ms (10) and 205.7 ms (100); extra process memory 0.3 MiB (10) and 3.0 MiB (100).
Installing one took 5.5–5.8 ms on average, including its self-test. Nothing is compiled when the store is opened; each plugin is read, validated and compiled the first time it is listed or used, and cached after. The application lists plugins when it starts, so that cost is paid then.

### Does the estimator predict the real peak?

Two answers.

*Whole-pipeline renders* (peak working set measured, against the estimate the application
refuses on). The estimate was above the measurement every time:

| Size | Native contrast | Native blur | Plugin (any) | Estimate: pointwise / blur |
| --- | --- | --- | --- | --- |
| 12 MP | 373 MiB | 556 MiB | 384–385 MiB | 497 / 680 MiB |
| 24 MP | 739 MiB | 1,105 MiB | 750 MiB | 955 / 1,321 MiB |
| 45 MP | 1,393 MiB | 2,087 MiB | 1,403–1,404 MiB | 1,772 / 2,466 MiB |

Measured is 75–85% of the estimate.

*Oversized-source decoders*, with a counting allocator (`tests/estimate_accuracy.rs`),
which fails if any decoder holds more than the planner priced. On a 2400 × 1800 fixture for
PNG (8-bit, RGBA, 16-bit), JPEG and WebP with and without alpha, region and reduced copy,
the decoder alone lands at 23–90% of its price and the whole open at 8–48%. **This test found
three real errors** the first version of the planner made, all fixed: WebP without alpha was
priced at 3 bytes a pixel and the decoder holds 7; the area reducer held its result twice
while finishing; and the opening price omitted a bounded preview and the decoders' own
buffers. See [oversized-images.md](oversized-images.md#are-the-prices-right).

Not validated: the estimator against a physically small machine. This host has 128 GB.

### Region decoding, per format

80 MP (48 MP for WebP and DNG), a 2000 × 2000 window, one process each. Full details and
caveats in [oversized-images.md](oversized-images.md#measured).

| Format | Whole frame | Region (top / middle / bottom) | Reduced to ¼ |
| --- | --- | --- | --- |
| PNG 8-bit | 471 ms, 234 MiB | 100 / 251 / 388 ms, **17 MiB** | 643 ms, 159 MiB* |
| JPEG | 374 ms, 257 MiB | ~385 ms, **257 MiB** | 559 ms, 173 MiB* |
| WebP lossless | 538 ms, 325 MiB | ~515 ms, **325 MiB** | 1,761 ms, 325 MiB |
| DNG | 1,212 ms, 1,657 MiB | 163–171 ms, **231 MiB** | not offered |

\* measured before the reducer was changed to build its result in place; the figure after
the change is lower (the accuracy test shows the reducer within its price).

### What the planner decides, by machine

The planner's own output for each format and size (`cargo run --release --example
admission_matrix`), automatic budgets for four machine sizes. A header-only table: nothing
is decoded.

| Machine (free) | Budget | Whole-image ceiling | 12 / 24 MP | 45 MP | 100 MP | 200 MP |
| --- | --- | --- | --- | --- | --- | --- |
| 8 GiB (4) | 2.5 GiB | 43 MP | whole | **region ≤ 43 MP** | region ≤ 43 MP | region ≤ 43 MP |
| 16 GiB (9) | 6.6 GiB | 111 MP | whole | whole | whole (60–73% of budget) | region ≤ 111 MP |
| 32 GiB (24) | 18.4 GiB | 308 MP | whole | whole | whole | whole (DNG: 52%) |
| 128 GiB (103) | 79.4 GiB | 1,332 MP | whole | whole | whole | whole |

PNG, JPEG, WebP and DNG reach the same verdict in every cell; they differ only in whether a "whole" carries a high-memory warning (a 100 MP DNG on the 16 GiB machine would use 73% of the budget against 60% for the others, and a 200 MP DNG on the 32 GiB machine 52%). **The trade-off, stated:** on an
8 GiB laptop with 4 GiB free, a 45 MP photograph is offered a region of up to 43 MP — nearly
all of it — where the old constant would have opened it whole (67 MP ceiling) and risked the
machine. That is the point of the change, and it is a real loss of convenience on small
machines.

## Packaging

`npm run tauri build` produced both Windows bundles for 0.14.0. The build was checked by the
files it produced, not by its exit status, which has reported success on this machine while
producing nothing.

| Artifact | Size | SHA-256 |
| --- | --- | --- |
| `nsis/PhotoForge_0.14.0_x64-setup.exe` | 13,108,865 bytes | `a2db40aebf0c7be693a5dbb75515770c2e66ca3ce50eac39d220b0cc44a309d1` |
| `msi/PhotoForge_0.14.0_x64_en-US.msi` | 19,189,760 bytes | `4ba9c0db284cd1a11ffe283467c51d83c0ba0b26077644f464d9f25d7155c02b` |
| `photoforge.exe` (also `PhotoForge-portable.exe`) | 59,049,984 bytes | `9e912e69bf35716eb81dce0ddfe794cd288c8ce0c92baf8edec51eab20b51f97` |

`SHA256SUMS.txt` (repository root, and `release/0.14.0/`, which is not tracked) records these
hashes of the **final bytes**, taken after the last build; the copies in `release/0.14.0/` were
checked byte-for-byte against the build output, and the sums verify against the files.

**All three report `NotSigned`** from `Get-AuthenticodeSignature`. No production signing
identity exists here and no certificate was generated; a self-made one would be a fabricated
one. The plugin packages are unsigned too, and the installer for one says so.

What is in them, checked rather than assumed:

* The MSI installs **two files**: `photoforge.exe` and `THIRD_PARTY_NOTICES.md` (read from its
  file table). The NSIS installer installs those and its uninstaller.
* **No test plugin, adversarial module, RAW fixture or cache is in the executable**: none of the
  fixture names (`infinite_loop`, `memory_hog`, `lying_locality`, `adversarial`,
  `photoforge.example`, `raw-fixtures`) appears in it.
* **An unused library was removed from the MSI.** The first 0.14.0 MSI also shipped a 19 MB
  `photoforge_lib.dll` beside the executable, which the executable does not load (it is
  linked statically). It existed because the library was built as `cdylib` and `staticlib`
  (for Tauri mobile) as well as `rlib`; the bundler packs any DLL next to the executable.
  The library is now `rlib` only, and the MSI went from 25.4 MB to 19.2 MB. This also existed
  in earlier versions.
* **The executable inside each installer is not byte-identical to the portable one.** Tauri
  patches a three-byte bundle-type marker into the copy it packs (`nsis`, `msi`) and leaves the
  portable file unpatched. The NSIS-installed copy was compared with the portable file: three
  bytes differ, at one offset. The MSI-installed copy also differs; it was not diffed.
* The executable grew from 45.3 MB (0.13.0) to 59.0 MB; see
  [dependency-decisions-phase-14.md](dependency-decisions-phase-14.md).

## Native validation

Done on the packaged build, on this machine, Windows 11 Pro build 26200, WebView2 runtime
154.0.4258.62. The window was driven through Windows UI Automation (the screen-control tool
cannot target a portable executable), which reads the *real* accessibility tree of the real window.

**Two defects were found by running the packaged application. Both are fixed, and both fixes
were re-verified on the rebuilt executable.**

1. **The window could not be closed.** A close request (the red button, or `CloseMainWindow`)
   did nothing, on a fresh instance too. As soon as the interface registers a close handler,
   Tauri vetoes every close, and the handler's wrapper calls `destroy()` once the handler has
   declined to prevent it; `destroy` is not in the default window permissions, so the call is
   refused and the window stays. **This was true in 0.13.0.** The earlier "always closeable" fix
   addressed a different failure and was tested in jsdom, which cannot see a permission file; I found no
   record that closing was checked in the packaged window, and the Phase 13 report says its packaged GUI
   matrix was not completed. Fixed by granting `core:window:allow-destroy`, the
   only permission added to the window in this phase. Re-verified: a fresh instance exits within
   10 s of a close request, also with Settings open, from the portable executable, from the NSIS
   install and from the MSI install. A regression test (`src/lib/utils/capabilities.test.ts`) now ties
   the permission to the calls that need it, and fails if the interface starts calling a window
   method that has not been granted.
2. **The Memory page said "A lower budget is saved but not in force yet" when nothing had been saved.**
   It compared the freshly recomputed Automatic budget with the one fixed at start-up, and free
   memory had simply fallen. Fixed to fire only when a lower *setting* was saved; three tests.
   Re-verified: the page shows "Memory budget: 83.3 GB" and no note.

What was checked in the packaged window:

* it starts, responds, and exposes a named accessibility tree: the File actions group, Open, Export,
  Commands, Settings, Undo, Redo, Image preview, Editing controls and the status text;
* **Settings, Memory**: installed memory 127.7 GB, free 107–109 GB, the budget (83–84.5 GB) with where
  it came from and the 19.2 GB reserve, "Video memory: Not measured" with the reason, the whole-image
  ceiling (1,418 MP) and its derivation, Automatic and Manual choices, Apply;
* **the command palette** lists *Manage plugins…*, *Automation…*, *Run a plugin filter…* ("No installed
  plugin has a filter that can run."), *Record a macro…* ("This needs a layered document: open or
  create one first.") and *Stop recording the macro* ("No macro is being recorded.");
* **the plugin manager** opens ("Stored in …\PhotoForge\plugins", "No plugins are installed.");
* **the macro editor** opens, makes a macro, and offers **22 operations from the backend registry**;
  "Run is unavailable: Open an image first.";
* **NSIS installer** (per-user, silent): installs `photoforge.exe`, the notices and an uninstaller to
  `%LOCALAPPDATA%\PhotoForge`, a Start menu shortcut and one `HKCU` entry for 0.14.0; the installed copy
  launches and closes cleanly; the uninstaller removes the folder, the shortcut and the registry entry;
  nothing is left;
* **MSI installer**, through normal elevation (not bypassed): install exit 0, the files above, a launch
  and clean close, uninstall exit 0, no folder, no `HKLM` entry, no process left. **It installs into
  `C:\Users\<you>\AppData\Local\PhotoForge\` although it is marked per-machine (`ALLUSERS=1`)**, the
  same folder the NSIS installer and PhotoForge's own data use. For a standard user who elevates with
  another account, that would be the other account's profile. Not changed in this phase.

**Network, observed.** PhotoForge's own process (`photoforge.exe`) held **no socket** in any
observation. The WebView2 processes were different on different launches:

| Launch | Observation |
| --- | --- |
| Portable, after 12 minutes up | none, across 8 samples over 32 s |
| Portable, fresh, 8–32 s | none |
| NSIS-installed, first launch | from 3 s to 60 s, **two established TLS connections** from `msedgewebview2.exe` to `52.96.121.50:443` (no reverse name; the address is in a block that appears to belong to Microsoft, which was not verified here) |

This is the behaviour `docs/privacy.md` and `docs/webview-network-boundary.md` already disclose: WebView2
is an operating-system component whose own traffic the embedding application does not control.
**PhotoForge makes no zero-network claim.** The plugin system adds no network access of its own.

**Not done, so not claimed:**

* **Display scaling at 125%, 150% and 200% was not exercised.** Changing it is a system setting this work
  does not change, and browser zoom is not a substitute.
* **No hands-on flow through the packaged window's file dialogs**: opening an image, installing a plugin,
  recording and running a macro, and the oversized-image dialog were **not** run in the packaged app.
  Their behaviour rests on the test suites above. Driving a native file dialog failed (the dialog's
  automation timed out while the application was inside it), and I stopped rather than guess.
* **No screen-reader pass.**
* **A mistake during this validation, disclosed:** two bursts of keystrokes (a file path and Enter) meant
  for PhotoForge's file dialog may have gone to another foreground window, which was your Chrome, because
  focus cannot be forced while the machine is in use. Nothing visible changed in the page, no file in
  your standard folders changed, and I sent no further global keystrokes. It is worth a glance at the tab.

## Security review

Done by reading the new surface, not by an automated scanner beyond `cargo audit`.

* **Dependencies.** `cargo audit`, offline, against an advisory database dated 3 August 2026:
  0 vulnerabilities, 18 unmaintained/unsound warnings, none in a crate Phase 14 added. The
  database was not refreshed. [dependency-decisions-phase-14.md](dependency-decisions-phase-14.md).
* **New IPC commands** (transactions, planning, plugins, macros, resources, source probing)
  take structured arguments that are validated by the engine; the document the interface
  sends is validated again in Rust. **One permission was added to the window**, found by running the
  packaged application: `core:window:allow-destroy`, without which the window could not be closed
  (see Native validation). It lets the interface destroy its own window and nothing else. The window's
  permissions are now `core:default`, `core:window:allow-destroy`, `dialog:allow-open`, `dialog:allow-save`,
  and a test fails if they change or if the interface starts calling an ungranted window method.
* **Paths.** Plugin packages and macro files go through the strict local-path guard
  (network shares, device names, streams and traversal refused) and a hard size cap on the
  read. *Found and fixed in this review:* the macro commands originally checked only
  "absolute and `.json`", which would have let a network path through; and reads relied on a
  size check that a growing file could race. The file commands only ever read or write the
  one kind of document they are for.
* **Not changed, observed:** the older commands that take a path from the interface
  (`open_image`, `probe_image_source`, `import_workflow`, `export_workflow`, exports) do not use
  the strict guard. They predate this phase and take a path the person chose in a file dialog.
* **No script, no HTML injection.** No `eval`, `new Function`, `innerHTML` or `{@html}`
  anywhere in the interface; plugin names, descriptions, READMEs and log lines are rendered
  as text.
* **Plugin sandbox.** The residual risks are in
  [plugin-security.md](plugin-security.md#residual-risk): the engine runs in-process with
  no OS sandbox behind it; side channels are not claimed absent; a self-test samples; denial
  of service is bounded, not removed; packages are not signed.
* **Network.** The plugin system adds none. WebView2's own traffic is as disclosed before.
  PhotoForge makes no zero-network claim. Observations are in [Native validation](#native-validation).

## Still incomplete or unverified

* **The Layers panel's simple actions are not routed through the engine at commit time.** They
  are TypeScript functions agreed with the engine by 25 parity vectors over 15 operations;
  `core.layer.add_pixel` is not in the vectors. Two implementations agreed by test are not one
  implementation. The single-image edit stack and the three history stacks are unchanged.
* **Out-of-core editing does not exist.** The pixel store, the compositor and project
  persistence hold whole buffers.
* **Plugins run in the PhotoForge process.** There is no OS-level sandbox, no separate
  process. An engine flaw is a process compromise. Moving them out is the single largest
  hardening step left.
* **Packaged-window hands-on flows were not run** (opening an image, installing a plugin, recording and
  running a macro, the oversized dialog), nor display scaling at 125/150/200%, nor with a screen reader.
  Two defects were found by what *was* run, which is a reason to distrust the rest of the packaged
  matrix until it is. The MSI installs into the elevating user's `LocalAppData`.
* **Nothing is signed** — not plugins, not the installer, not the executable. No legitimate
  production identity exists here and none was fabricated.
* **A plugin filter costs 2.1–2.5× a native pointwise filter**, and the example neighbourhood
  filter is slow at radius 4 (about 1 MP/s). Tiles run serially on one thread; plugins do not
  use the GPU. Unmeasured: a plugin written for speed.
* **Presets.** Macros are saved sequences and last-used filter values are remembered, but named
  parameter presets for a plugin filter do not exist.
* **The recorder is narrow.** It records Layers-panel actions that are registered operations
  and reports the rest. A layer with a shared or empty name is recorded by identifier and the
  macro then replays on that document only.
* **The estimator was checked on small fixtures with a counting allocator and on 48–96 MP files
  by working set, on a 128 GB machine.** Not on a physically small machine; the small-machine
  cases are simulated with fixed probes.
* **The advisory database was two months old**, and `jpeg-decoder` is in maintenance mode
  upstream (noted when added, not re-verified) and was not audited.
* **Native GUI at 125%, 150% and 200% display scaling was not done.** Changing the display
  scaling is a system setting this work does not change, and browser zoom is not a
  substitute. No screen-reader pass was made.
* **Accessibility was checked by role, name, label and keyboard assertions and Svelte's
  compile-time checks (0 warnings), not by an automated audit tool or with assistive
  technology.**
* **The reduced-feature builds were linted and tested, not packaged.**
* **Heavy App tests are timing-sensitive under load** (30-second scoped timeouts); they pass on
  a quiet machine and fail on timeouts on a busy one.
* **Windows only.**

## Deferred to Phase 15 and later

Plugin marketplace, auto-download and cloud registry; unrestricted Python, JavaScript or shell
scripting; native-DLL plugins; generative fill and text-to-image; cloud AI; accounts and
collaboration; video; full PSD/PSB; a CMYK document mode; proprietary RAW decoders; HDR and
print proofing. Also: running plugins in a restricted child process; routing the Layers
panel's actions through the engine; out-of-core editing; named filter presets; a command-line
interface (the registry and engine are built so one could be added without a second editing
path, but none exists).

## Git

Starting commit: `bcef259` (`origin/main` at the start; local, `origin/main` and `git ls-remote` agreed,
working tree clean). The 23 commits between there and this document, in order:

| Commit | Subject |
| --- | --- |
| `bc52480` | Audit Phase 14 and decide the plugin runtime |
| `cb28b25` | Add a resource manager and bounded reads of oversized sources |
| `1c5cd1d` | Add the oversized-image dialog, region selector and reduced-copy flow |
| `7486b85` | Open an oversized camera RAW as a region, identical to the crop of a whole development |
| `95bdde7` | Add the operation registry and transaction engine |
| `2aed0e7` | Add the plugin foundation: manifest, hostile-input package reader, sandboxed WebAssembly runtime |
| `79ee65c` | Connect plugins to the pipeline: store, render node, operations, commands, missing-plugin handling |
| `dcd8757` | Add the plugin interface: manager, palette, filter and command dialogs, panels |
| `5ab3fde` | Add step conditions and planning to the transaction engine |
| `49e16aa` | Add macros: editor, recorder, palette commands and bounded macro files |
| `27cc04b` | Add the memory settings page |
| `7c050ef` | Let a batch run plugin filters, and add the plugin benchmark |
| `86eb692` | Price what decoders really hold, and test that they do |
| `7659d43` | Document plugins, automation, resource management and oversized images |
| `a33774f` | Guard macro file paths and cap every file read |
| `be99771` | Give focus back when a dialog closes |
| `0187d70` | Record adjustment layers and direct edits in a macro |
| `1d84cf2` | Set the version to 0.14.0 |
| `4c9cb3f` | Record the dependency decisions, update the current-state documents |
| `18d40b0` | Test that a source view survives a project, and that the path guard is what refuses |
| `d1e386b` | Let the window close: grant destroy, and test the permission against its use |
| `687a0dc` | Do not report a lower budget as saved when nothing was saved |
| `d5b3c31` | Ship only what runs: build the library as rlib, record the 0.14.0 checksums |

The **ending commit is the one that adds this document** (`git log -1 -- docs/phase-14-results.md`); a
document cannot name the commit that contains it. Nothing was force-pushed. The push state, and the
check that local `HEAD`, `origin/main` and `git ls-remote` agree, are reported in the final message, not
here, for the same reason.

A scan of everything added since the starting commit found no secrets or keys, no absolute local paths,
no conflict markers, no stray artifacts (logs, caches, installers, plugin packages, WebAssembly, RAW
fixtures) and no file over 200 KB. The 0.14.0 installers and `release/0.14.0/` are not tracked
(`release/` is ignored); `SHA256SUMS.txt` is.
