# Phase 15.3d — OpenEXR import and export

The fourth part of 15.3 (metadata, SVG export, SVG import, **OpenEXR**, clipboard). An `.exr` opens
as a new, unsaved high-precision document with a report of what the file held that PhotoForge has
no place for, and the document is written, composited, as an OpenEXR file. What it reads and
writes, and the conventions converted each way, are in [openexr.md](openexr.md); this is what was
done and what was checked.

Status: **implemented and verified as listed; committed on its branch, not pushed, not released.**
The version is still 0.14.0. On branch `phase-15-3d-exr`, on top of `9d519fd` (15.3c).

## Completed

* **`src-tauri/src/openexr/`** (new): `import.rs` (header-first admission, channel sets, the
  conventions), `export.rs` (the composite as scene-linear float), `colour.rs` (primaries
  matrices), `testing.rs` (writes test files; test-only).
* **`import_exr_document` and `export_exr_document` commands**, modelled on the Photoshop and SVG
  ones: staged before the session changes, no project path (Save asks where to put the project),
  the export gate, plug-in availability, a destination the document reads from refused, a complete
  new file or the old one left.
* **The interface**: `.exr` in the Open dialog and by drop; *OpenEXR* in the Export dialog; the
  shared report dialog reads "OpenEXR import" / "OpenEXR export".
* **`AppError::ForeignFile`**: a problem with the content of a file the person chose is said as
  itself. The SVG importer's complaints, which came out as "PhotoForge could not process this edit
  pipeline: …", use it too.
* **Dependency**: `exr = "=1.74.0"`, default features off. Four packages new to the build
  ([openexr.md](openexr.md#dependency)); notices in `THIRD_PARTY_NOTICES.md`.

## Defects found by checking, and fixed

| Found by | Defect |
| --- | --- |
| The real files, against the reference reader | `ScanLines/Carrots.exr` (ACES 2065-1 colour with an alpha channel nobody filled) opened as a **completely transparent** picture: every pixel was "light with no alpha". An alpha that is zero at every pixel over colour that is there is now ignored, and said |
| The first compile of the primaries code | A primary was divided by its own `y`, which refused ACES AP0 (its blue has a negative `y`) as not a colour space; primaries are now (x, y, z) vectors, as OpenEXR's own `RGBtoXYZ` has them |
| A mutation check | A half-float clamp that was never made, hidden because the importer turns an infinity into 65,504; the test now reads the written samples raw |
| clippy | Two style lints (field assignment after `default()`, a negated float comparison), fixed |
| The SVG importer's wording (found while choosing the error type for this one) | Its refusals were prefixed "PhotoForge could not process this edit pipeline"; they now use the new `ForeignFile` error |

An equivalent mutant is recorded rather than "fixed": the lookup name a set without alpha asks the
reader for can be changed to `A` with no observable effect, because the conversion ignores the
alpha of a set that has none. It stays as belt and braces.

## Verification

| Gate | Result |
| --- | --- |
| `cargo fmt --check` | clean |
| `cargo clippy --all-targets --all-features -- -D warnings`, and the same with no GPU / no inference / no plugins | clean (all four) |
| `cargo test --all-targets --all-features --no-fail-fast` | **1,745 passed**, 0 failed, 50 binaries (15.3c ended at 1,676 and 48) |
| No GPU / no inference / no plugins | **1,738 / 1,737 / 1,706 passed**, 0 failed |
| `npx tsc --noEmit`, `npm run check` | clean; 517 files, 0 errors, 0 warnings |
| `npx vitest run --maxWorkers=3` | 89 files, **1,331 passed**, 0 failed (15.3c ended at 1,315) |
| `npm run build` | built |

The tree was scanned for control characters (none). `CommandPalette.test.ts` reports one
unhandled error in its "survives a command that throws" test with all 11 of its tests passing; it
does the same at `9d519fd`, before this change, and neither it nor its component is touched here.

New tests: 62 in the OpenEXR module (7 for the primaries against published matrices, 43 for the
importer, 12 for the exporter), 6 through the command boundary, 16 in the interface (5 helpers, 11
for the dialogs), and `tests/exr_real_files.rs` (below).

**Against the OpenEXR reference implementation**: 62 files from the OpenEXR project's own
`openexr-images` set, imported and exported by PhotoForge and read again by OpenEXR 3.5.2 (Python)
with the same conventions applied independently:

| | |
| --- | --- |
| Imported and exported | **52 of 62** |
| Refused, each for a stated reason | 10 (6 luminance/chroma, 3 deep, 1 with only a `G` channel) |
| Largest imported difference | 3.6 × 10⁻⁷ relative, on the XYZ-primaries file |
| Exported single-precision file against the composite | bit for bit, every file |
| Exported half-precision file | within half-float precision (largest 4.9 × 10⁻⁴) |

**Mutation checks**: 27 breakages, all caught after the one survivor was fixed (above).

## What was not verified

* The written files **in Nuke, Blender, After Effects or Photoshop**. They have been read by the
  OpenEXR reference implementation and by this program.
* The interface in a real window (component tests against a mocked command boundary only).
* Files larger than the samples (the largest was 2,048 × 1,556).
* **Luminance/chroma files** (6 of the 62 samples), **DWAA/DWAB/HTJ2K**, and **deep data** are
  refused, by name; none is read.

## Next

15.3e: the clipboard (Windows-only; tested against the real clipboard).
