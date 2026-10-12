# Phase 15 review and fixes: handoff

Written 12 October 2026 at the end of a cloud session. This note says what was reviewed, what was fixed, what was and was not verified, and what is still open, so that work can continue from a local checkout without this session.

## Where the work is

- Branch: `claude/compassionate-cray-15t8v8`, tip `8386b4b`.
- It is `phase-15-3e-clipboard` (`327454e`, your ten Phase 15 commits) plus eight fix commits on top. Nothing was rebased or rewritten, so it is a plain fast-forward of `phase-15-3e-clipboard`:
  `git fetch origin && git checkout phase-15-3e-clipboard && git merge --ff-only origin/claude/compassionate-cray-15t8v8`
- `main` and the `phase-15-*` branches were not touched. No pull request was opened.

## What was done

1. Reviewed `origin/main..phase-15-3e-clipboard` (160 files, about 42,000 added lines) in eleven areas: the 15.0 editing path, PSD import, PSD export, clipping/blend modes/project format, metadata, SVG, OpenEXR, the Windows clipboard, the IPC and path-guard surface, the frontend, and the claims in the results documents.
2. Each reported finding was checked by an independent agent whose job was to refute it. 61 findings were reported, 50 confirmed, 11 refuted. The ledger at the end lists all of them.
3. Fixed the four confirmed high-severity findings and then, in this order, the export-overwrite, metadata-choice, merge-down/clipping, raster-export and paste-success findings. Each fix has a regression test that fails on the old code and passes on the new, and each commit was reviewed by a second agent trying to break it.

| Commit | What it fixes |
| --- | --- |
| `1853d2e` | OpenEXR import: the memory budget now counts what the header and blocks make the reader hold (tile bomb). A 1 MiB cap on header bytes was added in a final repair that was not re-reviewed. |
| `5cab7f7` | PSD import: layer and group mask allocations are budget-checked before they are made. |
| `3ecfa52` | SVG import: element nesting is capped at 256 (stack overflow), and `url(#id)` lookups use an index instead of rescanning the document. |
| `de61df8` | PSD, SVG and EXR exports refuse to overwrite the file the document was opened from. Reuses the raster check; Unix also compares file identity. One existing EXR test had been asserting the overwrite and was corrected. |
| `b23e98e` | Every export follows the person's current metadata choice (shared store in `src/lib/metadata.ts`); the 16-bit PNG export and the staged workflow export were ignoring it; the batch hint no longer claims EXIF/GPS is stripped. |
| `a025b32` | Merge Down and Convert to Smart Object keep the picture identical or refuse cleanly when clipping is involved. One rule (`range_clipping` in `layers/clip.rs`) is used by the engine, the merge command, convert, and the TypeScript mirror. |
| `bff3bf6` | Raster export (PNG, JPEG, WebP, 16-bit, with profile) works for documents with no source file (pasted or blank documents). |
| `8386b4b` | Paste no longer announces success when the engine refused the placement, and releases the orphaned pasted buffer. |

## Verification state

All of this ran on Linux in a container, not on Windows.

