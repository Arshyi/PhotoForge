# Phase 15 — interoperability (0.15.0)

Phase 15 is the one in which PhotoForge stopped being a program that edits PNG, JPEG and WebP and
became one that exchanges layered and high-precision documents with the programs around it: it
reads and writes Photoshop documents, TIFF, OpenEXR and SVG, keeps the metadata a file carried,
develops camera RAW (DNG) the way the camera declares its picture, and copies to and from the
Windows clipboard. This page is the summary and the ledger of what is and is not claimed; the
increments have their own results pages, linked below, which are the evidence.

Status: **implemented and verified as listed.** Version **0.15.0**. Nothing here has been opened in
Photoshop, Lightroom, Nuke, Blender or Inkscape; see "What is not claimed".

## What 0.15.0 adds

| Increment | What | Evidence |
| --- | --- | --- |
| 15.0 | The Layers panel's edits are made by the Rust engine, one editing path (30 registered operations) | [results](phase-15-0-results.md) |
| 15.1 | Photoshop **PSD / PSB import**: layers, groups, masks, 26 blend modes, clipping masks, Curves / Levels / Invert / Exposure adjustments; a report of everything approximated or left out | [results](phase-15-1-results.md), [1b](phase-15-1b-results.md), [1c](phase-15-1c-results.md), [1d](phase-15-1d-results.md), [psd-import.md](psd-import.md) |
| 15.2 | Photoshop **PSD / PSB export**, layers kept, with a report | [results](phase-15-2-results.md), [psd-export.md](psd-export.md) |
| 15.3a | **Metadata** (EXIF, XMP, IPTC) kept with the document; the orientation tag is applied on open; three export policies (default removes place, serial numbers, maker note and embedded previews) | [results](phase-15-3a-results.md), [metadata.md](metadata.md) |
| 15.3b, 15.3c | **SVG** export of shape layers and import of a shape subset | [results](phase-15-3b-results.md), [3c](phase-15-3c-results.md), [svg.md](svg.md) |
| 15.3d | **OpenEXR** import and export, scene-linear float | [results](phase-15-3d-results.md), [openexr.md](openexr.md) |
| 15.3e | **Clipboard**: copy a layer or the merged picture, paste as a layer or a document (Windows) | [results](phase-15-3e-results.md), [clipboard.md](clipboard.md) |
| 15.5 | The fixes from an independent review of 15.0 to 15.3, and the project format version rule (the lowest version that holds the document) | [results](phase-15-5-review-results.md), [handoff](phase-15-review-handoff.md) |
| 15.6a, 15.6b | **DNG metadata** and **TIFF** import and export (8 / 16-bit, Deflate, ICC, metadata) | [6a](phase-15-6a-results.md), [6b](phase-15-6b-results.md), [tiff.md](tiff.md) |
| 15.7, 15.8 | A developed DNG is turned **upright** from its orientation tag and **cut to the picture** its camera declares (default crop inside the active area) | [7](phase-15-7-results.md), [8](phase-15-8-results.md), [raw-development.md](raw-development.md) |
| 15.9 | A RAW **region** is chosen on that picture and is turned with it | [results](phase-15-9-results.md) |
| 15.9.1 to 15.9.3 | What driving the **packaged window** found: three defects fixed, the window polished, and a local log of close requests | [9.1](phase-15-9-1-results.md), [9.2](phase-15-9-2-results.md), [9.3](phase-15-9-3-results.md) |

The project format is unchanged in kind: a project is written as version 2 unless it holds state
only Phase 15 knows (a clipped layer, one of the ten added Photoshop blend modes, a turned or cut
RAW layer), in which case it is version 3, which a 0.14.0 build refuses by name. See
[project-format.md](project-format.md).

## Verification

| Gate | Result at the release tree |
| --- | --- |
| `cargo fmt --check` | clean |
| `cargo clippy --all-targets -- -D warnings`, four feature sets (all, no GPU, no inference, no plugins) | clean |
| `cargo test --all-targets --no-fail-fast` | **1,993** / **1,986** / **1,985** / **1,954** passed, 0 failed (60 binaries) |
| `npx tsc --noEmit`, `npm run check`, `npx vitest run`, `npm run build` | 0 errors, 0 warnings (523 files); **1,389** tests in 93 files passed; build succeeds. One unhandled error, the `CommandPalette` test that throws on purpose, present since before Phase 15 |

