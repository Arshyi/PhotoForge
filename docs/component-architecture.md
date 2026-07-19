# Component architecture

PhotoForge 0.5.0 separates guided planning and restoration execution behind typed, optional component boundaries while preserving the built-in Phase 3 behavior. Phase 5 installs the first optional planner adapter—Ollama—without adding model downloads, image inference, cloud calls, or executable plugins.

## Runtime flow

```text
Guided Edit UI
  ├─ Rule → generate_edit_plan → RulePlanner
  └─ Ollama → generate_ollama_plan → bounded local HTTP → strict wire schema
          → existing EditPlan validation
          → human review

Preview / export
  → ComponentRegistry.active_engine
  → RestorationEngineFactory
  → dyn RestorationEngine.process
  → existing deterministic typed pipeline
```

The Guided Edit UI selects a typed provider identity and calls the matching bounded command. Rule Planner implements the synchronous `EditPlanner` path. The Ollama factory identity advertises the same capability boundary while its real network work uses a cancellable async adapter command. Both produce the existing `EditPlan`. Preview and export depend only on `RestorationEngine` and never depend on a planner.

## Interfaces and capabilities

`EditPlanner` exposes provider identity, typed planner capabilities, and `create_plan(request, analysis)`. A planner returns the existing `EditPlan` schema and cannot directly modify pixels, paths, history, or application state.

`RestorationEngine` exposes provider identity, typed restoration capabilities, and `process(image, operations)`. An engine receives only a decoded image and the already validated ordered operation slice. It does not own image open/export policy.

Capabilities are strongly typed Rust/TypeScript objects. Planner capabilities describe guided-edit support, reasoning support, model requirements, and offline behavior. Restoration capabilities describe restoration/neural support, model requirements, offline behavior, alpha policy, and an input-size estimate. The UI derives badges from these fields rather than trusting free-form provider descriptions.

## Registry and factories

`ComponentRegistry` is the authoritative in-process state for the platform; the exported `PlannerRegistry` alias names its planner-facing role. It owns:

- registered planners and restoration engines;
- active provider selections;
- installed, loaded, active, and unavailable status;
- provider/version/memory metadata and capabilities;
- initialization and plugin-validation failures;
- local endpoint, model-directory, plugin-directory, and timeout configuration.

Factories return trait objects for every declared provider. The built-in `RulePlanner` and `DeterministicEngine` are functional. Ollama is registered as an installed, lazy optional adapter; its synchronous trait entry directs callers to the cancellable async command. OpenAI, ONNX, Real-ESRGAN, and unassigned future implementations advertise intended capabilities and return typed not-installed errors.

Only the Rule Planner and Deterministic Engine are loaded at startup. Selecting Ollama records its lightweight adapter identity but makes no connection and loads no model. Failed explicit Ollama operations leave image editing available and record a concise diagnostic. Inactive optional component identities are removed from the loaded set, providing an explicit unload path for future resource-owning implementations.

## Configuration

Settings are stored in `components.json` under the local PhotoForge application-data directory. Parsing is bounded to 32 KiB. Unknown or malformed settings are rejected and safe defaults remain active. Rule or Ollama may be persisted as the planner identity; the Deterministic Engine remains the only available engine.

The planner endpoint defaults to `http://127.0.0.1:11434`. Phase 5 permits only an explicit HTTP loopback address and never connects automatically. Test Connection queries `/api/version`; Refresh Models queries `/api/tags`; generation posts a deterministic prompt to `/api/generate`. The client disables proxies, redirects, and retries; applies timeouts and streamed size bounds; validates UTF-8/status; and supports request cancellation.

Model discovery lists only models already installed in Ollama and reads name, byte size, modified date, and capabilities when returned by the API. It never pulls or installs a model. An empty list uses the exact message `No compatible local models found.`

The existing engine-model file discovery remains separate. It examines only files in up to eight explicitly configured local directories. It recognizes model-like extensions and reads filesystem metadata; it never reads model contents or loads a runtime. Every discovered item is marked incompatible until an engine is installed, and an empty scan reports `No compatible models found.`

## Plugin boundary

The `plugins/` directory defines a versioned JSON manifest format. The scanner reads at most 64 shallow `.json` files, bounds each to 64 KiB, rejects unknown fields, validates identifiers/versions/relative entries/capability counts, and records each result for diagnostics.

Manifest validation does not imply trust or installation. `executionAllowed` is always false, manifest entries are never opened, and no dynamic library, process, script, model, or command is executed. See `plugins/README.md` for the format.

## Diagnostics and performance

The Diagnostics settings page reports app version, configuration path, registered planners/engines, loaded and unavailable components, initialization failures, plugin validation errors, and bounded Ollama state/timings/counters without prompt or image data.

An explicit local measurement action samples registry lookup, real rule-planner dispatch (including plan construction/validation), and component factory creation. It performs no model load, plugin execution, or network access. The measurement is diagnostic evidence on the current machine, not a portable benchmark.

## Adding a future implementation safely

1. Implement the relevant trait in `src-tauri/src/components`.
2. Declare truthful typed capabilities and bounded memory expectations.
3. Register it in the matching provider factory and registry metadata.
4. Add an explicit initialization path with timeout and typed errors.
5. Preserve plan validation or operation validation at the Rust trust boundary.
6. Make network, download, or model-loading behavior explicit and opt-in; Phase 5 authorizes only the documented loopback Ollama requests.
7. Add unit, integration, UI, offline, resource, packaging, and failure tests before marking it installed.

Do not expand the manifest scanner into an arbitrary code loader. A future executable plugin system requires a separate threat model, signature/trust policy, sandbox boundary, permission model, and explicit user approval.
