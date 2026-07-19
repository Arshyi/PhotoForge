# Phase 5 results

PhotoForge 0.5.0 adds Ollama as an optional, explicitly invoked local planning adapter. The Rule Planner remains the default and the existing Deterministic Engine remains the only component allowed to modify image pixels. PhotoForge works without Ollama and makes no connection at startup.

## Delivered architecture

- `OllamaClient` provides loopback-only HTTP for `/api/version`, `/api/tags`, and `/api/generate`. It disables proxies, redirects, and retries; bounds connect/total time and response bytes; validates HTTP status and UTF-8; and supports cooperative cancellation.
- `generate_ollama_plan` sends a deterministic five-section JSON prompt containing only the user's request, approved scalar image analysis, supported operations, parameter ranges, and the output schema.
- Ollama API envelopes tolerate newer optional server/model metadata. The nested edit-plan response remains strict: required fields, field types, operation names, parameter names/ranges, operation count/order/conflicts, confidence, summary/warning bounds, and unknown fields are all validated before a plan can be reviewed.
- Ollama returns proposals only. It receives no pixels, thumbnails, file paths, usernames, environment data, application configuration, plugin access, filesystem access, shell access, or edit authority.
- Failures remain visible and typed. The user must explicitly choose **Use Rule Planner Instead**; PhotoForge never silently changes providers.
- Request IDs, document IDs, a single planning gate, a 20 ms cooperative cancellation watcher, and frontend generation tokens reject stale responses and prompt-edit races.
- Components settings persist endpoint, timeout, response bound, selected model, and maximum generated operations. Closing Components refreshes Guided Edit in the same session.
- Guided Edit includes provider tags in history, comparison without an automatic winner, a read-only original/validated JSON viewer, rejected-field/error reporting, explicit validation, cancellation, diagnostics, and a dedicated Local AI Privacy page.

See [Ollama provider](ollama-provider.md), [Local AI Privacy](local-ai-privacy.md), and [Component Architecture](component-architecture.md) for the detailed contracts.

## Automated verification

Final gates completed on Windows on July 19, 2026:

| Gate | Result |
| --- | --- |
| `cargo fmt --all -- --check` | Passed on `E:\PhotoForge` |
| `cargo clippy --all-targets -- -D warnings` | Passed on `E:\PhotoForge` |
| `cargo test -- --test-threads=1` | 279 passed, 0 failed on byte-identical staging source |
| `npm run check` | 0 errors, 0 warnings on `E:\PhotoForge` |
| `npm test -- --run` | 147 passed, 0 failed on `E:\PhotoForge` |
| `npm run build` | Passed; 141 modules transformed |
| `npm run tauri build` | Portable executable, NSIS, and MSI produced on `E:\PhotoForge` |

The two direct `cargo test` attempts on the USB repository exceeded five minutes while linking the debug test binary and did not reach test execution. SHA-256 comparison confirmed 72 code/configuration files were byte-identical to the writable staging tree; its warm target completed all 279 tests in 2.12 seconds. The optimized release linker completed successfully on `E:` three times, and strict Clippy compilation passed directly on `E:`.

Rust coverage includes endpoint spoofing and scheme/path/query rejection, connection refused, timeouts, HTTP errors, redirect refusal, malformed UTF-8/JSON, fixed and streamed size limits, unsupported versions/models, current and newer model metadata, deterministic prompts, data minimization, every supported operation, unknown/missing/wrong/excess fields, parameter ranges, conflicts/order, diagnostics, persistence, cancellation/stale requests, comparison, Rule fallback, and the existing deterministic processing/export pipeline.

Frontend coverage includes provider selection and same-session refresh, unavailable states, explicit connection/discovery actions, settings/reset, diagnostics, comparison, read-only JSON, validation reports, cancellation, prompt history/provider tags, fallback, accessibility, Local AI Privacy, and existing editor behavior.

## Packaged and live validation

