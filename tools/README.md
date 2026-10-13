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
| `clipboard_oracle.py`, `clipboard_backup.ps1` | The clipboard writers and readers against Pillow and the Win32 API. **They replace the clipboard**: back it up with `clipboard_backup.ps1 save` and restore after. Only images are preserved. |
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

* `python tools/mutate_<name>.py --dry` checks that each pattern still matches exactly once;
* `python tools/mutate_<name>.py [index ...]` runs them (minutes per mutant).

They are **tied to the code as it was** when each feature landed: a pattern that no longer matches
is reported as `BAD PATTERN` and needs updating before it is run. Never format, commit or build in
another window while one runs: it edits source files in place.

## The packaged-window harness

See [`validate/README.md`](validate/README.md).
