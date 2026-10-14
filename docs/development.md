# Developing PhotoForge

How to get from a fresh clone to a working build, how the project is verified, and how a release
is made. Everything in the project is meant to be reproducible from this repository on a new
Windows machine; if a step here does not work, that is a defect in this page.

## What you need

| | Needed | Developed with |
| --- | --- | --- |
| Windows | 10 or 11, x64 (the application is Windows-only: WebView2, the Win32 clipboard) | 11 |
| WebView2 | the Evergreen runtime (built in on Windows 11), recent enough to support `ICoreWebView2Settings8`; the application refuses to start on one that cannot apply its privacy setting | current |
| C++ toolchain | Visual Studio Build Tools, "Desktop development with C++", with the Windows SDK | 2022 |
| Rust | **1.96 or newer** (`rust-version` in `src-tauri/Cargo.toml`), `x86_64-pc-windows-msvc`; some dependencies need 1.90 to 1.91 on their own | 1.97.1 |
| Node.js | 20 or newer, with npm | 24.18.0, npm 11 |
| Git for Windows | its `bash` runs the `scripts/*.sh` and the mutation harnesses | current |
| Python (optional) | 3.12+, only for the oracles and corpus scripts in `tools/` | 3.14 |

`cargo` is often not on `PATH` in a non-interactive shell. `scripts/cargo-ext.sh` adds
`~/.cargo/bin` itself; in your own shell run `export PATH="$HOME/.cargo/bin:$PATH"` (bash) or add
it in the system settings. Cargo commands run from `src-tauri` (there is no workspace manifest at the
repository root).

NSIS and WiX, which `tauri build` needs to make installers, are downloaded by the Tauri CLI on first
use (once, from GitHub). Nothing else in a normal build or test run touches the network.

## First build

```powershell
git clone https://github.com/Arshyi/PhotoForge.git
cd PhotoForge
npm ci                       # exact versions from package-lock.json
npm run tauri dev            # the application, with hot reload of the interface
```

or, for a release executable without installers:

```powershell
npm run build
cd src-tauri
cargo build --release        # src-tauri\target\release\photoforge.exe (the interface is embedded)
```

This direct-Cargo executable is a development candidate, not the distributed portable
build. On the tested Windows toolchain it imports `VCRUNTIME140.dll` and
`VCRUNTIME140_1.dll`. Use `npm run tauri -- build -- --locked --offline` for the final
offline-capable packaging pass after dependencies and bundler tools have been fetched:
the Tauri CLI enables its static Visual C++ runtime build. Inspect the final executable's
imports rather than assuming the two build paths produce equivalent dependencies.
The 0.15.1 final build has neither runtime DLL import; WebView2 is still required.

A cold Rust build takes on the order of ten minutes and several gigabytes (wasmtime and wgpu are
large); an incremental release build of the application crate is about three minutes. The debug and
test targets for the four feature sets below are much larger, on the order of tens of gigabytes: set
`PF_TARGET_DIR` (read by `scripts/cargo-ext.sh`) or `CARGO_TARGET_DIR` to put the build output
somewhere with room. A running `photoforge.exe` locks the executable: close it before rebuilding.

### Build-storage policy (0.15.2 development)

`scripts/cargo-ext.sh` now defaults to `PF_BUILD_MODE=compact`: line-table debug information
for both PhotoForge and dependencies, with incremental compilation disabled. This retains
source/line backtraces but not full debugger variable inspection. `PF_BUILD_MODE=full` uses
ordinary Cargo profiles and preserves the caller's incremental setting. Neither mode removes
features or tests, changes optimisation/assertion/overflow settings, or changes release profiles.
Direct Cargo/Tauri commands do not automatically opt into this helper policy.

The gate runner checks storage before and after every command. Defaults are at least **20 GiB
free** and at most **64 GiB retained logical output**; `PF_MIN_FREE_GIB` and `PF_CACHE_MAX_GIB`
accept explicit positive GiB overrides. Hosted CI uses 4/32 GiB on its disposable runners.
Final and intermediate Cargo output are pinned to the same checked root. Counts include the
entire target (including release), conservatively count hard links more than once, and are not
allocated-space measurements. Checks are admission checks, **not a hard disk quota**: one command
can grow past the budget before its post-check refuses further work. A refusal fails the gate,
does not qualify skipped tests as passing, and never deletes anything.