| Scenario | Result |
| --- | --- |
| Portable startup | Responsive 0.5.0 window in approximately 573 ms |
| Browser-rendered shell | Local-first badge and Phase 5 Guided controls rendered; 800×600 had no horizontal overflow |
| No automatic connection | Ten-second packaged idle observation had zero TCP connections |
| Ollama installed | Explicit Test Connection reached local Ollama 0.32.1 in 4.4 ms |
| Multiple models | Explicit Refresh Models found two installed models and displayed names, sizes, dates, families/formats, and capabilities |
| Newer Ollama metadata | Live 0.32.1 discovery initially exposed a decoder incompatibility; fixed and covered by regression test before the final build |
| No/one model | Deterministic mock covers empty, one, and metadata-bearing lists; empty result is exactly `No compatible local models found.` |
| Failures and cancellation | Deterministic mock/UI tests cover refusal, endpoint typo validation, timeout/slow response, HTTP error, oversize, unsupported model, malformed data, busy state, cancellation, and stale responses without retry |
| Large prompt | A 1,001-character request was rejected locally in 0.009 ms in the reproducible sample |
| Rule/deterministic regression | Full Rust/frontend suites cover Rule planning, apply validation, undo/redo, preview, export, and pixel determinism; a packaged local PNG also opened successfully |
| Privacy/settings | Packaged Local AI Privacy text and all required Components/Diagnostics controls were exposed through Windows accessibility |
| NSIS | Silent install exit 0, registered 0.5.0, installed app responsive, silent uninstall exit 0, registration and install directory removed |
| MSI | Package database reports ProductName `PhotoForge` and ProductVersion `0.5.0` |

Live real-model inference was not used as a release gate: the installed models were 8.9 GiB and 22 GiB, and inference time/output would not be deterministic. The fixed mock server is the authoritative automated planner-generation gate and requires no installed Ollama model.

## Performance and footprint

The deterministic local mock sample measured 3.466 ms connection, 2.937 ms generation HTTP round trip, 0.155 ms validation, 0.280 ms Rule planning, and 3.375 ms comparison. These values measure PhotoForge overhead, not neural inference.

After ten seconds, the packaged main process used 27.6 MiB working set / 5.1 MiB private memory. The nine-process application/WebView tree used 434.4 MiB / 322.2 MiB and held zero TCP connections. See [Performance](performance.md) for caveats and size comparisons.

## Release artifacts

| File | Bytes | SHA-256 |
| --- | ---: | --- |
| `PhotoForge-portable.exe` | 12,458,496 | `44EEB8EA25A4E79AAA105116B1FCD87371B83BB597FA499B1B3101FEF9298EB8` |
| `PhotoForge_0.5.0_x64-setup.exe` | 2,861,976 | `DFB197AD39A72C615F0436C4F0903C959A152CDDE0EEF27B2E09512674FD5F0E` |
| `PhotoForge_0.5.0_x64_en-US.msi` | 4,186,112 | `81879FFFC08F3140DE3BDB3C2AC269CBC9D75D94FD439EE59FA62E24AAD7E7A4` |

`release/SHA256SUMS.txt` was compared with freshly computed hashes and matched all three files.

## Known limitations and roadmap

- Ollama must be installed, running locally, and have a user-installed compatible model. PhotoForge never downloads or pulls one.
- Only explicit HTTP loopback endpoints are accepted. HTTPS, LAN, remote, path-bearing, authenticated, query, and fragment endpoints are rejected in Phase 5.
- Generation latency and output quality depend on the selected local model and hardware. PhotoForge enforces a bounded timeout and validates all output.
- Cancellation stops PhotoForge from accepting or waiting for a response; a model server may continue internal work after the client disconnects.
- Raw JSON is inspection-only. There is no arbitrary JSON editing, tool calling, plugin execution, or automatic application.
- Future phases may add richer local-provider compatibility, optional engine implementations, tiled processing, metadata preservation, and more restoration workflows. Cloud APIs, autonomous agents, automatic downloads, and LLM pixel editing remain out of scope unless a later phase explicitly changes the security model.

The Deterministic Engine remains the only component that modifies image pixels. Ollama can only propose JSON that must pass PhotoForge validation and user review.
