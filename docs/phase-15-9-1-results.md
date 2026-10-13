# Phase 15.9.1 — what driving the packaged window found

15.6 to 15.9 were verified by unit tests, command-boundary tests, real-file oracles and mutation
runs, but never in the packaged executable. This pass built the release exe from `main` (`0a836e8`)
and drove it: a real window, the real Tauri IPC with its ACL, the real WebView2, real pointer
events into the page. It found **three defects that no test had seen**, one of them in 15.9 itself,
and they are fixed here.

Status: **implemented and verified as listed; not committed.** On branch
`phase-15-9-1-validation-fixes`, from `main` (`0a836e8`).

## How it was driven

* The release exe (`cargo build --release`, scratch target directory), launched with its own
  `LOCALAPPDATA` (settings, recovery, caches) and its own WebView2 profile, so the person's
  PhotoForge data was not touched; the only file the app wrote there was its `resources.json`.
* The page was driven over the DevTools protocol on a local-only port: real
  `Input.dispatchMouseEvent` pointer events into the page (never to the desktop), screenshots of
  the page, and a log of every IPC command with its outcome. The native file dialogs, which cannot
  be automated, were answered at the IPC call (`plugin:dialog|open` and `|save` return a path);
  everything else (buttons, dialogs, the chooser, the commands, ACL) was the app's own.
* Inputs: the Canon sample and a copy with its orientation tag rewritten to 6 (a portrait).

## What worked in the packaged window

* The window opens, the Memory settings apply a manual budget (0.5 GB), and the oversized dialog
  reports the **picture's** size, 5,760 × 3,840 (not the sensor's 5,920 × 3,950), with a region
  chooser whose preview has no masked border.
* A region drawn with a real pointer drag (move, resize) opens at the size chosen; the Camera RAW
  card shows the sensor as it is. A **portrait** copy's chooser is portrait, and its region opens
  upright and exports as a 1,971 × 2,956 16-bit TIFF that an independent reader (tifffile) reads as
  RGB, Deflate, orientation 1, make and model kept.
* Change Source Region on that portrait region starts the chooser at exactly the rectangle that was
  chosen (the layer records photosites; the backend maps them back onto the picture).
* The TIFF opens through the real Open button with its report ("Nothing was approximated or left
  out"; 16-bit with an sRGB profile).
* The window closes on `WM_CLOSE` (the Phase 14 defect did not return) with no document, a RAW
  region, and a TIFF open.

## Defects found, and fixed

1. **A portrait region could not be used (15.9).** The region opened, and then every thumbnail,
   analysis and histogram of the document was refused: *"a RAW region layer's raster must be the
   size of its region"*. The layer-document rule compared the raster with the region as the sensor
   has it (5 × 8), and a region is now turned (8 × 5). The rule is the region the way up it was
   turned to (`orientation::oriented_size`). Tests: `a_turned_region_is_a_valid_layer_and_survives_a_project`
   (a layer of the turned size saves and loads; of the sensor's way round is refused).
   Why nothing saw it: the 15.9 tests opened regions and exported them, but none put a turned
   region into a layer document and rendered it.
2. **The largest region the planner offered could not be opened (since Phase 14).** The dialog's
   own default rectangle (the largest allowed, "up to 8.4 MP") was refused with "may require more
   memory than is safely available", for every format, because the region's working copy and its
   preview (up to 1,600 × 1,600 pixels) go into the layer store together and the store holds one
   working image of the largest size: a region of exactly the pixel limit is over it by the
   preview. The planner now offers the limit less `STORE_PREVIEW_PIXELS` (2.56 MP), so the figure
   is a boundary that opens. Tests: `the_largest_region_the_planner_offers_opens` (through the
   commands, 4096 × 3072 sensor, 512 MiB budget) and two planner unit tests. At that budget the
   dialog now says "up to 5.8 MP".
3. **A portrait picture was stretched in the region chooser (since Phase 14; reachable for a DNG
   only now).** The chooser's stage was the full width with a height capped at 62 % of the viewport,
   so a 2 : 3 picture was drawn 718 × 531 and stretched about twofold; the rectangle was drawn in
   the wrong shape and proportional locks looked wrong. The stage now keeps the picture's
   proportions (`max-width: calc(62vh * W / H)`). Test: a tall source keeps its aspect ratio and
   the width limit in its style.

## Mutation checks

4 of 4 caught: the planner offering the whole pixel limit again (by the flow test and by the
planner tests), a turned region layer required to be the sensor's way round, and the chooser stage
not limited in width. The last was first "caught" for the wrong reason: the new interface test
compared the style attribute as written, and jsdom serialises it with a space after each colon, so
it failed unmutated too. The full gate found it; the assertion is fixed and the mutant re-run (the
test passes on the real component and fails on the mutant at the width limit).

## Verification

| Gate | Result |
| --- | --- |
| `cargo fmt --check` | clean |
| `cargo clippy --all-targets -- -D warnings`, four feature sets | clean, all four (all features, no GPU, no inference, no plugins) |
| `cargo test --all-targets --no-fail-fast` | **1,989** / **1,982** / **1,981** / **1,950** passed, 0 failed (60 binaries), same four feature sets (15.9 ended at 1,986 / 1,979 / 1,978 / 1,947) |
| `npx tsc --noEmit`, `npm run check`, `npx vitest run`, `npm run build` | 0 errors, 0 warnings (523 files); **1,388** tests in 93 files passed; build succeeds. One unhandled error, the `CommandPalette` test that throws on purpose, unchanged |
| Packaged window, after the fixes | the maximum portrait region opens (1,971 × 2,956; 5.8 MP) and the document renders; thumbnail, analysis and histogram succeed; the chooser is 2 : 3 |

## Observed, not fixed, and not investigated

* **Closing while the document was in the broken state (defect 1) did not close the window.** The
  close request reached the page and the page called `plugin:window|destroy`, which came back with
  *"runtime error: failed to send message to the webview"*, twice; the process stayed up and
  responding. With a healthy document (and none) the window closes at once, and this was not seen
  again after fix 1, so it is recorded as seen once, cause unknown.
* **Memory settings**: after applying a manual 0.5 GB budget the radio shows *Automatic* again,
  with the note that a lower budget is saved but not in force until the next open. The saved
  figure is not shown, so a person cannot see what they chose until they open something.
* **Layout of the right-hand panel at 1,360 × 860**: the *Metadata* and *Metadata precision*
  headings are drawn on top of each other; the *Text, shapes and smart objects* panel is drawn
  with the browser's own unstyled controls; and the toolbar overflows (*Settings* is cut at the
  window's edge when *Change Source Region…* is present). These appear with a TIFF document too, so
  they are not RAW-specific, and they were not investigated or fixed here.
* The drop hint reads "Drop a PNG, JPEG, WebP or Photoshop file here" and does not name DNG, TIFF,
  EXR or SVG, which all open.

## What was not verified

* **DPI scaling** (125 / 150 / 200 %), which cannot be changed from here, and a **second monitor**.
* **A native file dialog**: it was answered at the IPC call, not clicked.
* **Installers**: the NSIS and MSI bundles were not rebuilt or installed in this pass; the portable
  release exe was.
* **EXR, SVG, PSD and clipboard in the window**: only the RAW region path, TIFF export and import,
  settings and close were driven.
* **A portrait DNG from a real camera** (the tag was rewritten in a copy).
