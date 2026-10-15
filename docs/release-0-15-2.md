# PhotoForge 0.15.2 development — storage and installer hardening

Status: implementation/validation in progress on `release-0.15.2-hardening`.
This is not a released version. The published 0.15.1 artifacts remain unchanged.

## Build storage

- The Cargo helper defaults to compact validation symbols and no incremental cache;
  full debugging is an explicit opt-in. Feature selection, optimisation, assertions,
  overflow checks and release settings are unchanged.
- The full eight-step/four-feature gate matrix checks free space and retained target
  size before and after each command. Refusals fail the gate and never delete anything.
- Hosted CI uses the same compact helper and storage checks, and verifies a tiny actual
  Rust fixture for debug assertions and overflow checks. PR pushes do not also launch a
  duplicate branch-push workflow; main and PR checks remain enabled.
- 30 tool regressions cover the storage/profile/gate/template policies. Two actual
  compact-profile Rust probe tests passed, confirming assertions and overflow checks.
  [Hosted run 38655637540](https://github.com/Arshyi/PhotoForge/actions/runs/38655637540)
  passed at code/package head `eed7238345de4ea565d6ec49424d0ccf129aa477`: frontend
  1,393 tests, TypeScript, Svelte (zero errors/warnings), production build and 65 tooling
  regressions (30 Node, 26 isolated MSI guards, nine backup-integrity tests).
- All four hosted Rust jobs passed their required formatting, compact-profile probe,
  warnings-as-errors Clippy and product tests. Top-level product totals: 2,018 all features,
  2,011 no GPU, 2,010 no inference and 1,979 no plugins; zero failures. The two profile-probe
  tests are excluded. Each log contains 59 top-level result summaries plus one nested
  recovery-test subprocess summary, not 60 distinct test binaries. Earlier summed totals
  of 2,019 / 2,012 / 2,011 / 1,980 double-counted that isolated recovery case; the child
  execution still passed, but does not add a second distinct case.
  Retrieved log tails contain every product result summary but are truncated before
  the earlier setup/Clippy output. Ten real-camera RAW checks ran in each job with
  the required, hash-verified public DNG fixtures; optional external PSD/EXR/clipboard
  fixtures are not thereby verified.
- Final hosted retained output was approximately 14.77 / 13.59 / 9.17 / 11.61 GiB
  logical in the same feature order, with all storage admissions allowed. These are
  separate hosted runners, not local allocated disk measurements or hard quotas.
  No cache-size reduction or runtime performance figure is claimed before measurement.
- A later complete hosted repeat at documentation head
  `e125124e7ff1c48d444fbfd21e94d97b6ebf4337` also passed all five jobs:
  [run 38668329140](https://github.com/Arshyi/PhotoForge/actions/runs/38668329140).
  Its four Rust log tails independently confirm the corrected top-level totals above,
  the one nested recovery result per job, zero failures and all final storage admissions.
  This repeat predates the background harness changes below; its results are not relabelled
  as verification of a later revision.

The disk audit and approved cleanup receipt are private local work products, not public
repository documents. The cleanup was not a source-code change and is not repeated here.

## Installer and release acceptance

- A pinned Tauri WiX template now reads INSTALLDIR only from the machine registry,
  writes product/shortcut key paths only to the native machine view, preserves the
  existing upgrade code, and leaves NSIS explicitly per-user.
- MSI versions through 0.15.1 are detected and refused for automatic upgrade: the old
  uninstaller could remove overlapping per-user NSIS files. Migration requires explicit
  removal of the old MSI, not silent reuse/removal of a person's existing directory.
- Future packages refuse downgrades. The upstream template's MIT notice is retained
  and bundled separately; it is not an overall PhotoForge license.
- Eight source-policy regressions catch hive/view/scope/upgrade-identity, shared-Desktop
  removal, and unsafe migration changes. This is source lint, not built-MSI or native
  lifecycle proof.
- The old MSI lifecycle harness was replaced with a read-only default preflight and
  explicit install/verify/uninstall stages. It refuses existing installations, shortcuts
  and running apps, records exact package/product ownership, preserves per-user NSIS
  state and checks the Program Files default. Cancellation, timeouts, failed exit codes
  and uncertain ownership cannot be counted as a pass. It never force-closes another
  session or deletes a directory to make a test succeed.
- 26 isolated harness regressions passed in Windows PowerShell 5.1 and PowerShell 7.
  They mock registry, process, filesystem and UAC boundaries; they do not prove an
  actual elevated install or GUI launch. The harness keeps those claims explicitly false.
- The legacy NSIS test (automatic app launch/forced close/default per-user removal) was
  replaced by read-only default preflight and explicit payload/registration stages. It
  pins the installer hash/version/asInvoker manifest, requires an exact NSS-marked
  executable reference and uses a unique owned install path. Existing/stale registration,
  app processes, linked paths, changed payload/uninstaller hashes and extra files refuse
  the action; nothing is force-deleted to make verification pass.
- 47 isolated NSIS regressions passed in Windows PowerShell 5.1 and PowerShell 7 and
  are now required in the frontend CI job. The entry paths, failure/timeout handling,
  marker parser and guards use mocks/memory streams, not installer/GUI execution. The
  no-shortcut install does not auto-launch; update-mode removal preserves shell state.
  This intentionally does not qualify the normal shortcut or full user-uninstall flow.
  Silent NSIS retains its install-location key, which the harness records and leaves alone.
  The harness requires an existing valid WebView2 version registration before preflight
  or installation; it will not implicitly download/provision that prerequisite. This
  registration guard is not a runtime-health or other-account availability claim.
- A normal-user read-only probe inspected the existing NSIS package as version 0.15.2
  and asInvoker, confirmed the frozen MSI reference hash unchanged and identified its
  MSI marker (not a valid NSIS reference). The pristine-state guard refused the existing
  stale per-user key instead of replacing it. No installer or PhotoForge process was
  launched, no registry/clipboard/display changes were made, and zero PhotoForge host
  processes were observed. The new MSI harness also passed actual read-only default
  preflight on the frozen candidate. Current NSIS lifecycle/native acceptance remains pending.
- Both installer harnesses now inspect the path entry even when Test-Path reports false,
  rejecting dangling links and access errors while allowing genuinely missing entries.
  The MSI guard suite grew from 26 to 29 regressions; the executable/package is unchanged.
- This background slice passed all 115 tooling regressions locally (30 Node, 47 NSIS,
  29 MSI, nine backup-integrity), installer source policy, the control-character scan
  and diff whitespace checks. Initial sandbox-only Node failures were junction/Git Bash
  access restrictions; the normal-host retry passed without changing the tests. The
  hosted run above predates these new harness tests and is not evidence of their CI run.
- The final local frontend repeat passed 1,393 tests in 94 files with two workers,
  TypeScript, Svelte (zero errors/warnings) and the production web build. Vite's existing
  greater-than-500-kB chunk advisory remains visible; no warning threshold was raised.
- Actual WiX validation rejected the initial machine-registry shortcut-component design
  (ICE38/43/57). Both app shortcuts now advertise the required application file component,
  with matching icon identity (ICE50), and the executable belongs only to the required
  MainProgram feature. The MSI links with validation enabled and passes built-table policy
  inspection. Three retained upstream warnings (ICE03/40/61) are recorded, not hidden.
- Silent lifecycle tests leave AUTOLAUNCHAPP unset: assigning the string 0 would be
  truthy in MSI conditions and could launch outside the isolated test profile. Source,
  built-table and harness regressions protect this boundary.
- Clipboard-backup restoration now uses a valid message-only window owner and checks
  failed writes instead of ignoring them. Existing backups cannot be overwritten;
  malformed, redirected, truncated or checksum-mismatched backup data is refused before
  restoration. Nine serialization/integrity regressions passed in both PowerShell versions
  without touching the clipboard. A real canary round trip then passed, restoring all
  seven original formats byte-for-byte; private backup contents are not committed.

## Candidate consistency (read-only, 2026-10-15)

- No product-affecting path differs between `eed7238` (the source the frozen candidate was built
  from) and the branch head `31b9f37`: nothing under `src/`, `src-tauri/` (including the installer
  template, `tauri.conf.json` and the Cargo files), `package.json` or `package-lock.json`. The nine
  files that changed are the CI workflow, documents, the validation README and the two lifecycle
  harnesses with their tests. The candidate is therefore the build of the current product source.
- SHA-256 recomputed from the candidate artifacts on the build machine
  (`C:\PFBuild\target-compact\release`): the MSI and the executable equal the values above. The
  NSIS setup of the same build is
  `e8d32f1edb10a46d3b9b6661ef68b79ce86d106ec114b0d05b260ed9255e47f7` (13,699,460 bytes), recorded
  here for the first time. Hashing only: no installer and no PhotoForge process was started.
- The local tooling regressions were reproduced on the maintainer's machine: 30 Node, 47 NSIS,
  29 MSI and nine backup-integrity tests passed (the three PowerShell suites in Windows
  PowerShell 5.1; PowerShell 7 was not re-run in this check).

## Native input block, 2026-10-15 (frozen candidate, then the fix)

Run in an agreed interactive block with the operating system's own mouse and keyboard input
and the real Windows file dialogs (no picker substitution). The candidate executable
(`6f38c3df…`, hash recomputed first) ran under an isolated `LOCALAPPDATA` and WebView data
folder. The page's own state was read through the local debugging port **read-only** (page
text, the source of `window.confirm`); no input came from it. The clipboard was backed up
first with `tools/clipboard_backup.ps1` into a fresh directory and verified identical at the
end of each run (the harness's `type` action also uses the clipboard, which the backup covers).

| Check | Result |
| --- | --- |
| Open a PNG through the native Open dialog | opened |
| **Ctrl+C**, layer (80 x 48 of a 96 x 64 picture with a transparent border) | the `PNG` form and `CF_DIBV5` are the trimmed layer pixel for pixel; cloud opt-out 0 |
| **Ctrl+Shift+C**, merged | 96 x 64, both forms pixel for pixel |
| **Ctrl+V** of a 40 x 30 picture another program put there (`PNG` + 24-bit `CF_DIB`) | "Pasted a 40 x 30 picture ... as a new layer"; copied back out, it is that picture exactly |
| **Ctrl+V** with only text | "The clipboard does not hold a picture." |
| **Ctrl+C** with the caret in the command palette's text field | copies the field's text (`CF_UNICODETEXT`), not the picture |
| Palette: **New document from clipboard** (keys only) | "Opened the 40 x 30 picture ... as a new document" |
| Rectangle selection dragged over 200 x 150 of a 400 x 300 picture | 25.0 % average coverage, 30,049 pixels (30,000 expected; 0.16 % rounding of the pointer position) |
| **Ctrl+Shift+I** invert; Undo; Redo | 75.0 % / 89,951 px (exactly the complement); Undo 25.0 % / 30,049; Redo 75.0 % / 89,951 |
| **Ctrl+D**, **Ctrl+A** | "No active selection"; 100.0 % / 120,000 px |
| Layer mask *From selection*, mask *Invert* | the picture shows only inside the rectangle, then only outside it |
| *Save project* through the native Save As dialog, then *Open project* through the native Open dialog | the saved project (7,505 bytes) reopens with the mask and its inversion |
| *Export* (PNG) through the native Save dialog | 400 x 300 RGBA; the 30,049 masked pixels are transparent (bbox x 100-298, y 75-225); the 89,951 others are byte-identical to the source with alpha 255 |
| Export dialog cancelled with Escape | nothing written, document unchanged |
| Unsaved-work **recovery offer** at start-up after a previous session | offered; *Recover* restored both layers ("Recovered ... Save it to keep the recovered work.") |
| Window closed with its X button | exits at once; `close.log` shows the three expected lines |

### Defect found: the unsaved-changes question never appeared

With an unsaved edit (a pasted layer, or recovered work marked unsaved), opening another file,
opening a project, or *New document from clipboard* **replaced the document with no question**.
Cause: `confirmDiscardChanges()` called `window.confirm`, which in the packaged application is
replaced by the dialog plugin's `async function(i){return await ...("plugin:dialog|confirm",...)}`
(read from the running page). It returns a Promise, which is always truthy, and the command it
calls is not in the window's permissions, so `if (!confirm(...)) return` never returned. Every
component test mocked `window.confirm`, and jsdom has the browser's own, so none could see it.
It is present in 0.15.1 and the frozen 0.15.2 candidate (and earlier).

Fix (`src/App.svelte`): the question is asked in the page (*Discard the changes to this
document?* with *Keep working* focused, and *Discard and continue*; Escape keeps the work;
nothing else runs while it is open), awaited at all nine places that ask. Tests: four
lifecycle tests (asks and keeps; replaces only after *Discard and continue*; Escape keeps; no
question when nothing is unsaved), a source test that no page `confirm`/`alert`/`prompt` is
used, and one that every ask is awaited. Mutation checks: four of five caught (the fifth, the
extra `allowWorkspaceMutation` guard, is redundant with "one question at a time"). Frontend
gates after the change: TypeScript clean, Svelte check 524 files 0 errors and 0 warnings, 1,399
tests in 94 files, no unhandled errors.

Verified natively on a build with the fix (`ef88c067…`, built from the working tree into a
separate target so the frozen candidate was not overwritten): with an unsaved pasted layer,
choosing another file shows the question; *Keep working* leaves the two-layer document intact;
Escape does the same; *Discard and continue* opens the file.

**Consequence for the candidate:** `src/App.svelte` is a product-affecting file, so the frozen
candidate (`eed7238`, MSI `2ecde95b…`) does not contain the fix and is superseded for release
purposes; a new candidate has to be built, frozen and hashed. The earlier native evidence
(Program Files install/uninstall, 125/150/175 % scaling, the 760 x 560 minimum window) was
taken on the old candidate; the change is confined to the page's discard question and its
tests, but that evidence is for the old binary and is not relabelled.

### Observed, not changed

* A recovered document's file card shows the snapshot's internal name
  (`session-...photoforge`) while the notification says the original name.
* Choosing the *High JPEG* profile and then Export still opened the Save dialog with
  "PNG image" as the type (the dialog was cancelled; not investigated).
* A snapshot of work the person deliberately replaced persists and is offered at the next start
  (by design: the close dialog says "some work may be recoverable"). With the question now
  real, the person has been asked first.
* The harness's `open_application` launches a **second instance on the real profile** for a
  portable executable; bring an isolated window forward by its handle instead. (One such
  instance was started by mistake and closed gracefully.)

### Not verified in this block

The NSIS and portable packages; PSD, EXR, SVG and TIFF through the native dialogs; the
brush/eraser and lasso tools with the pointer; workflows and batch; the 175 %/150 %/125 %
scales again on the new build; a second monitor; another account.

## Second defect pass and candidate 2, 2026-10-15 (no native input)

### Packaged-only divergences found (the class of the `confirm` defect)

What the packaged window does differently from jsdom and from CDP-driven pages, found by reading
the page's own state and by posting targeted key messages to the window (no clipboard, no global
keystrokes):

| Finding | Evidence | Change |
| --- | --- | --- |
| **F5 / Ctrl+F5 / Shift+F5 / Ctrl+R / Ctrl+Shift+R reloaded the page**, dropping the open document and everything unsaved in it, with no question. The page's `beforeunload` handler did not help (`Page.javascriptDialogOpening` never fired). CDP key events do not trigger WebView2's host accelerators, so a CDP-only check would have passed. | Reproduced with a targeted F5 message to the render widget window: navigation type `reload`, document gone. On a build with the change the page's `timeOrigin` was unchanged and the document intact. | `3414fbf`: capture-phase cancel of those keys (`isPageReloadKey`), and of the host's right-click menu (Back, Refresh, Save as, Print) outside text fields (`allowsNativeContextMenu`). Unit tests for both predicates and an App-level test. |
| **Selection sessions filled the page's storage.** Each document's session is stored in `localStorage` (up to 3.5 million characters); nothing removed any, so the third large one hit the quota, every later save failed and the person was told persistence was off. | Reproduced against a quota-limited storage in the unit test (the cause is in the code, not in a packaged-only API). | `221ca4d`: the 12 most recently saved sessions are kept (index key, older sessions adopted), and the oldest are given up one at a time when the storage refuses. Four tests; mutation checks 4 of 4 caught. |

Not verified: Ctrl+R and the right-click-menu suppression natively (the menu change is by
reasoning about the host's defaults; a targeted right-click raised no detectable menu window);
the retention in the packaged window, where the quota is WebView2's own; other host accelerators
(Ctrl+P, Ctrl+F, F7, Ctrl+wheel zoom) are left to the host.

### Suspected, not changed

* **Layers panel drag-to-reorder.** The rows use HTML5 drag and drop (`draggable`, `dataTransfer`,
  `LayersPanel.svelte`), and the window is created with `dragDropEnabled: true`
  (`src-tauri/tauri.conf.json`) because `App.svelte` opens dropped files through the native
  `onDragDropEvent`. Tauri documents that on Windows the native handler stops HTML5 drag events
  reaching the page. If so, drag-reorder does nothing in the packaged app and the *Move layer
  up / down* buttons are the working path. This is unconfirmed: it needs one native drag of a
  layer row in the packaged executable. A fix is a pointer-event reorder (no `dataTransfer`),
  not turning the native file drop off.
* **Several instances.** Recovery snapshots are limited to three in one shared folder; with
  two instances running, one could prune the other's. Suspected from the design, not
  reproduced.
* **Workspace settings export/import.** *Export layouts & shortcuts* saves through an
  `<a download>` on a blob URL and *Import settings* uses `<input type="file">`
  (`WorkspaceSettings.svelte`); every other export goes through the native dialogs. How
  WebView2 handles that download in the packaged window (where the file lands, whether a
  dialog or the host's download bar appears) is untested. Check natively with one export and
  one import.

### Candidate 2 (built from `221ca4d`, supersedes candidate 1)

`npm run tauri build` finished with exit 0 in the compact profile
(`C:\PFBuild\target-compact`); the product source differs from candidate 1's (`eed7238`) by the
page-level discard question, the reload/menu guard, the selection retention and their tests.
Copies are kept in the ignored `release/0.15.2-candidate-2/`; candidate 1's are untouched in
`release/0.15.2-candidate-1/`.

| Artifact | SHA-256 | Bytes | Authenticode |
| --- | --- | --- | --- |
| `PhotoForge_0.15.2_x64_en-US.msi` | `4574f02f577faf88ac5c5c3dda1353c23ee91e53e6550e9855e8d68bee6331fb` | 20,025,344 | NotSigned |
| `PhotoForge_0.15.2_x64-setup.exe` (NSIS, per user) | `1efcbc7d6390fd8ca6c70a2d0c9070e4885306bf4a1e4b907bab105282f313b6` | 13,696,043 | NotSigned |
| `photoforge.exe` (build output, bundle tag `UNK`; the portable executable) | `2de5a3f71dd5644c49d902128c1dee3b5a06d499ebe25d0cec98007f953cb7f1` | 61,609,984 | NotSigned |

Read-only inspection (nothing installed, launched, or written outside `release/` and a scratch
folder):

* `tools/validate/check_msi.ps1` on the MSI: `pass`, version 0.15.2, product code
  `{C3891216-3C77-4659-A650-09B7F44B925A}`, scope `perMachine`, `ProgramFiles64Folder`, two
  machine registry entries, legacy migration "explicit uninstall required through 0.15.1".
  `nativeLifecycleVerified` is `false`.
* The MSI's embedded cabinet was copied out through the Windows Installer database (read-only
  open) and expanded: its `photoforge.exe` is
  `1f347b813f0271b56382e61f481e5aa1a210d34b8073670df4aa488b417c2bed` and equals the build
  output with only the three-byte bundle tag changed from `UNK` to `MSI`. Its notices file
  equals the repository's `THIRD_PARTY_NOTICES.md` (`ae81b511…`).
* The build log shows `candle` and `light` completing and contains no ICE lines, so the three
  upstream-template warnings recorded earlier (ICE03/40/61) are neither reproduced nor ruled
  out for this build.
* NSIS preflight (`nsis_lifecycle.ps1`, default action, Windows PowerShell 5.1, pinned setup hash
  above): the package assertions (hash, `PhotoForge` 0.15.2 identity, `asInvoker` manifest), the
  `currentUser` scope and the reference's `NSS` marker all passed; the **pristine-state guard
  then refused**, as before, at the stale per-user key
  `HKCU:\Software\photoforge\PhotoForge` in this account. That is a refusal, not a pass, and
  nothing was removed.
* The NSS reference used there is **derived**, not extracted: the build leaves only the
  unmarked and (inside the MSI) the MSI-marked executable, and Tauri patches the tag in place,
  so the NSS-tagged copy
  (`a8620f3efccc27eab77681893c48c7d106d71fdcd5eb182d08984ec17d7a8d14`) is the build output with
  `UNK` replaced by `NSS`. The same substitution reproduced the MSI payload byte for byte, but
  that the NSIS payload equals this copy is established only by the harness's Install stage,
  which has not been run.

Still required for this candidate, none of it run: MSI install under UAC and its scaling checks
on these bytes, the NSIS Install/Verify/Uninstall stages in a pristine account, the portable
executable launched natively, and the remaining native feature checks listed above.

## Groundwork branches (0.16 and DNG), 2026-10-15, local and unpushed

Three branches were cut from `2c99c6f` and are **not part of candidate 2**: each changes
product source, so merging any of them before release would make candidate 2 a different
build. They are source and unit-test work only; nothing was run in the packaged application.
Each passed `cargo fmt`, `clippy --all-targets -D warnings` and its module's tests in the
compact profile, and each fix was mutation-checked (a mutant that survived was a gap in a test,
which was closed and the mutant re-run).

| Branch | Change | Tests | Mutants |
| --- | --- | --- | --- |
| `groundwork-0.16-model-store` (`962ef7f`, docs `b646d07`) | Model store: ids that differ only in case, or spell a Windows device, are refused; the installed copy is re-hashed so a source that changes during import is refused; staging file removed however the import ends; a manifest that cannot be written rolls the import back; a failed removal leaves the model installed; one import at a time; `run_image` refuses a run whose output accumulators exceed 4 GiB (`run_image_within` takes a budget) before allocating; import/remove run on a blocking thread. | 10 new; 38 inference tests pass | 12 of 12 caught |
| `dng-masked-black-level` (`3b2d24b`) | `MaskedAreas` is read; the black level is measured on the masked photosites, recorded as `masked_black_level`, and used only when the file declares no `BlackLevel` (a declared level stays authoritative); a window decode measures what a whole decode does. | 7 new; 118 raw tests pass | 5 of 5 caught (the `counts.contains(&0)` guard is redundant with the finite check and has no distinguishing mutant) |
| `dng-default-scale` (`59a29ac`) | `DefaultScale` is read and kept as `SensorImage::default_scale`; **not applied** (applying it also changes region mapping, orientation, previews and the recorded cut, and no non-square-pixel DNG is available to check against). | 3 new; 114 raw tests pass | 4 of 4 caught |

Not verified: any of it against a production model or a real camera DNG that exercises the
changed path (the Canon sample declares a black level, so it takes the unchanged "declared wins"
route); the blocking-thread wrapper of the model commands has no unit test of its own; the
interface does not show the measured black level or the source of a model, and no run command
exists yet. `docs/local-inference.md` on the model-store branch lists what 0.16 still needs.
The two RAW limits in the roadmap ("DefaultScale and non-square pixels", "masked photosites")
stay as written until those branches are merged and, for the first, until it is applied.

## Executed native acceptance (partial)

The frozen MSI candidate was built from `eed7238`, before subsequent evidence-only
documentation updates. Its SHA-256 is
`2ecde95b09ad2b6c899995426bf120c2539b822a595d7ab684772ca605e514de`.
Its installed/reference executable SHA-256 is
`6f38c3dfbca72a999bcc15bbc14e6a2e7f63c1a0d1ff130fe8b9102cec8bf4f9`.

- Actual user-approved elevated install completed with exit 0. The default was
  Program Files, machine registration and advertised common shortcuts were verified,
  and the existing stale per-user NSIS registry state was preserved without redirecting
  the machine install. No existing installed product or process was replaced.
- The exact installed executable launched under an ordinary user with isolated
  settings/WebView data. Native Computer Use opened a synthetic transparent PNG
  through the real Windows file picker; no picker substitution was installed.
- Actual Windows scaling at 125% / 150% / 175% was verified as DPR 1.25 / 1.5 / 1.75,
  with visualViewport.scale 1, not browser emulation. Normal/maximized layouts,
  native rectangle coordinates, sidebar scrolling and refine before/after/apply
  passed at these scales. At 175%, the shorter refine modal scrolls; its complete
  Apply/Cancel footer remained reachable and Apply succeeded.
- The 175% coordinate check briefly attached passive canvas pointer listeners to
  measure trusted native mouse coordinates, with no input dispatch/event modification.
  The measured 47.4286 by 32 image-pixel drag predicts 24.7024% average coverage;
  the UI reported 24.7%. The observer was removed afterward. Other CDP checks were
  read-only; no CDP input or picker stubs are counted as native evidence.
- The user reported that the standard scale menu stops at 175%, accepted 175% as a
  substitute and explicitly waived 200%. No custom system scaling was forced.
  Native 200% is **waived/untested, not passed**. The original 100% baseline was restored
  and verified (DPR 1, screen 1920 by 1080); the selection survived the transitions.
- Native outer-border resizing at 100% reached the actual minimum client size of
  760 by 560, with no horizontal document overflow. The compact toolbar, preview,
  scrolling sidebar, bottom controls and footer remained usable. A new editable
  synthetic project saved through the real Windows Save As dialog at that size.
- Native Alt+F4 closed the saved installed-app session normally; the window and host
  process disappeared. The guarded elevated uninstall initially returned UAC cancellation.
  After the user reaffirmed approval, the exact owned-product retry completed exit 0
  with removal verified: machine product/files/common shortcuts gone, shared Desktop
  and stale per-user NSIS state preserved. The saved test project and original PNG
  remain separate from the removed installation. The original PNG byte-matched a
  freshly regenerated canonical fixture; it was not overwritten for this comparison.
  Harness GUI flags deliberately remain false; separate native evidence supplies the
  launch/open/save/close portion of this actual MSI install/launch/uninstall result.

The original 73-case native ledger currently has 12 passed, one partial, one waived
and 59 not run; the supplementary 175% case passed separately. It is not a complete
native acceptance result.

The full matrix, final clipboard-window regression, current NSIS/portable acceptance
and cross-application verification remain pending. No mixed-monitor, other-user-account
or complete zero-network process-tree claim is made. The upstream WebView2 prerequisite
handling is retained and is not proof of runtime availability for every Windows account.

The user chose to leave the project license unset for now and confirmed that no trusted
code-signing certificate/service is available. No project LICENSE file or signing identity
will be invented. Third-party license obligations/notices remain separate from that choice.
The pinned candidate executable and MSI were both inspected as Authenticode `NotSigned`.
If a project license is chosen later, GitHub provides a
[license-template workflow](https://docs.github.com/en/communities/setting-up-your-project-for-healthy-contributions/adding-a-license-to-a-repository);
that decision is separate from obtaining a trusted code-signing identity.