- `cargo fmt --check`: clean. `cargo clippy --lib --tests`: only three warnings that were already there (`network_policy.rs` unused import, `commands/smart.rs` unused `mut`, `clipboard/mod.rs` `DIBV5_MASK_TABLE`).
- Rust library and integration tests at `bff3bf6`: 1,831 passed, 6 failed. The six failures also fail on `main` and are platform problems: `plugins::tests::rejects_absolute_entry` and `local_path::tests::device_names_are_refused_when_a_path_is_about_to_be_opened` use `C:\` paths; the two `text::font` Persian tests, `layers::text` caret test and `text_layers::every_setting_changes_what_is_drawn` depend on installed fonts.
- Frontend at `8386b4b`: 1,371 tests passed, `svelte-check` 0 errors and 0 warnings. `vitest` exits 1 because `CommandPalette.test.ts` leaves one uncaught `boom` error; `main` does the same.
- The last Rust run was before `8386b4b`; that commit touches only `src/App.svelte` and `src/App.clipboard.test.ts`.

## Not verified (please check on Windows)

- Everything under `cfg(windows)`: the clipboard module (`clipboard/win.rs`, the real-clipboard tests), the Windows-only case-alias test `another_case_of_the_source_is_the_source`, and Windows short-name/case behaviour of the export guards. The clipboard FFI was only read, line by line, by a reviewer, and found sound.
- The real `open_clipboard_image` command. The pasted-picture tests build that session by hand.
- Hard-link detection exists only on Unix (`refuse_to_replace_source`), and the raster check has none.
- No hands-on pass through the packaged window (file dialogs, drag and drop) was possible.

## Behaviour changes to be aware of

- An SVG nested deeper than 256 elements is refused.
- An EXR whose header carries more than 1 MiB of attributes is refused. The EXR fix is bounded by the memory budget: on a machine with roughly 10 GB or more of free memory the 446-byte attack file is still admitted.
- Merges that would split a clipping group, or leave layers clipped to an adjustment layer that is replaced by pixels, are refused with a message mentioning the clipping group. The Merge Down button is not disabled beforehand; the refusal shows after the click.
- A raster export whose source file was deleted or unplugged still fails (unchanged).

## Open work, most valuable first

1. `PROJECT_FORMAT_VERSION` is still 2 although Phase 15 writes new state (`Layer.clipped`, metadata entries, ten new blend-mode names). A Phase 14 build silently drops `clipped` on load and loses the clipping on re-save. Not checked by a second agent (see "Coverage gaps").
2. HDR/EXR highlights are clipped by most blend modes in `blend_linear` (`layers/linear.rs`), including all ten new ones, with no note in the import report or docs.
3. Saving a project with metadata copies the whole encoded project and can fail the 1 GiB cap that Phase 14 saves passed.
4. SVG export writes isolated groups as plain `<g>`; SVG import turns colours outside its 40 built-in names, and `hsl()`, into black with a clean report; percentage lengths resolve against the canvas, not the viewBox.
5. `createLayer('import')` in `src/App.svelte` (about line 2335) leaks the imported pixel buffer when the placement is refused. A one-line `if (!(await runPanelPlan(...))) void releaseUnreferencedPixels()` fixes it.
6. `export_mask_file` and `export_mask_png` (`src-tauri/src/commands/mask.rs`) write to any absolute path without a source comparison. This was already so before Phase 15.
7. The rest of the open findings are in the ledger below (35 open, mostly low severity).

## Running the tests in a fresh Linux container

- Tauri needs system libraries: `apt-get install -y libwebkit2gtk-4.1-dev libgtk-3-dev libsoup-3.0-dev librsvg2-dev libayatana-appindicator3-dev libssl-dev pkg-config build-essential`.
- `cargo test` needs the built frontend: run `npm ci && npm run build` first, or the build fails with "frontendDist ... doesn't exist".
- Disk: a default debug build is about 28 GB. Use `CARGO_PROFILE_DEV_DEBUG=0 CARGO_PROFILE_TEST_DEBUG=0 CARGO_INCREMENTAL=0` and `cargo test --lib --tests` (skip examples).
- The integration tests in `src-tauri/tests` send commands from the origin `http://tauri.localhost`, which Tauri treats as a local origin only on Windows. On Linux they fail with "`<command>` not allowed. Plugin not found" (about 90 failures). To run them there, temporarily `sed -i 's#http://tauri.localhost#tauri://localhost#g' src-tauri/tests/*.rs` and revert afterwards. Making the tests choose the origin per platform would remove this trap.
- Use `--no-fail-fast`, otherwise cargo stops at the first failing suite.

## Coverage gaps the review itself reported

Changed files no area covered: `src-tauri/Cargo.toml`, `src-tauri/Cargo.lock`, `src-tauri/examples/layer_benchmark.rs`, `src-tauri/examples/precision_benchmark.rs`, `src-tauri/examples/tiled_benchmark.rs`.

1. [checked by reading] HDR and EXR documents are silently clipped by most blend modes, including all ten new Photoshop modes. blend_linear lists only 7 modes (Normal, Multiply, Screen, Darken, Lighten, Difference, Exclusion). Every other separable mode, including all ten new ones, goes through `_ => mode.blend(b, s)`. That call clamps both operands and the result to 0..1 via sanitize/clamp_unit and evaluates the mode in linear light, unlike the 8-bit path. Non-separable modes go through srgb_encode/decode and the same clamp. EXR import (new in this diff) makes values above 1 common, and docs/openexr.md promises 'kept, unclipped' only for storage. Nothing in the import report or the docs says a layer with a non-listed blend mode loses its highlights. A one-layer EXR is safe; a multi-layer one under Overlay, Soft Light or Vivid Light is not. The same linear-light evaluation is why 16-bit PSD imports, where plan.float selects LinearSrgbF32, may composite differently from Photoshop. The psd-export finding covers only the export side.  
   Where: `src-tauri/src/layers/linear.rs:241 (blend_linear), src-tauri/src/layers/blend.rs:119 (blend/sanitize), src-tauri/src/psd/import.rs:324 (precision choice), docs/openexr.md:20`