Beyond the suites: each increment was mutation-checked (breaking the new code and watching a test
fail), the file formats were read back by programs that are not PhotoForge (Pillow, tifffile,
psd-tools, OpenEXR, resvg), and PSD, TIFF, EXR and SVG were checked against real files from other
programs' own test corpora (hundreds of them; the per-increment pages give the counts and the
ones refused).

## The packaged window

15.0 to 15.8 were verified without the packaged executable. For this release the release exe was
built and driven in a real window (real Tauri IPC with its ACL, real WebView2, real pointer events
into the page; the native file dialogs answered at the IPC call). It found three defects that no
test had seen, listed in [15.9.1](phase-15-9-1-results.md) and fixed, and the polish in
[15.9.2](phase-15-9-2-results.md). Driven and clean afterwards: opening and exporting TIFF, EXR,
SVG and PSD, the RAW region chooser with a pointer (landscape and portrait), Change Source Region,
the memory settings, and closing the window.

**Installers** (built from `main` at `8f7e32a`, installed, driven and uninstalled):

| | NSIS (per-user) | MSI |
| --- | --- | --- |
| Size | 13.1 MB | 19.1 MB |
| Install | silent, 3.2 s, no elevation | `/qn`, 49 s with the UAC prompt |
| Registered | per-user uninstall entry, Start Menu shortcut | per-machine uninstall entry |
| Installed app | opened a RAW region, no IPC errors, closed on `WM_CLOSE` | the same |
| Uninstall | silent, 5.8 s, nothing left | 22 s, nothing left |

**The MSI is registered per machine but installs into the installing user's profile**
(`%LOCALAPPDATA%\PhotoForge`): a second Windows user on the same machine would see PhotoForge
registered with no files behind it. The NSIS installer is the one to use; the MSI is kept for
anyone who needs the format and is warned about here. Neither installer is signed
(`Get-AuthenticodeSignature` reports `NotSigned`), so Windows SmartScreen will warn.

## What is not claimed

* **No file has been opened in the program on the other side.** PSD and PSB exports have not been
  opened in Photoshop; TIFF in Lightroom or Photoshop; EXR in Nuke or Blender; SVG in Inkscape; the
  clipboard in Photoshop or Office. They were read by independent readers and compared with their
  sources.
* **The clipboard was not driven in the packaged window**: the person's clipboard held text, which
  the test harness cannot preserve. It has its own real-clipboard tests.
* **One real camera file.** The RAW work was checked against one converter's Canon 5D Mark III DNG
  (with its orientation tag rewritten in copies for the eight values), not against portrait files
  from real cameras, fractional or odd crops, or other makers' DNGs.
* **Display scaling (125 / 150 / 200 %) and a second monitor** were not tried.
* **Installers**: upgrade over an older version, repair, install over a running app, and a machine
  without WebView2 (which needs the network to fetch it) were not tried.
* **A window that did not close** was seen once, in a document that was failing to render, and in
  3 of about 15 attempts when that failure was reproduced by injection, never with a healthy
  document, and not again in later runs. Its cause is not known; [15.9.3](phase-15-9-3-results.md)
  adds a local log (`close.log`) that will say what happened if it recurs.

## Known and left

* **High-dynamic-range values are clipped by most blend modes** (everything but Normal, Multiply,
  Screen, Darken, Lighten, Difference, Exclusion). A design decision about what Overlay and its
  relatives mean above white.
* **PSD**: import decodes every channel, so extra channels exceed the per-layer estimate by up to
  about 14 times; export's peak memory is about three times what is guarded; a high-precision
  document is written as 16-bit sRGB; vector shapes and fills, text and smart objects are not
  imported as such (the report names them), and layers' effects are not carried.
* **SVG**: isolated groups are written as plain `<g>`; colour names outside 40, `hsl()` and
  percentage lengths are read wrongly; `<title>` is not read back.
* **OpenEXR**: luminance / chroma pictures, deep parts, and files with a lone G channel are
  refused (with the reason shown); a multi-part file whose earlier part has a codec the reader
  lacks is refused as a whole.
* **RAW**: DefaultScale and non-square pixels are not applied; the masked photosites are not used
  to measure the black level; only DNG is read (no CR2, CR3, NEF and so on).
* **Other formats**: no CMYK, no printer proofing, no general GPU compositing, no out-of-core
  editing.
* **Interface**: the Layers panel's own controls use a different type scale from the cards.

## Licence and artifacts

No licence has been selected; all rights are reserved. The dependency added in this phase (the
`exr` crate and the four crates it brings) is listed in
[THIRD_PARTY_NOTICES.md](../THIRD_PARTY_NOTICES.md). The checksums of the release artifacts are in
`SHA256SUMS.txt`.
