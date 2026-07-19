# Phase 4 results

PhotoForge 0.4.0 introduces typed, replaceable planner and restoration-engine boundaries while retaining the Rule Planner and Deterministic Engine as the only installed defaults. The Guided Edit command now resolves `dyn EditPlanner`; preview and export resolve `dyn RestorationEngine`. Phase 3 requests, review, validation, history, deterministic processing, and export behavior remain intact.

## Delivered architecture

- `ComponentRegistry` owns registrations, typed capabilities, active providers, loaded/unavailable state, memory/version/provider metadata, configuration, and diagnostics.
- `PlannerFactory` creates Rule, Ollama, OpenAI, and future planner trait objects.
- `RestorationEngineFactory` creates Deterministic, ONNX, Real-ESRGAN, and future engine trait objects.
- Ollama/OpenAI and ONNX/Real-ESRGAN/future implementations compile but are inert and return typed not-installed errors. There is no inference code.
- Optional initialization is lazy, bounded by a configurable 100–30,000 ms timeout, failure-safe, and represented in diagnostics. Inactive optional identities can be unloaded from registry bookkeeping.
- Component configuration persists locally with bounded parsing and safe fallback to Rule + Deterministic.
- Local model discovery reads shallow file metadata only. It performs no content load or download and reports `No compatible models found.` when empty.
- Versioned plugin manifests are validated without opening or executing their entries; `executionAllowed` remains false.

See `component-architecture.md` and `plugin-specification.md` for the detailed boundaries.

## Automated verification

All final commands completed on Windows on July 18, 2026:

| Gate | Result |
| --- | --- |
| `cargo fmt --all -- --check` | Passed |
| `cargo clippy --all-targets -- -D warnings` | Passed |
| `cargo test --workspace` | 192 passed, 0 failed |
| `npm run check` | 0 errors, 0 warnings |
| `npm test` | 88 passed, 0 failed |
| `npm run build` | Passed |
| `npm run tauri build` | Portable executable, NSIS, and MSI produced |

Rust coverage includes provider parsing, strict loopback endpoint validation, capabilities, factories, placeholder identities/errors, switching/fallback, registry snapshots, persisted configuration, timeouts, loading/unload bookkeeping, real planner/engine dispatch, manifest limits/traversal/unknown fields, local discovery, diagnostics, deterministic processing, history, export safety, and existing image workflows.

Frontend coverage includes registration/status/capability formatting, disabled unavailable choices, local endpoint display, explicit connection testing, settings persistence, model/plugin results, diagnostics, performance formatting, failure states, Guided Edit, plan review, restoration controls, history, image stage, and existing utilities.

## Packaged manual validation

| Scenario | Result |
| --- | --- |
| Default portable start | Responsive; Rule Planner and Deterministic Engine active and loaded |
| No plugins | Missing default plugin directory was safe; no plugin loaded or executed |
| Provider selection | Only installed choices enabled; six future choices visible and disabled |
| Unavailable planner request | Returned `Planner not installed.` and retained Rule as active |
| Ollama test action | Explicit click returned `Planner not installed. No connection attempted.` |
| Settings | Endpoint, model paths, plugin path, timeout, version/provider/memory/status/capability metadata rendered |
| Model discovery | Returned exact `No compatible models found.` with no download or inference |
| Diagnostics | Registered, loaded, unavailable, failures, plugin errors, config path, and 0.4.0 version rendered |
| Offline operation | Normal 10-second process-tree observation showed zero TCP connections |
| Guided Edit | “Reduce noise and sharpen slightly” proposed Denoise then Edge-Aware Sharpen through the planner interface |
| Apply/history | Apply produced one reviewed history commit with two typed operations |
| Undo/redo | Undo cleared the pipeline and enabled Redo; Redo restored both operations |
| Export | Packaged engine command exported 512×512 PNG in 35.85 ms; source SHA-256 stayed unchanged |
| Installer | NSIS exit 0, registered 0.4.0, installed app was responsive; uninstaller exit 0 and removed registration/directory |
| MSI | Built successfully and included in the hashed release set |

The history workflow identified and corrected a toolbar state-refresh defect before the final rebuild; the corrected packaged build was the one re-tested and hashed.

## Performance and default footprint

The explicit diagnostic used 250 samples. Registry lookup averaged below the Windows timer's per-sample resolution (displayed as `<1 ns` after the final UI formatting correction), real Rule Planner dispatch including plan creation/validation averaged 4.25 µs, and built-in factory/loading bookkeeping averaged 3 ns. No model, network connection, or plugin execution occurred.

Three alternating warm startup samples produced a 76 ms median for the untouched 0.3.0 portable baseline and 66 ms for 0.4.0. Median main-process working set was 22.2 MiB and 22.1 MiB; private memory was 3.5 MiB for both. Separate clean-session observations were 422 ms for 0.3.0 and 484 ms for 0.4.0; first-window timings are sensitive to WebView2 process reuse. A 10-second 0.4.0 idle observation used 26.5 MiB main working set / 4.9 MiB private memory and 78.1 ms CPU, with zero process-tree TCP connections.

The production frontend is 131.04 kB JavaScript and 32.20 kB CSS (42.15 kB and 6.74 kB gzip). No npm package, Cargo package, model, inference runtime, Python component, GPU requirement, or networking library was added. Only Tokio's already-present `time` feature was enabled for initialization timeout support.

## Release artifacts

| File | Bytes | SHA-256 |
| --- | ---: | --- |
| `PhotoForge-portable.exe` | 10,895,872 | `9EDFBF2F0C1049C1EE0861D4A642045F5017A065DE92CD1C8AE3F7DBC3C8862E` |
| `PhotoForge_0.4.0_x64-setup.exe` | 2,482,549 | `07BF1C06FE7B1A185A488FFDD45BE822B7F8530FF65D344CF24DC5B2F7763630` |
| `PhotoForge_0.4.0_x64_en-US.msi` | 3,641,344 | `8514BE4474AF3FE7A77E2AF51BF7BAA8519F489AE72E13C24510F933B8D1CD14` |

`release/SHA256SUMS.txt` contains the same final hashes.

## Honest scope

This release contains no Ollama inference or automatic connection, OpenAI/cloud request, ONNX inference, neural/super-resolution/face restoration, model download, arbitrary plugin execution, Python runtime, OCR, batch processing, perspective correction, hidden prompt, or telemetry. The declared future providers are architecture placeholders, not working AI features.