2. [checked by reading] Saving a project with source metadata can fail, and always duplicates the project in memory. attach_source_metadata copies the entire encoded project into a new Vec (with_capacity(bytes.len()+1024), then two extend_from_slice calls over the whole body) and only afterwards checks `out.len()+8 > MAX_PROJECT_BYTES` (1 GiB). Two consequences. (a) Peak memory is about 2x the project (up to roughly 2 GiB) on every save and every recovery snapshot, outside any resource gate that I could see. The recovery path write_snapshot_typed_with_metadata does the same. (b) A project that encoded under the 1 GiB cap and saved on Phase 14 can now fail with ProjectTooLarge because up to about 1 MiB EXIF + XMP + IPTC is appended after encode_project_typed's own cap check. Autosave would fail the same way. The metadata area covered the loader (a finding was refuted); the save/recovery side was not reported.  
   Where: `src-tauri/src/layers/project.rs:99-190 (attach_source_metadata), :1077 (save_project_typed_with_metadata), src-tauri/src/layers/recovery.rs:121`

3. [checked by reading] PROJECT_FORMAT_VERSION stays at 2 although Phase 15 writes new state: the Layer.clipped flag, metadata/* container entries, ten new blend-mode names, and PSD-derived layer content. Loading Phase 14 files in Phase 15 is fine: clipped has serde(default), and I found no strict entry-count check on decode at origin/main. In the other direction, Layer has no deny_unknown_fields. A Phase 14 build therefore silently drops `clipped` on load, and on re-save permanently destroys the clipping. It also fails with a generic 'manifest is unreadable: unknown variant' for new blend modes instead of UnsupportedProjectVersion. This is a downgrade-safety and data-loss gap that no version check can catch. The compositing reviewer saw project.rs only for the metadata hunk.  
   Where: `src-tauri/src/layers/project.rs:21, :749, src-tauri/src/layers/model.rs:225`

4. [NOT checked] The new compositing core produced no confirmed finding in its own math, only in structural operations. The files involved are clip.rs (644 new lines), tiled.rs (+326), composite.rs, linear.rs and blend.rs. Unexamined here: (a) whether the tiled renderer, the 8-bit path and the float path agree for a clipping group nested in a non-isolated (pass-through) group or an isolated group, a transformed or masked base, or a base with a blend mode other than Normal; (b) whether tile halos and the cache fingerprint (cache.rs clip_bases, influence_of) cover a clipped layer whose transform moves it outside its base's influence; (c) whether the resource estimate (one buffer per distinct clipped depth, resources/mod.rs ~243) matches the renderer's real simultaneous buffers (clip buffer + isolated-group buffer + cross_fade canvas.clone()). The prompt names clipping + PSD export + tiled render as an interaction, and nobody reported on it.  
   Where: `src-tauri/src/layers/clip.rs, src-tauri/src/layers/tiled.rs, src-tauri/src/layers/cache.rs:384, src-tauri/src/resources/mod.rs:243`

5. [could not check] Nothing in this review environment compiles or runs the Windows-only code: all of clipboard/win.rs (unsafe Win32 calls) and the cfg(windows) arms in clipboard/mod.rs, plus the new `windows =0.61.3` feature set (Win32_Foundation, DataExchange, Memory, Ole). The repo has no .github/workflows, so no CI does either. A signature mismatch against the pinned crate would break the Windows build entirely. win.rs opens the clipboard with OpenClipboard(None) and then calls EmptyClipboard. Microsoft's remarks for EmptyClipboard say a NULL owner makes SetClipboardData fail, although in practice immediate-data copies by null-owner programs usually work. That is worth confirming on a real Windows machine (the clipboard_real tests would do so, if they actually run there).  
   Where: `src-tauri/src/clipboard/win.rs, src-tauri/src/clipboard/mod.rs:15-232, src-tauri/Cargo.toml:105-115`

6. [checked: absence by grep] Copying a picture puts it into Windows clipboard history and, if the person enabled sync, the cloud clipboard. win.rs registers no CanIncludeInClipboardHistory, CanUploadToCloudClipboard or ExcludeClipboardContentFromMonitorProcessing format, and docs/clipboard.md does not mention the exposure. For a product whose rule is 'the app makes no uploads' and whose documents may hold private photos, the OS can upload what the app copies. This may be an acceptable product choice, but it is undecided and undocumented. The clipboard reviewers looked at parsing, locking and the copy sequence, not at what Windows does with the data afterwards.  
   Where: `src-tauri/src/clipboard/win.rs (copy path), docs/clipboard.md`

7. [checked by reading call sites] The clipping invariant is enforced only inside engine transactions. normalise_clipping is called at engine.rs:239 and :324 and nowhere else. decode_project, LayerDocument::validate, PSD import and the non-engine layer commands (text, shape, smart object, paste, merge_layer_pixels and the like) never call it. A project file, or an edit through one of those commands, can therefore store `clipped: true` on the bottom layer of a stack. The renderers treat that layer as unclipped, but the model's own comment says the flag then 'attaches it to whichever layer is next moved or inserted beneath it'. The result is a visual change on a later unrelated edit. move_layer also silently carries clipped_run_ids along. The engine reviewers covered the transaction paths, not these entry points.  
   Where: `src-tauri/src/layers/model.rs:820-835, src-tauri/src/operations/engine.rs:239,324, src-tauri/src/layers/project.rs (decode_project)`

8. [checked by reading the diff] Batch export drops the source metadata of projects and RAW files without saying so. run_batch passes source_metadata = None for .pfproj inputs and for RAW inputs, although projects now store EXIF/XMP/IPTC (read_source_metadata) and RAW files carry metadata. Only ordinary image inputs flow through Plan::new. Under a 'keep' policy a batch silently writes files with no metadata and no report entry. The metadata reviewers' batch findings were about the panel text and the policy not being passed from the UI, not about this input-dependent loss.  
   Where: `src-tauri/src/application/batch.rs:139-160`

## Findings ledger

Status "FIXED" names the commit; "open" means confirmed and not fixed; "refuted" means the skeptic could not reproduce it. "Skeptic severity" is the highest severity any verifying agent gave. Several rows are the same bug reported from different areas.

| Status | Skeptic severity | Area | Where | Finding |
| --- | --- | --- | --- | --- |
| FIXED 5cab7f7 | high | psd-import | `src-tauri/src/psd/import.rs:1007` | Layer-mask decode limits its allocation by the mask's own declared rectangle, so a tiny file can force a multi-GB or process-aborting allocation |
| FIXED 3ecfa52 | high | svg | `src-tauri/src/svg/import.rs:1227` | Unbounded recursion on element nesting overflows the stack (process abort) |
| FIXED 3ecfa52 | high | svg | `src-tauri/src/svg/import.rs:1061` | Quadratic document scan per url(#id) lookup lets a small file hang the importer for minutes |
| FIXED 1853d2e | high | openexr | `src-tauri/src/openexr/import.rs:446` | Admission budget ignores header-driven block sizes, so a tiny file can force multi-GB allocations (and an uncatchable abort) |
| FIXED a025b32 | medium | engine | `src-tauri/src/operations/engine.rs:801` | Merge down ignores clipping: the merged layer loses its clip and clipped layers above it are released |
| FIXED 5cab7f7 | medium | psd-import | `src-tauri/src/psd/import.rs:996` | Group and adjustment-layer masks allocate a canvas-sized buffer before any pixel-count check, so a small PSB can request up to 17 GB |
| FIXED a025b32 | medium | compositing | `src-tauri/src/operations/engine.rs:801` | Merge down un-clips the layers above the merged pair, and merge ranges ignore clipping |
| FIXED a025b32 | medium | compositing | `src-tauri/src/commands/smart.rs:122` | Converting a clipped layer to a smart object drops its clip relation |
| FIXED b23e98e | medium | metadata | `src/lib/components/ProfessionalWorkspace.svelte:439` | 16-bit PNG export and the batch panel do not use the person's metadata choice |
| FIXED bff3bf6 | medium | ipc-security | `src-tauri/src/commands/layers.rs:1381` | Export to PNG/JPEG/WebP fails for any document with no source file (clipboard documents, new blank documents) |
| FIXED de61df8 | medium | ipc-security | `src-tauri/src/commands/layers.rs:1314` | New PSD/SVG/EXR exports can overwrite the very file the document was opened from |
| FIXED 8386b4b | low | engine | `src/App.svelte:3046` | Paste reports success after the engine refused or dropped the new layer |
| FIXED b23e98e | low | metadata | `src/lib/components/ProfessionalWorkspace.svelte:825` | Batch panel says 'camera EXIF/GPS is stripped' next to a policy that can keep GPS |
| FIXED 8386b4b | low | frontend | `src/App.svelte:3046` | Paste announces success after the engine refused or failed to place the picture |
| FIXED 8386b4b | low | claims | `src/App.svelte:3045` | Paste announces success after the engine refused the placement; the paste test passes because its mock returns {} |
| open | medium | psd-import | `src-tauri/src/psd/import.rs:748` | Every channel with id >= -1 is decoded and held, so the per-layer memory estimate is exceeded up to about 14x by extra channels |
| open | medium | psd-export | `src-tauri/src/psd/export.rs:1314` | High-precision documents are written as 16-bit sRGB with every layer property called exact, but Photoshop composites that file in encoded space and PhotoForge composites in linear light |
| open | medium | psd-export | `src-tauri/src/psd/export.rs:1346` | Export memory peak is about three times the encoded layer data plus several copies of the flattened picture, but the only guards count one copy |
| open | medium | svg | `src-tauri/src/svg/import.rs:1697` | Shapes plus groups can exceed 512 layers, so a valid SVG fails to open instead of being capped |
| open | medium | svg | `src-tauri/src/svg/import.rs:1295` | Percentage lengths are resolved against the canvas in pixels, not the viewBox user space |
| open | medium | claims | `src-tauri/src/svg/export.rs:241` | SVG export writes default (isolated) groups as plain <g>, so children's blend modes escape the group, and nothing is reported |
| open | low | engine | `src/lib/layers/sharing.ts:31` | shareUnchanged shares only whole unchanged layers, so every edited layer's mask, and the smart sources, are re-allocated into each undo entry |
| open | low | engine | `src-tauri/src/operations/engine.rs:348` | Every panel edit ships all mask data over IPC, decodes each mask at least three times, and hashes the whole document for an unused revision |
| open | low | engine | `src/lib/components/LayersPanel.svelte:67` | With the selected layer inside a collapsed group, 'Clip to layer below' is disabled and unchecked, and says there is no layer beneath |
| open | low | psd-import | `src-tauri/src/psd/import.rs:226` | The colour note claims an embedded profile is equivalent to sRGB when the profile could not be read at all |
| open | low | psd-import | `src-tauri/src/psd/document.rs:699` | Blend If ranges, mask density/feather and document-level resources are parsed past and never reported, despite the 'nothing is dropped silently' contract |
| open | low | psd-export | `src-tauri/src/psd/export.rs:949` | A pass-through group with a transform is baked as an isolated group with the transform applied to its content, but the renderer applies that transform only to the group's mask |
| open | low | psd-export | `src-tauri/src/psd/export.rs:1338` | The PSD/PSB switch and the memory gate are computed from the wrong set of layers |
| open | low | psd-export | `src-tauri/src/psd/export.rs:1391` | Whole-document edits that keep the picture's size are dropped from the flattened image and reported as changing its size or shape |
| open | low | metadata | `src-tauri/src/metadata/mod.rs:349` | 'Keep everything' still carries pre-edit previews (XMP thumbnails, maker-note previews) while the UI says the preview was removed |
| open | low | svg | `src-tauri/src/svg/import.rs:929` | Colour names outside the 40 built in, and hsl(), are silently ignored: shapes turn black with a clean report |
| open | low | svg | `src-tauri/src/svg/import.rs:1131` | <title> is never read on import, and export writes names only as <title>, so layer names are lost on round trip |
| open | low | openexr | `src-tauri/src/openexr/import.rs:674` | read_set decodes the first part that has the channel names, not the part admit() chose (wrong picture imported under another part's name) |
| open | low | openexr | `src-tauri/src/openexr/import.rs:650` | No completeness check on delivered blocks: a wrong or duplicated offset-table entry silently yields black or transparent bands |
| open | low | openexr | `src-tauri/src/openexr/import.rs:315` | A deep part anywhere in a multi-part file makes the whole import fail, although admit() plans to skip it and says so |
| open | low | openexr | `src-tauri/src/openexr/colour.rs:159` | Primaries conversion accepts ill-conditioned (near-degenerate) chromaticities and produces garbage colour; the guard is an absolute 1e-12 determinant |
| open | low | clipboard | `src-tauri/src/clipboard/mod.rs:165` | Clipboard stays open (and the in-program lock held) for the whole decode of a paste, not just while bytes are copied |
| open | low | clipboard | `src-tauri/src/clipboard/dib.rs:152` | Parsers can overflow-panic in checked builds: mask 0xFFFFFFFF in Channel::new and a PNG header of 0xFFFFFFFF x 0xFFFFFFFF in decode_png |
| open | low | clipboard | `src-tauri/src/clipboard/dib.rs:250` | A colour table on a 16/24/32-bit BI_RGB bitmap (biClrUsed > 0) is ignored, so pixels are read from the wrong offset |
| open | low | clipboard | `src-tauri/src/clipboard/mod.rs:138` | Copy empties the clipboard before the new data is allocated, so a failed copy destroys the previous contents and can leave a half copy |
| open | low | clipboard | `src-tauri/src/clipboard/mod.rs:126` | decode_png's pixel pre-check admits PNGs that the image crate then refuses on its default 512 MiB allocation limit |
| open | low | ipc-security | `src-tauri/src/commands/layers.rs:1279` | PSD, SVG, EXR and clipboard document opens skip the memory-limit refresh/restore that other opens perform |
| open | low | frontend | `src/lib/components/PsdImportReport.svelte:76` | Import/export report dialog renders every entry with no cap, so a hostile SVG can freeze the window |
| open | low | frontend | `src/App.svelte:603` | Report dialog is not part of modalOpen or the inert app shell, so global shortcuts and Tab reach the document behind it |
| open | low | frontend | `src/App.svelte:2828` | Dragging a slider with the 'A new adjustment layer' target keeps only the first tick and spams a 'Wait' error |
| open | low | frontend | `src/lib/utils/shortcuts.ts:195` | Ctrl+C / Ctrl+Shift+C are swallowed even when the user has selected text on the page |
| open | low | frontend | `src/App.svelte:3302` | Undo inside the 400 ms gesture window discards the pending change and also undoes the previous edit |
| open | low | claims | `src-tauri/src/psd/import.rs:195` | PSD import silently ignores any embedded ICC profile it cannot parse (all Gray profiles) and reports it as 'equivalent to sRGB' |
| open | low | claims | `src-tauri/src/svg/import.rs:1131` | SVG import silently drops properties it claims to name: mix-blend-mode (which its own exporter writes), markers, paint-order, vector-effect; pattern fills are dropped, not 'first colour' |
| open | low | claims | `src-tauri/src/metadata/mod.rs:331` | Metadata 'Keep everything' (and the default when the packet names no place) copies XMP verbatim, so XMP previews, size and orientation stay stale, contradicting 'under every policy the thumbnail is removed / corrected for the edit' |
| refuted | none | engine | `src-tauri/src/operations/engine.rs:559` | Moving an unclipped layer into, or grouping a middle member of, a clipping group silently re-parents the clipped layers above it |
| refuted | none | psd-export | `src-tauri/src/commands/layers.rs:1717` | A .psd name that needs the large format is overwritten first and renamed to .psb second, so a failed rename reports failure after the old file has been destroyed |
| refuted | none | compositing | `src-tauri/src/operations/structure.rs:233` | Ungroup releases a clipped group's children and re-attaches layers clipped to the group onto its last child |
| refuted | none | compositing | `src-tauri/src/layers/model.rs:762` | Moving or deleting a base changes locked layers clipped to it without any lock check |
| refuted | none | metadata | `src-tauri/src/metadata/mod.rs:349` | Kept XMP is not corrected for the edit: tiff:Orientation and pixel dimensions go stale |
| refuted | none | metadata | `src-tauri/src/layers/project.rs:191` | Project loader copies whole metadata entries before applying the size cap |
| refuted | none | clipboard | `src-tauri/src/clipboard/dib.rs:241` | DIBV5 mask-table heuristic misfires on real pixel data and rejects a valid bitmap as 'cut short' |
| refuted | none | ipc-security | `src-tauri/src/commands/layers.rs:1676` | New export commands skip the strict local-path guard; export_psd_document has almost no path validation |
| refuted | none | ipc-security | `src-tauri/src/resources/mod.rs:36` | One panic inside any job-holding worker poisons the global CPU_JOB gate for the rest of the session; the new untrusted-input parsers run under it |
| refuted | none | ipc-security | `src-tauri/src/clipboard/win.rs:113` | Clipboard reads are capped at a fixed 1 GiB copy, taken before any header is examined and independent of the memory budget |
| refuted | none | claims | `src-tauri/tests/clipboard_real.rs:202` | Real-clipboard tests are counted as 12 but 3 are no-op 'step' helpers, and all of them pass by early return in the default gate |
