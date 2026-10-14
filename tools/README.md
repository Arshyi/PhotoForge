# Tools

Scripts that were written, one at a time, while the project was built, and kept because they are
how its claims were checked. None of them is needed to build or run PhotoForge. They are
documented in [`docs/development.md`](../docs/development.md); this is the index.

Paths come from [`pfenv.py`](pfenv.py): the repository root and `src-tauri` are found from the
script's own location, and scratch space (sample corpora, dumps, reports) is `<repo>/.work` unless
`PF_WORK` says otherwise.

Python packages: `python -m venv .work/venv`, then `.work/venv/Scripts/pip install -r tools/requirements.txt`.

## Oracles: read what PhotoForge wrote with programs that are not PhotoForge

| Script | Checks |
| --- | --- |
| `exr_oracle.py` | OpenEXR import and export against the OpenEXR reference reader |
| `tiff_export_oracle.py`, `tiff_dump.py` | TIFF export read by tifffile and Pillow; a structural dump of a TIFF |
| `psd_check.py`, `psd_tools_check.py`, `colour_cases.py` | PSD export: an independent strict structural check, psd-tools' own reading, and colour-depth cases |
| `svg_compare.py`, `svg_import_compare.py` | SVG export and import rendered against resvg |
| `upright_oracle.py` | A developed DNG with each orientation value against Pillow's transposes |
| `exif_oracle.py`, `export_oracle.py` | Metadata and exported pictures against Pillow |
| `clipboard_oracle.py` | The clipboard writers and readers against Pillow and the Win32 API. **It replaces the clipboard**: save it first with `clipboard_backup.ps1` and restore after. |
| `clipboard_backup.ps1` | Saves and restores the **whole** clipboard (every format in global memory, in order): `save`, `restore`, `verify`, `list`, `selftest`. Refuses (exit 3) if something on it cannot be put back; `-AllowLoss` overrides. OLE bookkeeping and formats Windows makes from others are not content and are not restored. |
| `check_corpus_expectations.py` | The expected outcome of each file in a corpus |
| `scan_control.py` | Fails if a source file contains a control character (a patch made by a shell once put a NUL into a test string) |

## Corpus scripts: real files from other programs' test suites (never committed)

`fetch_exr_samples.py` (openexr-images, BSD-3), `fetch_svg_samples.py` (Bootstrap Icons, MIT),
`fetch_psd_samples.py` (psd-tools fixtures, MIT), `prep_svg_samples.py`, `make_tiff_corpus.py`
(writes 70 files with tifffile / libtiff variants). Each writes into `<work>`.

## Mutation harnesses: break the code, watch a test fail, restore

Each `mutate_*.py` holds a list of small breakages of one feature's code, runs the tests that should
notice, and restores the file (and refreshes its modification time, so cargo does not reuse the
mutant's build). They are how this project's tests were shown to fail for the right reason.

* `python -I tools/mutate_<name>.py --dry` checks that each pattern still matches exactly once and
  **changes nothing**. Every harness takes it; any other `--option` is refused (exit 2) before a file
  is touched, because a harness that ignores its arguments would read it as "run every mutant".
  Run `--dry` first;
* `python -I tools/mutate_<name>.py [index | name ...]` runs them (minutes per mutant, each one a
  cargo build and test run).

A pattern is matched in the line-ending convention the file has (`pfenv.fit_line_ends`), so a
checkout with `core.autocrlf=true` works. They are otherwise **tied to the code as it was** when
each feature landed: a pattern that no longer matches is reported (`BAD`, or `BAD PATTERN` in a run)
and is skipped, and needs updating before it is run. At 0.15.0, `--dry` says:

| Harness | Patterns | No longer match (the code moved on) |
| --- | --- | --- |
| `mutate_clip.py` | 15 | `png-not-written`, `dibv5-not-written`, `dib-preferred-over-png`, `clipboard-not-emptied` |
| `mutate_crop.py` | 16 | the active area origin is ignored; a crop outside the active area is believed; a picture that is the whole sensor is a cut |
| `mutate_dng_meta.py` | 21 | private data is not noticed; an Exif maker note is not noticed; no Photoshop-resource IPTC; the GPS directory is not followed; a repeated tag is not skipped |
| `mutate_exr.py` | 27 | `unfilled-alpha-obeyed`, `store-budget-ignored`, `export-straight-alpha`, `export-bad-depth-accepted` |
| `mutate_followups.py` | 12 | `clipping-not-a-reason-for-v3`, `blend-modes-not-a-reason-for-v3`, `primaries-conditioning-removed` |
| `mutate_svg_import.py` | 10 | a DTD allowed (matches twice) |
| `mutate_tiff.py` | 33 | orientation 6 is not rotated; the profile is not embedded |
| `mutate_upright.py` | 17 | the transpose is not one (matches twice); a whole open is not turned; the tag is not spent; re-developing does not turn; a raw layer is not turned when added; a batch is not turned; a recorded turn is version two; a region may be turned |
| `mutate_fixes.py`, `mutate_meta.py`, `mutate_region.py`, `mutate_svg.py` | 4, 8, 26, 8 | none |
| `mutate_review_items.py`, `mutate_layout.py` (Phase 15.11 and 15.12) | 18, 5 | none |

That is 30 of the 197 above (the last row is new and complete). They are the proof the tests were checked when the code was written, not a
suite that is current; the ones above are what a new change to those areas should re-establish by
writing a fresh mutant. (The `mutate_upright.py` ones were superseded by `mutate_crop.py` and
`mutate_region.py` when 15.8 and 15.9 changed what "the picture" is.)

Never format, commit or build in another window while one runs: it edits source files in place. If
a run is killed, the file it was changing stays changed: `git status` shows it, and
`git checkout -- <file>` restores it (a harness that copies the file first leaves
`<file>.mutbak` beside it; delete that too).

`mutate_clip.py` runs the tests that **replace the clipboard**: save it first
(`clipboard_backup.ps1 -Action save`) and restore it after.

## The packaged-window harness

See [`validate/README.md`](validate/README.md).
