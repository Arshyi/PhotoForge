# Plugin security

This is the threat model for plugins, what each defence is, where it is tested, and
— the part that matters most — what is **not** protected. If a claim here is not
backed by a test or by the code it names, treat it as unverified.

Short version: a plugin is a pure function from pixels to pixels, in a WebAssembly
sandbox with **no authority**, whose work, memory and time are capped, whose output
is checked before it is believed, and whose every effect on a document goes through
the same transaction engine as a person's own edit. It runs **inside the PhotoForge
process**, behind the engine's sandbox and nothing else. There is no operating-system
sandbox and no separate process. See *Residual risk*.

## What is being protected

| Asset | Threat |
| --- | --- |
| The person's files and accounts | A plugin reads or writes them, or phones home. |
| The open document | A plugin corrupts it, half-edits it, or edits what it was not allowed to. |
| The PhotoForge process | A plugin crashes it, hangs it, exhausts its memory, or escapes the sandbox. |
| The person's trust | A plugin or a document silently renders with something other than what it names. |

## Who the attacker is

1. **The author of a package**, who wants code to run with more authority than they
   were given, or to make the application unusable.
2. **The author of a document or project**, who controls what a `.photoforge` file
   asks a plugin to do and with what parameters.
3. **A plugin acting on another's behalf** — a confused deputy that spends authority
   a different plugin, or the person, holds.

The person who chooses to install a package is trusted to decide whether to trust its
author. PhotoForge does not and cannot verify who that is: **packages are not signed
and the installer says "Not signed."** What PhotoForge limits is what a package can do.

## Defences, in the order an attack meets them

| Stage | Defence | Tested in |
| --- | --- | --- |
| The package file | A purpose-built ZIP reader with a closed file list; stored and deflate only; no encryption, ZIP64, comments or links; overlap, bomb and CRC checks; every size and offset checked in 64-bit arithmetic. `docs/plugin-package.md`. | `tests/plugin_package.rs` (12 tests, each refusal constructed from real bytes, and mutation-tested) |
| The manifest | Strict parsing: unknown fields refused, every count and length bounded, ids and paths validated, no expressions. A capability in `filesystem`, `network`, `process`, `shell`, `registry`, `env`, `clipboard`, `device`, `native`, `net`, `fs` **does not exist** and is refused outright. Commands cannot delete, flatten, merge down or lock; a plugin may apply only its own filters. | `plugins::manifest` unit tests |
| The module | Linked against exactly one import, `photoforge.log`. **No WASI.** Anything else — including every WASI function — is refused at install, naming it. Exports and their signatures are checked; memories, tables and instances are capped. | `tests/plugin_runtime.rs`, fixtures `imports_wasi`, `imports_unknown_host_function`, `imported_memory`, `missing_filter_export`, `filter_with_wrong_signature`, `wrong_abi_version`, `empty_module` |
| One run | A **fresh instance per call**; memory capped by a `ResourceLimiter` including the declared initial size; work capped by fuel; wall-clock capped by an epoch interrupt, which is also how cancelling works; threads absent from the engine; relaxed SIMD off; NaN canonicalised. | `infinite_loop`, `start_loop`, `memory_hog`, `huge_initial_memory`, `deep_recursion`, `unreachable`, `reads_out_of_bounds`, `bad_alloc_pointer`, `zero_alloc_pointer` |
| Its output | The right size, every value finite, alpha within tolerance: **refused, not repaired**. Log lines bounded in number and length, control characters stripped. | `nan_output`, `alpha_out_of_range`, `infinite_output`, `log_flood`, `log_out_of_range`, `log_with_wrong_type`, `reports_error` |
| Installation | All or nothing. The module must compile and each filter must pass a self-test: deterministic (two runs identical) and honest about locality (whole and tiled results bit-identical). A file changed between inspecting and installing is not installed. | `tests/plugin_pipeline.rs`, `lying_locality`, `honest_copy` |
| The store | The validated package file itself is kept and re-read through the same reader on every load, so a file altered on disk afterwards is refused. Identity is a content hash. Updates install beside the old version. State is written atomically. | `tests/plugin_pipeline.rs`, `tests/plugin_ipc.rs` |
| The document | A plugin filter in a document records the plugin's id, exact version, content hash and declared locality. A document whose locality disagrees with the installed plugin is refused. A plugin that is missing, off, ungranted, damaged or a different version is **reported, never replaced**. | `tests/plugin_pipeline.rs`, `src/App.plugins.test.ts` |
| Authority | Default none. Granted at install, **re-checked on every use**, revocable. A plugin command is a transaction with `origin: plugin` and the plugin's id on it, so a revoked grant stops it and locks are respected. | `tests/operations_engine.rs`, `tests/plugin_ipc.rs` |
| What a plugin changes | Through the transaction engine only: typed operation, validation, resource admission, an all-or-nothing transaction, the authoritative document validator after every step, one undo entry. A plugin that fails part-way leaves **no** layer and **no** pixel buffer behind. | `tests/operations_engine.rs`, `tests/plugin_ipc.rs` |

