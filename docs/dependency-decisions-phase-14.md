# Dependency decisions, Phase 14

Every dependency Phase 14 added, why, what it was weighed against, and what it
costs. 58 package versions enter the lockfile that were not there before (it grows
from 576 to 627 packages; some updated versions replace older ones). Of the six direct
additions, three are new to the graph (`wasmtime`, `jpeg-decoder`, and the test-only
`wat`) and three were already in it (`miniz_oxide`, `crc32fast`, `windows`).

All 58 are under permissive licences — Apache-2.0 with the LLVM exception (27),
MIT or Apache-2.0 in several spellings (30), and one `Unlicense OR MIT` — and none
needs a notice beyond the usual.

## Supply-chain check

`cargo audit` was run with `--no-fetch` against the advisory database already on
this machine, which is **dated 3 August 2026, two months old**. It loaded 1,225
advisories and scanned 627 crates:

* **0 vulnerabilities.**
* 18 warnings, all `unmaintained` or `unsound` notices, **none in a crate this phase
  added**: the GTK 3 bindings (`gtk`, `gdk`, `atk` and their `-sys` crates — the Linux
  windowing stack Tauri carries in its lockfile but does not build on Windows),
  `proc-macro-error`, `glib`, `ttf-parser` and the `unic-*` crates (the text-layer
  stack from Phase 13).

The database was not refreshed, because that would be a download this phase was not
asked to make; the result is "none known as of 3 August" and not "none". Run
`cargo audit` with a fresh database before shipping to anyone else.

`wasmtime` is pinned to **exactly** `=49.0.2`; the other additions are on caret
ranges and resolved by the committed lockfile. Builds and every gate in this phase ran
`--offline` from the lockfile.

## wasmtime 49.0.2 — the plugin runtime

| | |
| --- | --- |
| Added to | `src-tauri/Cargo.toml`, optional, behind the `plugins` feature (on by default) |
| Features | `cranelift`, `runtime`, `std` only. No WASI, no component model, no text format, no threads, no async |
| Licence | Apache-2.0 WITH LLVM-exception |
| Brings | about 45 packages, a few of them dev-only: Cranelift and its code generator, `regalloc2`, `object`, `gimli`, `wasmparser`, `postcard` and friends |
| Raises | the minimum Rust version from 1.91 to **1.96** |
| Costs | about **9.5 MB** of the executable; a cold build of about 9 minutes, once |
| Alternatives | Wasmi 2.0.0 (an interpreter): measured **10.9× slower** with fuel metering on the compute-bound kernel, which would put a 45 MP filter at about four minutes instead of twenty seconds. Wasmer: not measured, a scoping decision rather than a finding. A native-DLL plugin model: rejected on security grounds |
| Why | The only candidate measured fast enough *with the limits switched on* to make filters usable at 12–45 MP; it has fuel and epoch interruption, a resource limiter, and a stable embedding API |
| Risk accepted | A JIT compiler in the process; an engine bug is a bug in PhotoForge. Stated in `docs/plugin-security.md` as the first residual risk, not worked around |
| Removing it | Build with `--no-default-features` and the other features wanted. The editor is complete without it; documents that refer to a plugin open and say which plugin they need. This configuration is built and tested |

The full evaluation, with the measurements, is `docs/plugin-runtime-decision.md`.

## Our own ZIP reader, on `miniz_oxide` and `crc32fast`

| | |
| --- | --- |
| Added to | `src-tauri/Cargo.toml` as direct dependencies. **Both were already in the graph**, through `png`; this adds no package |
| Licence | `MIT OR Zlib OR Apache-2.0`; `MIT OR Apache-2.0` |
| Alternative | The `zip` crate |
| Why not | A `.photoforge-plugin` is a file from a stranger. A general ZIP library supports encryption, ZIP64, several compression methods, archive comments, multi-disk archives, symbolic links and extra fields — each a surface a hostile archive can aim at, none of which a plugin needs. A reader that handles stored and deflated entries from a closed list of names, and refuses everything else, is a few hundred lines that can be read and fully tested |
| Cost | `src/plugins/package.rs`, with 12 tests that each construct the hostile archive from bytes, and mutation-checked |
| Risk accepted | Writing a parser is itself a risk. The mitigation is that it is small, that it refuses rather than interprets, that every number is checked in 64-bit arithmetic before use, and the tests |

## jpeg-decoder 0.3.2 — DCT-scaled JPEG decoding

| | |
| --- | --- |
| Added to | `src-tauri/Cargo.toml` as a direct dependency, `default-features = false` |
| Licence | `MIT OR Apache-2.0` |
| Why | The decoder `image` uses (`zune-jpeg`) can decode only the whole frame. This one can decode at ½, ¼ or ⅛ size in the DCT domain, which is the only way to build a bounded preview or a reduced copy of a JPEG too large to decode whole. Measured on a 108 MP file it holds about eight bytes per output pixel at a DCT scale |
| Scope | Used for that path alone. Ordinary JPEGs, and a JPEG region, still go through `image` |
| Risk accepted | The crate is, as `Cargo.toml` notes, in maintenance mode upstream (this was recorded when it was added and was not re-verified in this phase). It is a pure-Rust decoder whose source was **not audited** here. The path it serves is the only one that would otherwise be unable to open an oversized JPEG at all |

## windows 0.61.3 — three narrow reads

| | |
| --- | --- |
| Added to | `src-tauri/Cargo.toml` as a direct dependency with four features. **Already in the graph** through Tauri and `tao` **at exactly this version**, so no second copy |
| Licence | `MIT OR Apache-2.0` |
| Used for | Installed and available physical memory (the budget), this process's working set and private bytes (the memory page and the benchmarks), and free disk space (the render cache). `Win32_System_SystemInformation`, `Win32_System_ProcessStatus`, `Win32_System_Threading`, `Win32_Storage_FileSystem` |
| Not used for | Windows, handles, the registry, processes other than reading this one's own counters, or anything with side effects |

## wat 1.261.0 — tests only

A `[dev-dependencies]` entry. Fixtures for the plugin runtime and the adversarial
modules are written as WebAssembly text and compiled by the test suite. Production code
reads binary modules only; the `wat` feature of `wasmtime` is off, so the shipped
executable carries no text-format parser. Brings `wast`, `wasm-encoder` and
`wasmprinter` into the dev graph only.

## A build setting

`[profile.dev.package."*"] opt-level = 2` optimises third-party code in dev and test
builds while PhotoForge's own crate stays at opt-level 0, so its debug assertions and
overflow checks still run. Decoding a 70 MP PNG took 15 s with unoptimised dependencies,
and the image, compression and colour crates are where the time goes. It costs more time
once per dependency rebuild and repays it on every test run. It does not affect the
release build.

## What was considered and not added

* **`zip`**, above.
* **`image-webp` features for region decoding**: the crate decodes the whole frame; no
  setting changes that.
* **A WASI implementation** (`wasmtime-wasi`): a module is offered no filesystem, clock
  or network, so none is a dependency. A module that imports any WASI function is refused
  at install.
* **A scripting engine** (Lua, a JavaScript engine, Python): nothing in a manifest or a
  macro is code, and none is added.
* **A hashing or compression crate for macros or settings**: the existing `sha2` and
  `serde_json` do everything required.
