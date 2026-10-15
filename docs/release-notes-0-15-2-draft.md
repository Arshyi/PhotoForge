# PhotoForge 0.15.2 — release notes (DRAFT)

**Draft, not a release.** Written 2026-10-15 for candidate 2 (source `221ca4d`). Nothing here is
published, tagged or uploaded; the acceptance that has and has not been run is in
[the 0.15.2 ledger](release-0-15-2.md), and this page must be revised against it before use.
Unsigned: no code-signing identity exists and none is claimed.

## What changed since 0.15.1

### Data safety in the packaged window

- **The "discard unsaved changes?" question now appears.** In 0.15.1 and earlier the packaged
  window never asked: opening another file, opening a project or *New document from clipboard*
  replaced a document with unsaved edits silently. The question is now asked in the page
  (*Keep working* is focused; Escape keeps the work).
- **F5, Ctrl+R and their Shift/Ctrl variants no longer reload the window.** The host's reload
  shortcut used to discard the open document without asking. The host's right-click menu (Back,
  Refresh, Save as, Print) is suppressed outside text fields, which keep cut/copy/paste.
- **Selection sessions no longer fill the page's storage.** The 12 most recently saved documents'
  selection sessions are kept, older ones are given up first; before this a few large documents
  made every later save fail.

### Installers

- **MSI** installs for all users under `Program Files` (administrator consent), keeps its machine
  registration in the 64-bit registry view, refuses downgrades, and refuses to upgrade over an
  MSI of 0.15.1 or earlier in place — remove that one first. The upgrade code is unchanged.
- **NSIS** stays per user; a pinned template makes that explicit.
- Per-user NSIS files and the shared Desktop are not removed by the MSI's uninstall.

### Build and validation (not user-visible)

- Compact build/validation profile with storage admission checks; hosted CI runs the same
  helper.
- Read-only default preflights for the MSI and NSIS lifecycle harnesses, with guards that refuse
  an existing installation, running application, link or unexpected file; a built-MSI table
  inspection (`check_msi.ps1`); clipboard backups that are verified before any restoration.

## Known limits

- Unsigned installers and executable: Windows SmartScreen will warn. No project licence file.
- Acceptance is partial. Verified natively on the previous candidate: Program Files install and
  removal, Windows scaling 125 / 150 / 175 %, the 760 x 560 minimum window, clipboard copy and
  paste, rectangle selection with invert/undo/redo, mask from selection, project save/open and
  export through the real file dialogs. **Not** verified on these bytes: any of that again; the
  NSIS installer's lifecycle; the portable executable; PSD, EXR, SVG and TIFF through the native
  dialogs; brush and lasso; workflows and batch; 200 % scaling (waived), a second monitor,
  another user account.
- Dragging a layer row in the Layers panel to reorder may not work in the packaged window (the
  *Move layer up / down* buttons do); unconfirmed, see the ledger.
- Optional neural models are not bundled and not downloaded; the deterministic editor does not
  depend on them.
