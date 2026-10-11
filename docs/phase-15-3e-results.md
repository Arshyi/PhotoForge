# Phase 15.3e — the clipboard

The fifth and last part of 15.3 (metadata, SVG export, SVG import, OpenEXR, **clipboard**).
PhotoForge copies pictures to the Windows clipboard and pastes pictures from it. What it puts
there and reads from there, and why, are in [clipboard.md](clipboard.md); this is what was done
and what was checked.

Status: **implemented and verified as listed; committed on its branch, not pushed, not released.**
The version is still 0.14.0. On branch `phase-15-3e-clipboard`, on top of `d910565` (15.3d).

## Completed

* **`src-tauri/src/clipboard/`** (new): `dib.rs` (a pure codec for Windows bitmaps),
  `win.rs` (the Win32 calls, 158 lines, all the `unsafe`), `mod.rs` (the PNG codec, the policy: write
  `PNG` + `CF_DIBV5`, read `PNG` then `CF_DIBV5` then `CF_DIB`).
* **Commands** `copy_to_clipboard`, `paste_clipboard_image`, `clipboard_has_image`,
  `open_clipboard_image`.
* **The interface**: Ctrl+C (copy layer), Ctrl+Shift+C (copy merged), Ctrl+V (paste as a new layer,
  or open as a document when none is open), and Copy layer / Copy merged picture / Paste picture /
  New document from clipboard in the command palette. A text field keeps its own Ctrl+C / Ctrl+V.
* **No new dependency**: four more feature flags of the one `windows` crate PhotoForge already
  depends on. `Cargo.lock` is unchanged.

## Findings, and what was done about them

| Found by | Finding |
| --- | --- |
| Reading Windows' own synthesised `CF_DIBV5` (made by System.Windows.Forms) | It repeats the three colour masks as a table after a V5 header that already has them; read as the header says, the picture came out three pixels shifted, with the masks as its first pixels. The table is recognised (those three DWORDs are the header's masks) and skipped |
| Asking Windows' own reader about a `CF_DIBV5` written **alone** | A DIBV5 *with* that table is read shifted by Windows itself; one without it is read exactly. So PhotoForge writes none. (With a `PNG` form beside it, Windows' reader would have used the PNG and the layout would never have been tested: found by taking the PNG away) |
| A mutation check | A copy that did not empty the clipboard first left earlier formats beside the picture (text, a stale bitmap); the next-copy test read through PhotoForge's own reader and did not see it. A test now puts text there first and asks whether it is still there |
| A mutation check | "Another program holds the clipboard" could not be tested with a program that opens it with no window: Windows does not exclude that at all. The test uses one with a window. The finding led to one use of the clipboard at a time inside PhotoForge, because two of its own commands could otherwise interleave |
| The unit tests | A bounds function that computed `right - left` for an empty selection overflowed (a debug panic); it is computed only when there is something |

## Verification

| Gate | Result |
| --- | --- |
| `cargo fmt --check` | clean |
| `cargo clippy --all-targets --all-features -- -D warnings`, and the same with no GPU / no inference / no plugins | clean (all four) |
| `cargo test --all-targets --all-features --no-fail-fast` | **1,783 passed**, 0 failed, 52 binaries (15.3d ended at 1,745 and 50) |
| No GPU / no inference / no plugins | **1,776 / 1,775 / 1,744 passed**, 0 failed |
| `npx tsc --noEmit`, `npm run check` | clean; 520 files, 0 errors, 0 warnings |
| `npx vitest run --maxWorkers=3` | 91 files, **1,350 passed**, 0 failed (15.3d ended at 1,331) |
| `npm run build` | built |
| Real clipboard (`PHOTOFORGE_CLIPBOARD_TESTS=1`) | 12 + 3 passed |

The tree was scanned for control characters (none). `CommandPalette.test.ts` still reports the
one unhandled error it reported before this phase (see the 15.3d results). The real-clipboard
tests are not part of the default gate (they replace the clipboard); they were run separately, and
the clipboard that was there (an image) was backed up first and put back afterwards.

New tests: 18 in the clipboard module (bitmap depths and layouts, the header against the
documentation, a mask table, an unfilled alpha, lying sizes, bad masks, truncation and changed-byte
sweeps, the PNG codec), 5 for the selection and the trim, 3 through the command boundary
(`tests/clipboard_ipc.rs`; the refusals run always), 12 against the real clipboard
(`tests/clipboard_real.rs`; only with `PHOTOFORGE_CLIPBOARD_TESTS=1`, which **replaces the
clipboard**, so they are off by default), 5 for the shortcuts, 4 for the helpers, 10 for the
interface.

**Against independent readers and writers** (details in [clipboard.md](clipboard.md)): Windows'
own reader through System.Windows.Forms and System.Drawing, exactly, for what PhotoForge writes
(`PNG` + `CF_DIBV5`, and `CF_DIBV5` alone); PhotoForge's reader on a picture System.Windows.Forms
copied; Pillow's `ImageGrab` and raw reads for both forms and for the `CF_DIBV5` alone, alpha
included; and Pillow-written `CF_DIB` (24-bit, 32-bit), `PNG`, and `PNG` beside a different DIB, read
by PhotoForge, `PNG` preferred. No difference on any.

**Mutation checks**: 15 breakages, all caught after two survivors were turned into tests.

## What was not verified

* **Photoshop, Office, a browser, Paint.NET** as the program on the other side. System.Windows.Forms
  and Pillow are two independent readers and one independent writer; they are not the programs
  people use.
* **Programs that put premultiplied pixels in a `CF_DIBV5`**: read as straight alpha, so their soft
  edges would paste darker; the `PNG` form, read first, is unambiguous.
* A clipboard held by a program that renders its formats **on request** and slowly.
* The interface in a real window (component tests against a mocked command boundary).
* Pasting a very large picture (the bounds are tested; a 100-megapixel paste was not timed).

## Not done

Cut; copy of several layers at once; a layer copied with its group's opacity; pasting a list of
files (`CF_HDROP`); pasting at the centre of the view or at the cursor (a pasted picture goes where
a placed image goes); colour profiles on the clipboard; copying a layer *as a layer* (text, shape,
smart object, adjustment) between documents.

## What 15.3 is, now

Metadata policy and report (15.3a), SVG export (15.3b), SVG import (15.3c), OpenEXR import and
export (15.3d), the clipboard (15.3e): all committed on their branches, each with its own results
page. Still not done from the interoperability list: TIFF, and DNG metadata (the EXIF of a RAW
source is not read yet); and the files written have not been opened in the applications that use
them (Photoshop, Inkscape, Nuke, Blender).
