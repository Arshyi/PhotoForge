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
  warnings-as-errors Clippy and product tests. Product totals: 2,019 all features,
  2,012 no GPU, 2,011 no inference and 1,980 no plugins, across 60 test binaries each;
  zero failures. The two profile-probe tests are excluded from these product totals.
  Retrieved log tails contain every product result summary but are truncated before
  the earlier setup/Clippy output. Ten real-camera RAW checks ran in each job with
  the required, hash-verified public DNG fixtures; optional external PSD/EXR/clipboard
  fixtures are not thereby verified.
- Final hosted retained output was approximately 14.77 / 13.59 / 9.17 / 11.61 GiB
  logical in the same feature order, with all storage admissions allowed. These are
  separate hosted runners, not local allocated disk measurements or hard quotas.
  No cache-size reduction or runtime performance figure is claimed before measurement.

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