```powershell
$env:PF_TARGET_DIR = 'C:/PFBuild/target-compact'  # one reusable compact cache, not one per run
node scripts/build-cache.mjs check --target-dir $env:PF_TARGET_DIR --base-dir src-tauri
bash scripts/gates.sh
```

Keep full-debug work in a separate, deliberately managed target. Retain one active cache per
mode, stop/review oversized caches, and retire only explicitly selected generated output after
preserving release packages and evidence. There is no automatic pruning, age-based deletion,
global environment change, source/test trimming, or movement of models/projects to another drive.
The compact profile probe additionally checks that debug assertions and overflow checks still run.

## The gates: what "green" means

Frontend (from the repository root):

```powershell
npx tsc --noEmit
npm run check          # svelte-check: 0 errors, 0 warnings
npm run test           # vitest
npm run build
```

Rust (from `src-tauri`, or let `scripts/gates.sh` run all of it):

```powershell
cargo fmt --check
cargo clippy --all-targets --all-features -- -D warnings
cargo test --all-targets --all-features --no-fail-fast
```

The project is held to **four feature sets**, because each of `gpu`, `inference` and `plugins` can
be compiled out and must still build, pass clippy and pass its tests: all default features,
`--no-default-features` plus `custom-protocol,inference,plugins` (no GPU), `...,gpu,plugins` (no
inference) and `...,gpu,inference` (no plugins). `scripts/gates.sh` runs clippy and tests for all
four and prints a summary:

```bash
scripts/gates.sh                     # logs under .work/gates ; about 25 minutes warm
```

At the 0.15.0 release: **1,993 / 1,986 / 1,985 / 1,954** Rust tests passed (60 test binaries), 0
failed; **1,389** frontend tests in 93 files; clippy clean in all four sets. That historical run
contained an unhandled command-palette error. It was a synchronous command escaping the promise
handler and is fixed for 0.15.1. Current gates require zero unhandled errors; 1,393 frontend tests
in 94 files now pass. `npm run test:tools` also checks the Rust gate runner's exit status with
successful, failed, and missing-output commands. The runner finishes all configurations and exits
nonzero if any check fails or produces no test results. Tool regressions also cover compact/full
profile dispatch and fail-closed, read-only storage admission, including post-command growth.

Run the frontend suite on a quiet machine. The heavy `App*.test.ts` files time out under CPU load
(a cargo build running at the same time is enough), and a timeout is not a defect.
`components::ollama` has two localhost tests that can fail once under load and pass alone.

## Tests that need real files

Normal test runs never touch the network. A few tests check the decoders and writers against real
files from other programs; they **skip themselves and still pass** unless an environment variable
names a directory:

| Variable | Used by | How to get the files |
| --- | --- | --- |
| `PHOTOFORGE_RAW_FIXTURES` | `tests/raw_real_files.rs` and friends (a Canon DNG in three encodings) | `scripts/fetch-raw-fixtures.ps1` (CC0, about 75 MB) into `raw-fixtures/` |
| `PHOTOFORGE_EXR_SAMPLES` | `tests/exr_real_files.rs` | `python tools/fetch_exr_samples.py` |
| `PHOTOFORGE_PSD_SAMPLES` | `tests/psd_real_files.rs`, `psd_import_real.rs`, `psd_export_real.rs` (a file or a directory, searched recursively) | `python tools/fetch_psd_samples.py` |
| `PHOTOFORGE_TIFF_CORPUS` | `tests/tiff_real_files.rs` | `python tools/make_tiff_corpus.py` |
| `PHOTOFORGE_CLIPBOARD_TESTS=1` | `tests/clipboard_real.rs` | none: **these replace the clipboard**; `tools\clipboard_backup.ps1 -Action save` first and `-Action restore` after |
| `PHOTOFORGE_*_DUMP` | the same real-file tests | a directory to leave outputs in for the oracles in `tools/` |
| `PHOTOFORGE_PSD_EXPORT_DUMP` | `psd::tests::exporter` (a unit test) | a directory: the exported PSDs are left there, to compare two builds' bytes |
| `PHOTOFORGE_SVG_DUMP` | `svg::export::tests` (a unit test) | a directory: SVGs and PhotoForge's render of each, for `tools/svg_compare.py` (resvg) |
| `PHOTOFORGE_SVG_SAMPLES` + `PHOTOFORGE_SVG_IMPORT_DUMP` | `svg::import::tests` (a unit test) | `python tools/fetch_svg_samples.py` then `prep_svg_samples.py` into `.work/svg-samples-256`; the dump directory is read by `tools/svg_import_compare.py` (resvg) |

