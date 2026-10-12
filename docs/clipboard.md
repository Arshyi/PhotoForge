# The clipboard

PhotoForge copies pictures to the Windows clipboard and pastes pictures from it. Windows only.

## What the person can do

| Command | Keys | What it does |
| --- | --- | --- |
| **Copy layer** | Ctrl+C | Puts the active layer on the clipboard as a picture: as it looks on the canvas (its transform, mask and adjustments applied), **trimmed to where it has anything**. With a selection, only the selected part, cut to the selection's bounds |
| **Copy merged picture** | Ctrl+Shift+C | Puts everything visible, with the document's own edits (crop, rotation, adjustments), on the clipboard. With a selection, only the selected part |
| **Paste picture** | Ctrl+V | Places the clipboard picture as a **new layer**. With no document open, it opens the picture as a new document instead |
| **New document from clipboard** | (command palette) | Opens the clipboard picture as a new, unsaved document, even when one is open |

Ctrl+C and Ctrl+V belong to a text field while the caret is in one (a layer name, a number, the
command palette's search). A partly selected pixel is copied partly transparent.

The document is never changed by a copy. A paste is one undoable step, like placing an image.

## What goes on the clipboard

One picture, written in the two forms programs actually read:

* **`PNG`** (the registered clipboard format of that name): lossless, with a straight alpha
  channel. Photoshop, Office, browsers, GIMP and Paint.NET read this first.
* **`CF_DIBV5`**: a 32-bit bitmap with colour masks and an alpha mask, straight alpha, tagged
  sRGB. Windows synthesises `CF_DIB` and `CF_BITMAP` from it for programs that ask for those.

The clipboard holds **8-bit sRGB**. A high-precision document is rendered and encoded for it: values
brighter than white are clipped there and nowhere else, and the notification says so when it
happened. (To keep them, export an OpenEXR file.)

The clipboard is replaced whole. PhotoForge never monitors it, never reads it except when asked to
paste, and never writes to it except when asked to copy.

**Everything is made before the clipboard is touched.** The picture is encoded and its memory
allocated first; only then is the clipboard emptied and filled. Running out of memory costs the
copy, not what was on the clipboard.

**It is marked as not for the cloud clipboard.** A copy carries `CanUploadToCloudClipboard` = 0.
When the person has Windows' clipboard sync turned on, Windows honours it and does not upload
the picture: an editor that sends nothing anywhere does not have the system send its pictures.
The picture still goes into the local clipboard history, which is the person's own. (A program
that wants a picture to roam can be given one by saving a file.)

Within PhotoForge one use of the clipboard happens at a time (Windows does not exclude a second
program that opens the clipboard with no window of its own, so a copy and a paste of PhotoForge's
could otherwise interleave). A program that opens it with a window, as most do, is excluded by
Windows, and PhotoForge waits about 0.3 seconds for it and then says the clipboard is in use.

## What is read from it

In order: `PNG`, then `CF_DIBV5`, then `CF_DIB`; the first that reads is used.

| Form | Read |
| --- | --- |
| `PNG` | Any PNG; converted to 8-bit RGBA. Embedded colour profiles are not applied |
| `CF_DIBV5`, `CF_DIB` | 1, 4, 8 (with palette), 16, 24 and 32 bits per pixel; uncompressed or with colour masks; bottom-up or top-down; in an info, V4 or V5 header |
| Not read | RLE-compressed bitmaps, JPEG or PNG payloads inside a DIB, OS/2 headers: each named. A file list (`CF_HDROP`), text and anything that is not a picture say "The clipboard does not hold a picture" |

Two behaviours worth knowing:

* **An alpha channel that is zero at every pixel is ignored.** A great many programs write a 32-bit
  bitmap and leave the fourth byte zero; taken at its word the picture would paste as invisible.
  (The same rule as for OpenEXR files.) A 32-bit bitmap with no alpha mask is opaque.
* **A colour table before the pixels of a 16-, 24- or 32-bit bitmap is skipped** (`biClrUsed`
  entries; some programs write one to suggest colours to a display with few). It used to be read
  as pixels, which shifted the picture.
* **Windows' own synthesised `CF_DIBV5` repeats the three colour masks** as a table between the
  header and the pixels, which the V5 header does not call for. It is told by those three
  DWORDs being the header's masks, and skipped. (PhotoForge writes the layout the header
  describes, with no table; Windows' own reader was asked and reads it correctly. A table there
  makes Windows read the picture shifted by three pixels.)

## Bounds

