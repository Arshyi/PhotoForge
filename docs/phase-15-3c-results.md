# Phase 15.3c — SVG import of a bounded subset

The third part of 15.3. An `.svg` opens as a new, unsaved document of shape layers, with a report
of what the subset did not cover. What it reads, what it leaves out, and why it cannot reach the
network or run anything are in [svg.md](svg.md#reading-svg); this is what was done and what was
checked.

Status: **implemented and verified as listed; not committed, not released.** The version is still
0.14.0. On branch `phase-15-3c-svg-import`, on top of `8f479c9` (15.3b).

## Completed

* **`src-tauri/src/svg/import.rs`** (new): the reader. A parser that refuses a DTD and stops at a
  node limit (`roxmltree`, see below); PhotoForge's own reading of lengths, colours, styles
  (attributes and `style`, inherited as CSS says), transform lists, and path data (every command,
  relative and absolute, arcs as cubics in quarter turns).
* **A transform is baked only when it has to be.** A move or an axis-aligned scale keeps a
  rectangle a rectangle and an ellipse an ellipse; a rotation or skew turns the shape into its
  outline. A stroke is scaled with the transform's mean scale, and said so when the scale is
  uneven.
* **`import_svg_document` command**: modelled on the Photoshop import — staged before the session
  changes, no project path (Save asks where to put the project), `prepare_project` for the
  render.
* **The interface**: `.svg` in the Open dialog and by drop; the report dialog reads "SVG import",
  counts shapes, and titles its approximated section "Imported, but not exactly as in the SVG".
* **Dependency**: `roxmltree = "0.20"` in `Cargo.toml`. It was already in the lockfile (under the
  font-configuration parser Linux builds use); on this platform it is new to the build. MIT OR
  Apache-2.0, `#![forbid(unsafe_code)]`, no dependencies, DTD refused by default, a node limit.
  `THIRD_PARTY_NOTICES.md` carries its notice. It is the only addition; everything SVG-specific
  is PhotoForge's code.

## Against real files

230 icons were downloaded into scratch (never committed): 70 from Feather (MIT), 90 from Bootstrap
Icons (MIT), 70 from Simple Icons (CC0), each given a 256 × 256 size, imported, rendered by
PhotoForge, and rendered by `resvg`:

| | |
| --- | --- |
| Imported | **230 of 230**, none refused |
| Reported as approximated or left out | none |
| Difference from `resvg`, mean per file (0–255) | median **0.22**, 90th percentile 0.86, worst 2.20 |
| Files over 1.0 / over 3.0 | 17 / 0 |

The worst file (Simple Icons' EDEKA) stacks two nearly identical shapes in one path. Investigating
it showed that PhotoForge's rasteriser, which sums signed partial coverage and clamps, draws such an
edge pixel fuller than the true union (0.73 + 0.72 reads as 1.0 where it should be 0.73), while
`resvg` is right to a quarter. This is how PhotoForge has always drawn a path, not something the
importer does; it is recorded as a known limitation in `docs/svg.md` and left alone, because a
true non-zero union needs per-sample winding, which is a rasteriser project of its own. The
rasteriser is exact for single contours, which a test on fractional edges confirmed.

## Defects found by checking, and fixed

Every test passed on its first run, so these were found by reading the draft against hostile and
unusual input, before it was in the tree, and not by a failure:

| Found by | Defect |
| --- | --- |
| Review against hostile input | A polyline of a million points would have been allocated and refused only afterwards; the point list is now capped at the model's limit as it is read |
| Review against the path grammar | A drawing command after `Z` must start from the start of the closed subpath it closed; it is given an explicit move |
| The real-file comparison | Nothing in the importer: the one visibly different icon was PhotoForge's rasteriser (above) |

## Verification

| Gate | Result |
| --- | --- |
| `cargo fmt --check` | clean |
| `cargo clippy --all-targets --all-features -- -D warnings`, and the same with no GPU / no inference / no plugins | clean (all four) |
| `cargo test --all-targets --all-features --no-fail-fast` | **1,676 passed**, 0 failed, 48 binaries (15.3b ended at 1,654) |
| No GPU / no inference / no plugins | **1,669 / 1,668 / 1,637 passed**, 0 failed |
| `npx tsc --noEmit`, `npm run check` | clean; 514 files, 0 errors, 0 warnings |
| `npx vitest run --maxWorkers=4` | 87 files, **1,315 passed**, 0 failed (15.3b ended at 1,308) |
| `npm run build` | built |

The tree was scanned for control characters (none).

New tests: 20 in the importer (shapes and colours in linear light; view box and units; a move or
scale keeps a rectangle, a rotation bakes; style and inheritance; what the subset leaves out is
named and never fetched or run; a DTD and an entity bomb; size and shape limits; nesting; far and
long shapes; a view box of another shape; child-before-group transforms and stroke scaling;
`none` and defaults; the path grammar; arcs; an error mid-path; colour forms; transform lists; the
exporter's own output read back and rendered against the original; every truncation and changed
byte; a file too large or not text), 2 through the command boundary, 5 in the interface (the Open
dialog, the command and the report, a clean import, left-out and approximated lines, a refusal).

**Mutation checks**, each broken then restored: a child transform applied after its group's,
colours not decoded to linear light, a DTD allowed, `fill="none"` read as black, group opacity
dropped, an arc's sweep flag inverted, the view box not centred, stroke widths not scaled, the
shape limit off, a path error not reported. All ten were caught.

## What was not verified

* SVGs that are **not icons** (maps, charts, drawings with gradients and filters): they are
  expected to report heavily, which is what the report is for.
* The interface in a real window (component tests against a mocked command boundary, and the
  command by tests of the real command).
* **Text, `<use>`, gradients as themselves, style sheets, dashes, clip paths and masks** are not
  read; each is named when it is met.

## Next

15.3d: OpenEXR needs a dependency decision (below); the clipboard is Windows-only and is tested
against the real clipboard.

**Decision for review, OpenEXR.** The choice is between the `exr` crate (BSD-3-Clause, pure Rust,
no unsafe code, five small dependencies, used by the `image` crate and others, reads and writes
every compression Photoshop and Blender produce) and an in-house reader (uncompressed, RLE and ZIP
only: a day's work, but PIZ, which cameras and Nuke write by default, would be refused). I would
take the crate. It is not in this machine's registry, so fetching it needs a network download; I
have not done that.