The adversarial modules are in `plugins/adversarial/` as WebAssembly text (25 of them)
and are compiled by the test suite, never shipped. Test plugins, adversarial fixtures
and caches are not part of any installer.

## What a plugin sees and can do

* It sees **the pixels of the window it is given** — the tile of a layer the person
  chose to run a filter on, plus the margin its declared locality needs — the size of
  the whole image, and its own parameters. It does not see other layers, the document
  structure, the file name, other plugins, the clock or anything on disk.
* It can return pixels, a non-zero error code, and up to 32 short log lines. Log lines
  are shown to the person; a plugin could use them to leak what it was given into a
  place the person can read. It has nowhere else to put it.
* Its commands can issue registered operations the manifest names — a fixed list,
  fixed at install, minus the four it may never issue — and only if the person
  granted `document.operations`.
* Its panels can show eight kinds of fact about the document. Its tools are menu
  entries. It cannot draw, receive input events, open windows or show a dialog.

## Network

**The plugin system adds no network access.** A plugin has no network capability to
be granted, PhotoForge never downloads, updates or checks for plugins, and there is no
marketplace or cloud registry. The person brings a file.

That is a statement about plugins, not about the application. PhotoForge's interface
runs in Microsoft **WebView2**, an operating-system component whose own network
behaviour (required diagnostic and configuration traffic, which Microsoft documents
an embedding application cannot switch off) is outside what this repository
controls. Observations of WebView2 opening TLS connections to Microsoft hosts, while
PhotoForge's own Rust process opened none, are recorded in `docs/privacy.md` and
`docs/webview-network-boundary.md`. **PhotoForge does not claim to have zero network
traffic**, and installing or running a plugin does not change that either way. What
PhotoForge does control — its own requests, and its window's navigation, download and
fetch policy — is described in those two documents.

## Residual risk

These are the things this design does **not** prevent. They are listed so that nobody
has to discover them.

1. **A bug in the engine is a bug in PhotoForge's process.** Wasmtime compiles the
   module to native code and runs it in-process. A flaw in Wasmtime or its code
   generator could let a malicious module run native code with the person's
   privileges. WebAssembly is not, by itself, a sandbox; the module is confined by the
   host giving it nothing. If the engine fails, **nothing else stands behind it** — no
   separate process, no AppContainer, no job object, no integrity level. Wasmtime
   describes continuous fuzzing and a security-response process; that is the project's
   own account and was not verified here. Moving plugins into a restricted child
   process would close most of this and is deferred (`docs/phase-14-results.md`).
2. **Side channels.** Wasmtime applies its standard mitigations, and no clock, no
   threads and no shared memory are offered, which removes the usual measuring tools.
   That is not a proof that no timing or speculative-execution channel exists, and none
   is claimed.
3. **A self-test samples; it does not prove.** A filter that is honest about locality
   on the test image can still behave differently on another — for example one that
   changes its answer when it sees an image of a particular size or position. The
   effect is bounded to wrong pixels in *that plugin's own output* (a visible seam, a
   wrong result), not to other layers, the document structure, or anything outside
   the sandbox. The output checks still apply.
4. **Denial of service is bounded, not eliminated.** A plugin may use up to its memory
   allowance (at most a quarter of the memory budget), and up to its work and time
   limits *per call*; across many tiles a hostile filter can make a render slow. The
   render can always be cancelled, and cancellation interrupts the module at its next
   loop back-edge, at 10 ms resolution. A module cannot hang the application, but it
   can make an edit take minutes.
5. **A plugin sees the pixels it is run on.** That is its purpose. What stops it
   sending them anywhere is the absence of anywhere to send them, not a promise.
6. **Trust in authorship is the person's.** An unsigned package from an unknown
   author is limited to what it was granted; it is not verified to do what its
   description says within that.
7. **Resource accounting is per call.** The memory cap is exact for the guest's linear
   memory; the host's own copies of tile windows and the engine's compiled code are
   not counted against it. Installed plugins hold their compiled modules in memory
   (see the measurements in `docs/phase-14-results.md`).

## Reporting

PhotoForge has no security contact in this repository. A problem found in the plugin
sandbox should be reported to the maintainer through the repository's issue tracker,
and a problem in Wasmtime to the Bytecode Alliance's published security process.
