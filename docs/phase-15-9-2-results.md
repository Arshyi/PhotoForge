# Phase 15.9.2 — the window polish the packaged run asked for

[15.9.1](phase-15-9-1-results.md) listed what the packaged window showed that was not a defect in
a RAW or TIFF path but was plainly wrong on the screen. This phase fixes those, each checked in the
packaged window with real input.

Status: **implemented and verified as listed; not committed.** On branch
`phase-15-9-2-window-polish`, from `phase-15-9-1-validation-fixes` (`9f865e9`, PR #6).

## Completed

* **Overlapping cards in the right-hand panel.** *Color and precision* and *Metadata* were drawn
  on top of each other. The panel was a grid with three explicit rows (`auto auto minmax(0, 1fr)`)
  from when it had three children; two cards were added between them, so the `1fr` row, which is
  0px when the panel's height is not definite, landed on the first card, and the second card sat
  in the next row at the same place. The panel is now a column that scrolls as one
  (`aside` in `styles.css`), so it does not depend on how many cards it has. Measured in the
  window: the cards are at 190 and 236 px, 38 px tall each, where they were both at 190.
* **The *Text, shapes and smart objects* panel was unstyled.** It had no class and was drawn with
  the browser's controls (a white select, default buttons). It has the same card, label and control
  styling as the others (`.semantic-tools`); the cards' summaries, paragraphs, labels and selects
  are styled too (`.raw-card`).
* **Settings was drawn past the window's edge** when Change Source Region was in the toolbar: the
  toolbar was 29px wider than a 1,360px window. The brand no longer has a fixed 205px minimum and
  the tool buttons are 4px tighter in padding and 2px in gap. Measured: Settings ends at 1,349px of
  1,360 with the button present, and the header is 1,360px wide (it was 1,389).
* **The Memory settings forgot what was chosen.** After applying a manual 0.5 GB budget (which waits
  for the next document) the radio showed *Automatic* again. The status now carries the saved
  mode (`savedMode`, from `resource_status`), and the settings show it. Test: after saving a lower
  manual budget the radio is *Manual* and the figure is 2.
* **The drop hint** named four of the formats that open (PNG, JPEG, WebP, Photoshop); it now reads "Drop a PNG, JPEG,
  WebP, TIFF, DNG, EXR, SVG or Photoshop file here".

## Verification

| Gate | Result |
| --- | --- |
| `cargo fmt --check` | clean |
| `cargo clippy --all-targets -- -D warnings`, four feature sets | clean, all four (all features, no GPU, no inference, no plugins) |
| `cargo test --all-targets --no-fail-fast` | **1,989** / **1,982** / **1,981** / **1,950** passed, 0 failed (60 binaries), same four feature sets (unchanged from 15.9.1; the one Rust change is a field on a status struct) |
| `npx tsc --noEmit`, `npm run check`, `npx vitest run`, `npm run build` | 0 errors, 0 warnings (523 files); **1,388** tests in 93 files passed (one assertion added to an existing test); build succeeds. One unhandled error, the `CommandPalette` test that throws on purpose, unchanged |
| Packaged window | the cards are separate, styled and scroll as one; the drop hint is the new text; with a RAW region open Settings is inside the window; Memory shows *Manual* and 0.5 after the saved choice |

## What was not done

* **The one-off window that did not close** (see 15.9.1). It was investigated: with the render
  commands made to fail it did not close on `WM_CLOSE` in 3 of 15 attempts across two batches
  (and twice in the real broken state), never with a healthy document in about ten, and then
  0 of 10 in later batches with the same steps; no destroy call and no prompt was seen in those
  cases, so the page did not act on the request. It is **not reproduced on demand and its cause is
  not known.** A long idle (120 s) with a document open did not trigger it.
* **The layer panel's own controls** (the Layers header buttons, the blend select) use a different
  scale of type from the cards; left.
* Nothing at other widths or DPI settings was checked.
