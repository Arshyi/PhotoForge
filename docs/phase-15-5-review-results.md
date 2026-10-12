# Phase 15.5 — the cloud review, verified on Windows, and its follow-ups

A cloud session reviewed everything Phase 15 had added (`main..phase-15-3e-clipboard`: 160 files,
about 42,000 lines), fixed what it found most serious, and left a handoff note
([phase-15-review-handoff.md](phase-15-review-handoff.md)). It could not run Windows code. This is
what was done with its work: checked on Windows, then the findings it left open, fixed or recorded.

Status: **implemented and verified as listed; not committed.** On branch
`phase-15-5-review-followups`, on top of the cloud branch `claude/compassionate-cray-15t8v8`
(`82980f6`), which sits directly on `phase-15-3e-clipboard` (`327454e`). Nothing was rebased or
rewritten, and `main` was never touched.

## The cloud's eight fixes, on Windows

The cloud branch was built and tested here, as it was pushed:

| Gate | Result |
| --- | --- |
| `cargo clippy --all-targets`, four feature sets | clean |
| `cargo test --all-targets --no-fail-fast` | **1,851 / 1,844 / 1,843 / 1,812 passed**, 0 failed (53 binaries) |
| Real EXR files (62 of the OpenEXR project's own), after its tile-bomb admission | **52 imported and exported, 10 refused**, exactly as before: no file was newly refused |

(The six failures it reported on Linux were Linux's: Windows paths, installed fonts, and the origin
the IPC tests use. All pass here.)

## Fixed in this follow-up

| What | Where | How it is held |
| --- | --- | --- |
| **A Phase 14 build read a Phase 15 project and silently destroyed its clipping.** The format version was 2 for both | `layers/project.rs` | A project is written as the **lowest** version that holds it: version 3 when a layer is clipped or uses one of the ten Photoshop blend modes, version 2 (byte for byte as before) otherwise. Reads 1, 2 and 3; an earlier build refuses 3 by name. 4 tests; 5 mutants caught |
| Saving a project with metadata made a **second copy** of the whole project, and checked the size after | `layers/project.rs` | Attached in place; the size is checked before a byte moves. (The cap itself is unchanged: a project that fills it still cannot also carry its metadata, and says so) |
| `export_mask_png` / `export_mask_file` wrote to **any** path, including the open photograph | `commands/mask.rs` | Held to the rule the other exports have. A test through the real command, 2 mutants caught |
| `refuse_to_replace_source` could not see an NTFS **hard link** to the source | `infrastructure/image_io.rs` | Asks Windows for the volume serial number and file index. Test with a real hard link; mutant caught |
| Clipboard: a mask of all 32 bits, and a PNG header of 0xFFFFFFFF × 0xFFFFFFFF, **overflowed** (a debug panic) | `clipboard/dib.rs`, `mod.rs` | 64-bit and saturating arithmetic; tests for both; 2 mutants caught |
| Clipboard: a **colour table** before the pixels of a 16-, 24- or 32-bit bitmap was read as pixels | `clipboard/dib.rs` | Skipped; test and mutant |
| Clipboard: a copy **emptied the clipboard before** its memory was allocated, so a failed copy destroyed what was there | `clipboard/win.rs`, `mod.rs` | Everything is made and staged first |
| Clipboard: a paste held the clipboard **for the whole decode** | `clipboard/mod.rs` | The bytes are copied out and the clipboard released before decoding |
| Clipboard: the PNG decoder's own 512 MiB limit was **below** what the pixel bound admits | `clipboard/mod.rs` | Set from the same bound |
| Clipboard: a copied picture could go to the **cloud clipboard** and clipboard sync, which an editor that "sends nothing anywhere" should not cause | `clipboard/win.rs` | `CanUploadToCloudClipboard` = 0 on every copy; real-clipboard test reads it back. It stays in the local history |
| OpenEXR: near-degenerate primaries gave garbage colour (the guard was an absolute determinant) | `openexr/colour.rs` | A relative test; test with nearly collinear primaries; mutant caught |
| The report dialog rendered **every** line (a hostile SVG could freeze the window) | `PsdImportReport.svelte` | 200 lines a section and "…and N more"; the count in the heading is every finding. Test |
| The report dialog was not a modal for the **keyboard** (Ctrl+V pasted behind it) | `App.svelte` | It is part of `modalOpen`. Test |
| **Ctrl+C with text selected** on the page copied the picture instead | `App.svelte` | Left to the page. Test |
| Placing an image file the engine then refused **leaked** the decoded buffer in the session | `App.svelte` | Released. Test |

## Recorded, not fixed

* **High-dynamic-range values are clipped by most blend modes** (everything but Normal, Multiply,
  Screen, Darken, Lighten, Difference, Exclusion). Written into `docs/layers.md` and
  `docs/openexr.md`; a real fix is a design decision about what Overlay and its relatives mean above
  white.
* **PSD import decodes every channel**, so extra channels exceed the per-layer estimate by up to
  about 14 times; **PSD export's peak memory** is about three times what is guarded; **the PSD
  export of a high-precision document** is written as 16-bit sRGB and composited differently by
  Photoshop. Each is in the cloud's ledger.
* **SVG**: isolated groups are written as plain `<g>`; colour names outside the 40, `hsl()`, and
  percentage lengths are read wrongly (the last as a fraction of the canvas, not the view box);
  `<title>` is not read back, so layer names are lost on a round trip.
* **A multi-part EXR** whose earlier part has the same channel names but a codec the reader lacks is
  refused as a whole; a deep part anywhere makes the file fail though it is planned to be skipped.
* **The clipping invariant** is enforced inside engine transactions only; a project file can store
  `clipped: true` on a bottom layer. Not reachable from the interface; harmless to the renderer.
* Batch export drops the source metadata of projects and RAW files without saying so.
* The rest of the cloud's ledger (about 25 rows, mostly low severity) stands as it was written.

## Verification

| Gate | Result |
| --- | --- |
| `cargo fmt --check` | clean |
| `cargo clippy --all-targets`, four feature sets | clean |
| `cargo test --all-targets --no-fail-fast` | **1,860 / 1,853 / 1,852 / 1,821 passed**, 0 failed (54 binaries; the cloud branch ended at 1,851) |
| `npx tsc --noEmit`, `npm run check` | clean; 520 files, 0 errors, 0 warnings |
| `npx vitest run` | 91 files, **1,375 passed**, 0 failed (one unhandled error from `CommandPalette.test.ts`, as at `main`) |
| Real clipboard (`PHOTOFORGE_CLIPBOARD_TESTS=1`) | 13 + 3 passed; Pillow oracle: no problems |
| Mutation checks of the follow-ups | **12 of 12** caught |

## What was not verified

Everything that the cloud could not and this could not either: the files written, in the
applications that read them; the interface in a real window; Photoshop, Office and a browser as
the program on the other side of the clipboard.
