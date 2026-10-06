# Plugin runtime decision

**Decision: Wasmtime 49.0.2, core WebAssembly modules only, no WASI.**

This was decided from measurements taken on the development machine, not from
the projects' own descriptions. The benchmark crate and its raw output are
summarised below; the numbers can be reproduced by building the same kernel
under each engine.

## Requirements

A plugin runtime for a professional image editor has to:

1. confine guest code: no ambient filesystem, network or process access;
2. bound **memory**, **time** and **work**, and be interruptible mid-call;
3. run **compute-heavy pixel kernels** fast enough to be usable on 12–45 MP
   images, *with the limits above switched on*;
4. work on Windows x64, under a licence PhotoForge can ship;
5. have an API stable enough to build a public plugin ABI on.

Requirement 3 has to be judged with the limits on, because a sandbox that is only
fast when unsandboxed is not the sandbox that ships.

## Candidates

Only runtimes that could meet requirement 1 were considered.

| | Wasmtime | Wasmi | Wasmer |
| --- | --- | --- | --- |
| Evaluated | yes | yes | no — see below |
| Stable version tested | 49.0.2 | 2.0.0 | — |
| Execution | Cranelift JIT | interpreter | JIT (several backends) |
| Licence | Apache-2.0 WITH LLVM-exception | MIT OR Apache-2.0 | MIT |
| MSRV | 1.96 | 1.86 | — |

Wasmer was not benchmarked. Its capabilities overlap Wasmtime's, but it offered
nothing the measurements below needed and it adds a second large JIT to evaluate
and audit without a stated advantage here. That is a scoping decision, not a
finding that it is inferior.

A Wasmtime **release candidate** (`50.0.0-rc.1`) was the newest version on the
registry when this was evaluated; the stable 49.0.2 was used.

## Measurements

Single-threaded, release builds, Windows x64, 12-megapixel RGBA f32 buffer
(192 MB) held in guest linear memory. Two kernels:

* **Solarize** — one load, compare, select and store per float: memory-bound.
* **Poly** — 64 dependent multiply-adds per float over 3 M floats: compute-bound.

| | Wasmi 2.0.0 | Wasmtime 49.0.2 |
| --- | --- | --- |
| Solarize, no metering | 209 ms | 31.2 ms |
| Solarize, fuel | 211 ms | 34.8 ms |
| Solarize, epoch | — | 35.3 ms |
| **Poly, no metering** | 2,331 ms (329 MMAC/s) | 625 ms (1,228 MMAC/s) |
| **Poly, fuel metering** | **15,347 ms (50 MMAC/s)** | **1,406 ms (546 MMAC/s)** |
| Poly, epoch interruption | — | 2,053 ms (374 MMAC/s) |
| Module compile (trivial module) | 0.04–0.46 ms | 1.2–7.0 ms |
| Instantiation | 0.03–0.11 ms | 0.01–0.12 ms |
| Infinite loop, stopped by fuel | 39 ms | 8–34 ms |
| Infinite loop, stopped by epoch | — | 101 ms (100 ms deadline) |
| Memory hog, 64 MiB cap | stopped at 48 MiB | stopped at 48 MiB |
| Minimal binary containing the engine | 2.56 MB | 12.03 MB |
| Clean release build of the dependency | ~23 s | ~9 min |

Memory caps held exactly in both: with a 64 MiB limit a guest that grows memory
until refused obtained 769 pages (48 MiB), because the next 16 MiB step would
have exceeded the cap.

### What the numbers say

* **The memory-bound kernel hid the real difference.** On Solarize, Wasmi looked
  respectable (6× slower). On the compute-bound kernel with fuel on — the
  configuration a sandbox needs — Wasmi is **10.9× slower** than Wasmtime with
  fuel (50 vs 546 MMAC/s), and fuel costs Wasmi 6.6× over its own unmetered
  speed. Choosing from the Solarize figure alone would have under-reported the
  gap by almost half.
* **Interruption has a price in both, and it differs by mechanism.** In
  Wasmtime, fuel was cheaper than epoch on this tight loop (546 vs 374 MMAC/s),
  because epoch checks run at every loop back-edge and this inner loop is a few
  instructions. They answer different questions, so they are not substitutes:
  fuel bounds *work* deterministically, epoch bounds *wall-clock time* and is
  what cancellation needs.
* **A 45 MP filter is where it matters.** Scaling the compute-bound figure,
  Wasmtime with fuel does a 64-MAC-per-sample pass over 45 MP × 4 channels in
  roughly 21 s; Wasmi with fuel would take roughly 230 s.

## Costs accepted

| Cost | Size | Mitigation |
| --- | --- | --- |
| Binary size | about +9.5 MB over an interpreter, in a 45 MB executable | Behind a `plugins` Cargo feature; a build without it carries no engine |
| MSRV | PhotoForge's `rust-version` rises from 1.91 to 1.96 | The installed toolchain is 1.97.1 |
| Cold build | ~9 min once, then cached | CI caching; incremental builds are seconds |
| JIT attack surface | Generated native code, not interpreted bytecode | See below |
| Virtual address space | Wasmtime reserves large ranges per linear memory | 64-bit only; tuned via `Config`; one instance per call |

### The JIT, honestly

A pure-Rust interpreter has no code-generation step, which removes a class of
bug. That is a real advantage and it is why Wasmi was a serious candidate. It is
weighed here against Wasmtime's security posture. Wasmtime is developed under the
Bytecode Alliance and, by the project's own account, is fuzzed continuously and has
a published security-response process. **Those are claims from the project's
documentation and my prior knowledge; they were not verified in this evaluation**,
and no claim is made here about Wasmi's process either way. Neither runtime is a
guarantee, and the design below does
not depend on either being flawless: the guest gets **no authority** to begin
with, so an escape is bounded by the host process's own privileges, which is why
the plugin threat model assumes defence in depth rather than trust in the engine.

## Configuration

Wasmtime is configured to remove what plugins do not need and what would make
results machine-dependent:

* **No WASI.** `wasmtime-wasi` is not a dependency. The linker defines only
  PhotoForge's own host imports, so a module that imports `wasi_snapshot_preview1`
  fails to instantiate rather than being offered a filesystem.
* **Binary modules only.** Wasmtime's `wat` feature is off in production; text
  format is accepted only by tests, through the `wat` crate as a dev-dependency.
* **No threads, no relaxed SIMD.** Relaxed SIMD is implementation-defined and would
  make the `deterministic` declaration false across machines. Threads would
  escape the per-call budget.
* **NaN canonicalisation on**, so float results are bit-reproducible.
* **Core modules, not the component model.** A small, documented C-style ABI is
  easier for authors to target and to audit than generated bindings.
* **Fuel and epoch interruption both on.** Fuel is the deterministic work bound;
  epoch is the wall-clock bound and the cancellation path.
* **A `ResourceLimiter` on every store** capping memory, tables and instances.

## What this does not claim

It does not claim that WebAssembly is, by itself, a sandbox. A module is confined
only by what the host chooses to give it: here, nothing but the imports in
`docs/plugin-api.md`, gated by capabilities granted at install. The exact
residual surface is stated in `docs/plugin-security.md`.