Do not set one of these to an empty string: an empty value counts as "set" and the test then fails
(unset it instead).

Because they skip, **the gates cannot tell you a real-file test is broken**: run them with the
variables set whenever a decoder, the picture geometry or a writer changes. One orientation test
was wrong from 15.8 to 0.15.0 (its baseline was still the uncut sensor) and was only found by
running the file on a fresh clone.

## How the tests are checked

* **Mutation checks.** A new test is shown to fail for the right reason by breaking the code it
  guards and watching it fail (`tools/mutate_*.py`, see `tools/README.md`). Several tests that
  passed for the wrong reason were found this way. Run a harness with `--dry` first (it checks
  that every pattern still matches and changes nothing; 30 of the 197 patterns no longer do at
  0.15.0, listed in `tools/README.md`). Never format, commit or build while a harness runs: it
  edits source files in place.
* **Oracles.** What PhotoForge writes is read back with programs that are not PhotoForge (Pillow,
  tifffile, psd-tools, OpenEXR, resvg): `tools/*_oracle.py` and friends.
* **The packaged window.** The release executable is driven in a real window, with real pointer
  input, to see what tests cannot (permissions, the real WebView2, the installers, the window
  closing): `tools/validate/`. **Run it before calling a phase done**: it found three defects in
  15.9 that nothing else had.

## Checking a new machine

After the first build, this is the shortest path that shows the checkout works end to end. It is
what was run, from a fresh clone in an empty directory with an empty build output, to check this
page:

```powershell
npm ci; npm run build; npx tsc --noEmit; npm run check; npx vitest run
cd src-tauri; cargo build --release --locked; cd ..       # 9 min 18 s cold, on the development machine
.\scripts\fetch-raw-fixtures.ps1                           # ~75 MB, about two minutes
node tools\validate\make_inputs.mjs
.\tools\validate\close_repro.ps1 -Runs 2 -NoInject -Observe
$env:PHOTOFORGE_RAW_FIXTURES = "$PWD\raw-fixtures"
$env:PHOTOFORGE_UPRIGHT_DUMP = "$PWD\.work\upright"        # the test leaves its pictures here
bash scripts/cargo-ext.sh test --release --locked --test raw_real_files   # ~3 min after the build
python -m venv .work\venv; .work\venv\Scripts\pip install -r tools\requirements.txt
.work\venv\Scripts\python -I tools\upright_oracle.py .work\upright
```

(`PF_TARGET_DIR` and `PF_WORK` move the build output and the scratch space; set them in the same
shell first if the repository is on a small drive.) What to expect:

* the frontend: 0 errors and 0 warnings in `npm run check`; 1,393 tests in 94 files with no
  unhandled errors; 30 storage/profile/gate/installer-source regressions in
  `npm run test:tools`, plus 26 mocked MSI harness guards in
  `tools/validate/msi_lifecycle.test.ps1` (no real installation or UAC), and nine
  serialization/integrity checks in `tools/clipboard_backup.test.ps1` (no clipboard access);
* `close_repro.ps1`: each run opens a RAW region (the harness writes a 512 MiB manual memory budget
  so that the region chooser is offered) and the window exits at once on a close request; `close.log`
  shows `close requested`, `the page decided: close`, `the window was destroyed` for each run. An
  `ERROR no visible control with text` line means the chooser was not offered (the file fitted the
  budget) and nothing past that was exercised;
