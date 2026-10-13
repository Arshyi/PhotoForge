# Validating the packaged application

Tests prove the code; they cannot see what only the packaged executable has: the Tauri permission
list, the real WebView2, the installers, the window closing. This directory holds the harness that
drove the release executable in a real window for Phase 15.9 and found three defects no test had
(see [`docs/phase-15-9-1-results.md`](../../docs/phase-15-9-1-results.md)).

Everything here is Windows-only, runs as the ordinary user (apart from the MSI's own UAC prompts),
and leaves the person's own PhotoForge settings alone: each run gets its own `LOCALAPPDATA` and
WebView2 profile under `<work>\validate`.

## How it works

* `launch.ps1` starts the executable with its own data folders and a **local-only remote-debugging
  port** (127.0.0.1:9333, `PF_CDP_PORT` to change), and prints `PID <n>`.
* `cdp.mjs` is a small DevTools-protocol client (Node 22+, no dependencies). It sends **real pointer
  events into the page** (never to the desktop, so they cannot reach another program), takes
  screenshots, reads the page's text, and `install`s a wrapper around `window.fetch` that
  (a) answers the native file dialogs, which cannot be automated, with a path you give it
  (`window.__pf.nextOpen` / `nextSave`), and (b) logs every other IPC command with its outcome, so a
  permission refusal or an error shows up as data. Everything else (buttons, dialogs, the commands)
  is the application's own.
* The `.ps1` scripts are scenarios built from those two.

## One-time setup

```powershell
npm ci
npm run build
cd src-tauri; cargo build --release; cd ..          # or: npm run tauri build  (also builds installers)
.\scripts\fetch-raw-fixtures.ps1                    # three CC0 DNG files into .\raw-fixtures (~75 MB)
node tools\validate\make_inputs.mjs                 # landscape.dng and a portrait copy, in <work>\validate\inputs
```

`<work>` is `.work` in the repository (ignored by git); set `PF_WORK` to put it elsewhere. The
executable is looked for in `%PF_TARGET_DIR%\release` (or `CARGO_TARGET_DIR`), else
`src-tauri\target\release`.

**The memory budget.** The RAW scenarios go through the *Choose Region* chooser, which the
application only offers for a file that does not fit its memory budget. A new profile's automatic
budget on a machine with plenty of memory fits the 22-megapixel sample whole, so `launch.ps1` writes
the file the Memory settings write (`<work>\validate\localappdata\PhotoForge\settings\resources.json`,
a manual budget of 512 MiB) unless the profile already has one. `PF_BUDGET_BYTES` changes the
amount; `PF_BUDGET_BYTES=0` leaves the profile alone, and the application then picks its own budget.

## Scenarios

| Script | What it does |
| --- | --- |
| `drive_formats.ps1` | Opens and exports EXR, SVG and PSD through the real buttons; records each report dialog, the IPC commands and errors. Needs the samples from `tools/fetch_exr_samples.py`, `fetch_svg_samples.py`, `fetch_psd_samples.py`. Then `python tools\validate\check_formats.py` reads the outputs with OpenEXR, resvg and psd-tools. |
| `nsis_lifecycle.ps1` | Installs the NSIS installer silently, opens a RAW region in the installed app, closes it, uninstalls; reports files, registration, shortcuts and what is left. **Installs to and removes `%LOCALAPPDATA%\PhotoForge`**: do not run it on a machine where you use an installed PhotoForge. |
| `msi_lifecycle.ps1` | The same for the MSI, which needs UAC elevation twice (install and uninstall): somebody has to click Yes. |
| `close_repro.ps1` | Opens a portrait region with the render commands made to fail and asks the window to close, repeatedly; prints `close.log` (the application's own record of close requests). `-Runs n`; `-NoInject` for a healthy document (nothing made to fail); `-Observe` also records what the page saw. |

The helper files `listen_close.js`, `close_state.js` and `aside_layout.js` are small page snippets
the scenarios (and you, with `node cdp.mjs evalfile`) use.

## Doing it by hand

```powershell
$p = .\tools\validate\launch.ps1                    # prints PID n; wait ~8 s for the window
node tools\validate\cdp.mjs install
node tools\validate\cdp.mjs eval "window.__pf.nextOpen = 'C:/path/to/file.dng'; 1"
node tools\validate\cdp.mjs clickText Open          # the app asks for a file; it gets the path above
node tools\validate\cdp.mjs shot out.png
node tools\validate\cdp.mjs log
```

Use forward slashes in paths you pass through `eval` (a shell eats backslashes).

## Limits

It does not drive a native dialog, a second monitor or a changed display scale, and the debug port
is a development affordance: never ship a build with it enabled (it is only on for processes
`launch.ps1` starts).
