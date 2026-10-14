# Phase 15.10 — the clipboard, driven in the packaged window

The 0.15.0 ledger said the clipboard had not been driven in the packaged window: the person's
clipboard held text, which the validation harness had no way to give back, and a copy would have
replaced it. The clipboard has its own tests (real-clipboard, command boundary, Pillow, interface),
but none of them ran in the packaged executable, with its own permission list, its real WebView2
and real key events. This does that, and first builds the part that makes it safe to do.

Status: **implemented and verified as listed; integrated for 0.15.1.** Developed on
`phase-15-10-clipboard-window`, from `main` (`242f6f9`). No application code changed.
Current integration checks are recorded separately in [0.15.1](release-0-15-1.md).

## What it adds

* **`tools/clipboard_backup.ps1`, rewritten.** It used to save one PNG and could not give text back.
  It now saves **every format that lives in global memory**, in the clipboard's order (text in all
  its forms, bitmaps, HTML, a file list, registered formats such as `PNG`), and puts them back. What
  is not content is left out and said so: OLE bookkeeping of the program that copied
  (`DataObject`, `Ole Private Data`, an address in that program) and forms Windows makes from others
  (`CF_BITMAP` from a DIB). **What cannot be saved is a refusal**, not a silent loss: exit 3 unless
  `-AllowLoss`. Actions: `save`, `restore`, `verify` (exit 0 only if identical), `list`, `selftest`.
* **`tools/validate/clipboard_window.ps1`**: the scenario, 14 checks (below).
* **`tools/validate/clipboard_probe.py`**: reads and writes the clipboard from outside PhotoForge
  (Win32 through `ctypes`, Pillow for pixels): what a copy put there, and what "another program"
  puts there to be pasted. Its own checker was shown to reject a wrong picture (a 40 × 30 picture
  on the clipboard checked against the 80 × 48 expectation: three problems, exit 1).
* **`tools/validate/make_pictures.mjs`**: the two pictures (no dependencies).
* **`cdp.mjs`**: `chord` (a key with modifiers, as a real key event into the page) and `insert`.

## The scenario

Run against the 0.15.0 executable, in its own profile, with real key events into the page. The
clipboard is read and written by a separate process.

| # | What | Result |
| --- | --- | --- |
| 1 | A 96 × 64 PNG with a transparent 8-pixel border and a half-transparent block opens | ok |
| 2 | Text is on the clipboard; **Ctrl+C**: the clipboard changed | ok |
| 3 | …its `PNG` form and `CF_DIBV5` are the layer **trimmed to 80 × 48, pixel for pixel**, alpha included (the half-transparent block at 128), with `CanUploadToCloudClipboard` = 0 | ok |
| 4 | …the text that was there is **gone**, not kept beside the picture | ok |
| 5 | …the notification says "Copied the layer (80 × 48) to the clipboard." | ok |
| 6 | **Ctrl+Shift+C**: the whole 96 × 64 canvas, pixel for pixel, both forms | ok |
| 7 | …and its notification | ok |
| 8 | A 40 × 30 picture put there by another program (`PNG` with alpha and a 24-bit `CF_DIB`); **Ctrl+V**: "Pasted a 40 × 30 picture … as a new layer" | ok |
| 9 | …that layer, copied back out with Ctrl+C, is **that picture exactly** | ok |
| 10 | A clipboard with only a 24-bit `CF_DIB`; Ctrl+V reads it | ok |
| 11 | A clipboard with only text; Ctrl+V: "The clipboard does not hold a picture." and nothing placed | ok |
| 12 | The palette (Ctrl+Shift+P, "New document from clipboard", Enter): opened as a new document | ok |
| 13 | **No IPC command failed** except the one deliberate refusal in 11 (`paste_clipboard_image`, once) | ok |
| 14 | The person's clipboard is back, **identical** (`clipboard_backup.ps1 verify`) | ok |

14 of 14, 19 s. No defect was found in the application: the clipboard works in the packaged window
as the design document says.

## How the person's clipboard was kept

* `selftest` first: the real clipboard saved, a canary with Unicode text, a DIB, `PNG` and a private
  registered format put on it, replaced by other text, put back from disk, compared byte for byte
  (**identical**), then the person's clipboard put back and compared (**identical**).
* Every run saves it first and restores it in a `finally`, then runs `verify`; the scenario's last
  check is that verification.
* **One slip, recovered.** While exploring by hand before the scenario existed I saved the clipboard,
  pressed Ctrl+C in the window and did not restore: for a few minutes the clipboard held the test
  picture and the person's was only in the backup directory. It was restored from that backup and
  verified (identical, same sizes) before anything else was run. The scenario cannot do this: the
  restore is in a `finally`.

## What the tooling found about itself

* The person's real clipboard (a screenshot copied from a program that sets OLE data) carried two
  OLE bookkeeping formats, `DataObject` and `Ole Private Data`. The first version of the backup
  saved them like any other format (8 and 312 bytes); put back from another process they would be
  an address into a process that may be gone. They are now recognised, listed as not content, and
  not restored; the picture itself (`PNG`, `CF_DIB`, `CF_DIBV5`, the cloud and history flags) is.
* A copy at the Windows level makes the clipboard also offer `CF_DIB` and `CF_BITMAP`
  (synthesized from `CF_DIBV5`). The scenario's reader decodes `CF_DIBV5` itself, from its header's
  masks, rather than through a library that may take the synthesized 24-bit form.
* The deliberate paste of text is logged by the window as a failed IPC call; the scenario names it
  and allows exactly one.

## What was not verified

* **Another program on the other end.** Photoshop, Office, a browser and Paint.NET were not run
  against it; the forms are the ones they document reading. The reader here is Pillow and Win32.
* **Keys while the caret is in a text field**, in the real window: covered by component tests, not
  driven here.
* **A clipboard another program holds open**, in the real window (unit and real-clipboard tests
  cover it).
* **Large pictures** (a 24-megapixel layer): the scenario uses small pictures; the size limits are
  tested at the Rust level.
* **A clipboard with formats the backup cannot restore**: the refusal was reasoned, and exercised
  only for `CF_BITMAP` made from a DIB (restored by Windows). It was not run against a clipboard
  holding, say, a metafile.
* The scenario passed on its second run (the first said 12 of 13: my check had not allowed for the
  deliberate refusal in 11 being logged as a failed call). It is not a soak test.

## Known and left

* The 0.15.0 mutation harness for the clipboard (`tools/mutate_clip.py`) has four patterns that no
  longer match; unchanged here.