* the real-file tests: `10 passed`. A test that finds no fixtures **skips and passes**, so look for
  `skipped:` in the output if the variable might not name a directory that exists;
* the oracle: `orientation 2` to `8` each `identical to Pillow's transpose` and `problems: none`:
  the developed Canon sample, turned by each orientation value, is what a program that is not
  PhotoForge makes of it.

## Packaging and releasing

```powershell
npm run tauri build          # NSIS and MSI under <target>\release\bundle\, and the exe
```

A release is, in order:

1. Version: `package.json`, `package-lock.json` (two places), `src-tauri/Cargo.toml`,
   `src-tauri/Cargo.lock` (the `photoforge` entry) and `src-tauri/tauri.conf.json`.
2. The results page for the work (`docs/phase-N-results.md`), and the README's version and claims.
3. The gates above on exactly that tree; then the installers from that commit.
4. `SHA256SUMS.txt` (tracked) with the hashes of the installers and the portable executable; the
   binaries themselves go under `release/<version>/` (ignored) and into the GitHub release.
5. A pull request into `main`, merged; then an annotated tag on the merge commit and a GitHub
   release:

```powershell
git tag -a v0.15.0 <merge-commit> -m "PhotoForge 0.15.0"
git push origin v0.15.0
gh release create v0.15.0 <files> --prerelease --title "PhotoForge 0.15.0" --notes-file <notes.md>
```

The installers are **not signed** (no code-signing identity exists), and the release notes say so;
Windows SmartScreen can warn. The NSIS installer is per-user. Published MSI versions through
0.15.1 are registered per machine but can install into the installing user's profile. The
0.15.2 candidate isolates machine state and defaults to Program Files, with explicit refusal
of unsafe legacy MSI migration. Built-table and elevated lifecycle evidence are required
before treating that candidate policy as validated (see `docs/release-0-15-2.md`).

## Layout of the repository

| Path | What it holds |
| --- | --- |
| `src/` | the interface (Svelte 5 with the legacy `export let` / `on:` syntax, TypeScript); `src/lib/` by area; `*.test.ts` beside the code |
| `src-tauri/src/` | the Rust crate: the engine and every format (`layers/`, `operations/`, `raw/`, `psd/`, `tiff/`, `openexr/`, `svg/`, `metadata/`, `clipboard/`, `resources/`, `source/`, `plugins/`, `commands/`, ...) |
| `src-tauri/tests/` | integration tests, many of them through the real IPC boundary with a mock webview |
| `docs/` | topic documents (`architecture.md`, `layers.md`, `raw-development.md`, `psd-import.md`, ...) and one `phase-N-results.md` per piece of work |
| `plugins/` | the plugin format, examples and adversarial test packages |
| `scripts/` | `gates.sh`, `cargo-ext.sh`, `fetch-raw-fixtures.ps1`, `benchmark-precision.ps1` |
| `tools/` | oracles, corpus scripts, mutation harnesses and the packaged-window harness |
| `release/`, `raw-fixtures/`, `.work/` | ignored: artifacts, downloaded samples, scratch |

## Conventions that matter

* **Report honestly.** Each piece of work ends in a results page with what was completed, what was
  verified and how, what was **not** verified, and what is known and left. A claim without a run
  behind it does not go in.
* **One behaviour change per commit, with its tests**, and the message carries the verification
  numbers and the not-verified list.
* **Project format version**: the lowest that holds the document (2 unless it uses state only newer
  builds know), so an older build opens what it can and refuses the rest by name
  (`docs/project-format.md`).
* **Line endings.** Files on disk are a mix of LF and CRLF and git warns about it; edit with a tool
  that preserves what is there (a blanket normalisation makes every line a diff).
* **Shells.** Pass paths with forward slashes to anything that goes through a bash shell; write
  scripts with a file, not a here-document, when they contain quotes or backslashes.

## Where the history is

`git log` and the `docs/phase-*-results.md` pages are the record. The original phase briefs were
written in a chat and are not in this repository; the results pages restate what each phase set
out to do. [`roadmap.md`](roadmap.md) says where the project is going.