A clipboard is another program's data. Nothing is allocated from a size it claims: the bytes under a
form are capped at 1 GiB; a PNG's header and a bitmap's header are checked against the working
pixel limit before a pixel buffer exists; every length is checked against the data that is there;
and the clipboard is held open only while bytes are copied (and tried again for about 0.3 seconds
if another program holds it, then reported as in use). A paste copies the bytes out and **lets the
clipboard go before decoding**: a large picture takes a while to decode, and no other program
should wait for that.

## Dependency

None added. The clipboard is read and written with the `windows` crate PhotoForge already depends
on at exactly one version, with four more feature flags (`Win32_Foundation`,
`Win32_System_DataExchange`, `Win32_System_Memory`, `Win32_System_Ole`). `Cargo.lock` is unchanged.
The `unsafe` is in one file (`src-tauri/src/clipboard/win.rs`, 158 lines), each block with the
reason it is sound.

## How it was checked

* **Unit tests** (18 in the module, 5 for the cutting): every bitmap depth and layout, built by
  hand against the documented structures; the header of what is written against the Windows
  documentation field by field; a mask table; an alpha channel nobody filled; sizes that lie;
  masks that make no sense; **every truncation and a changed byte of three samples, none of which
  panics**; the selection (coverage, partial alpha, empty, wrong size) and the trim.
* **Against the real clipboard** (`tests/clipboard_real.rs`; they replace the clipboard, so they
  run only with `PHOTOFORGE_CLIPBOARD_TESTS=1`): a picture written is the picture read, alpha
  included; the next copy replaces the last whole; **a copy replaces what was there, and does
  not add to it** (text put there first is gone afterwards); text is not a picture; a picture
  another program copied (System.Windows.Forms puts a device-dependent bitmap there) reads with
  exact colours; what is written is read by another program (System.Windows.Forms again),
  exactly; `CF_DIBV5` **alone** (so that no `PNG` form hides its layout) is read by Windows' own
  reader exactly; and **a clipboard another program (one with a window) holds** is reported as in
  use within about 0.3 seconds, not waited on, and works again when it lets go.
* **Through the command boundary** (`tests/clipboard_ipc.rs`): the refusals (a layer that does not
  exist, a selection of another size, an empty selection, an invalid document) run always and never
  touch the clipboard; copy a layer trimmed, paste it as a layer and as a document, copy through a
  selection, copy the merged picture, run only with the variable set.
* **Against Pillow** (`ImageGrab` and raw reads, from outside): the `PNG` form decodes to exactly
  the picture; Pillow reads the `CF_DIBV5` written alone as exactly the picture, alpha included;
  and Pillow-written `CF_DIB` (24-bit and 32-bit), `PNG`, and `PNG` beside a different `CF_DIB`
  are read as the picture, `PNG` preferred.
* **In the interface** (10 + 5 + 4): the keys, the palette, what is sent (the layer, the selection,
  the edits), the sentences, a failure, a text field keeping its own keys.
* **Mutation checks**: 15 breakages (the alpha dropped, rows not flipped, the mask table not
  skipped, an unfilled alpha obeyed, each size limit removed, the palette ignored, the selection's
  alpha or bounds ignored, a layer not trimmed, the PNG or the DIBV5 not written, PNG ignored on
  read, the clipboard not emptied, "has a picture" always true) are each caught. Two survived the
  first run and led to the "replaces, does not add" and the "held by another program" tests above.

## Not checked

* **Other programs' behaviour** beyond System.Windows.Forms and Pillow: Photoshop, Office, a
  browser and Paint.NET were not run against it. The `PNG` and `CF_DIBV5` forms are the ones they
  document reading.
* **Premultiplied alpha**: some programs put premultiplied pixels in a `CF_DIBV5`. Windows does
  not say which a `CF_DIBV5` holds, and PhotoForge reads it as straight alpha, which Pillow and
  Windows' own reader agree with; a program that does otherwise would paste with dark fringes on
  soft edges, and the `PNG` form (read first) is unambiguous.
* A clipboard held by a program that **delays rendering** (formats promised and produced on
  request) was not tried with a program that does it slowly.
* The interface in a real window.

## Not done

Cut (it would clear pixels, which needs the pixel editor); copying several layers at once; copying
a layer *with* its group's opacity; pasting a list of files (`CF_HDROP`) as layers; pasting at the
cursor, or at the centre of the view (a pasted picture goes where a placed image goes); keeping
colour profiles; internal layer copy with full fidelity between documents (smart objects, text,
shapes, adjustments); the clipboard's history.
