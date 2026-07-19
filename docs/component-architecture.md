# Component architecture

PhotoForge 0.4.0 separates guided planning and restoration execution behind typed, optional component boundaries while preserving the built-in Phase 3 behavior. The release is an architecture milestone: it does not add AI inference, model downloads, cloud calls, or executable plugins.

## Runtime flow

```text
Guided Edit UI
  → generate_edit_plan
  → ComponentRegistry.active_planner
  → PlannerFactory
  → dyn EditPlanner.create_plan
  → EditPlan validation
  → human review

Preview / export
  → ComponentRegistry.active_engine
  → RestorationEngineFactory
  → dyn RestorationEngine.process
  → existing deterministic typed pipeline
```

The Guided Edit UI depends only on `EditPlanner`. It never imports or selects a concrete rule, Ollama, or cloud implementation. Preview and export depend only on `RestorationEngine`. Both routes resolve the active provider immediately before use, so the interface boundary is exercised by the real application rather than existing only as unused scaffolding.

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

Factories return trait objects for every declared provider. The built-in `RulePlanner` and `DeterministicEngine` are functional. Ollama, OpenAI, ONNX, Real-ESRGAN, and unassigned future implementations compile, advertise their intended capabilities, and return a typed not-installed error. They contain no network or inference implementation.

Only the Rule Planner and Deterministic Engine are loaded at startup. Optional providers use bounded asynchronous initialization with a configurable timeout. Failed initialization leaves the current provider active and records a concise diagnostic. Inactive optional component identities are removed from the loaded set, providing an explicit unload path for future resource-owning implementations.

## Configuration

Settings are stored in `components.json` under the local PhotoForge application-data directory. Parsing is bounded to 32 KiB. Unknown or malformed settings are rejected and the safe defaults remain active. Only the built-in providers can be persisted as active in 0.4.0.

The planner endpoint defaults to `http://localhost:11434`. Phase 4 permits only an explicit loopback address and never connects automatically. The **Test Connection** action is user-initiated and currently returns `Planner not installed. No connection attempted.` without opening a socket.

Model discovery examines only files in up to eight explicitly configured local directories. It recognizes model-like extensions and reads filesystem metadata; it never reads model contents or loads a runtime. Every discovered item is marked incompatible until an engine is installed. No results use the exact message `No compatible models found.`

## Plugin boundary

The `plugins/` directory defines a versioned JSON manifest format. The scanner reads at most 64 shallow `.json` files, bounds each to 64 KiB, rejects unknown fields, validates identifiers/versions/relative entries/capability counts, and records each result for diagnostics.

Manifest validation does not imply trust or installation. `executionAllowed` is always false, manifest entries are never opened, and no dynamic library, process, script, model, or command is executed. See `plugins/README.md` for the format.

## Diagnostics and performance

The Diagnostics settings page reports app version, configuration path, registered planners/engines, loaded and unavailable components, initialization failures, and plugin validation errors.

An explicit local measurement action samples registry lookup, real rule-planner dispatch (including plan construction/validation), and component factory creation. It performs no model load, plugin execution, or network access. The measurement is diagnostic evidence on the current machine, not a portable benchmark.

## Adding a future implementation safely

1. Implement the relevant trait in `src-tauri/src/components`.
2. Declare truthful typed capabilities and bounded memory expectations.
3. Register it in the matching provider factory and registry metadata.
4. Add an explicit initialization path with timeout and typed errors.
5. Preserve plan validation or operation validation at the Rust trust boundary.
6. Make network, download, or model-loading behavior explicit and opt-in; none is authorized by the Phase 4 design.
7. Add unit, integration, UI, offline, resource, packaging, and failure tests before marking it installed.

Do not expand the manifest scanner into an arbitrary code loader. A future executable plugin system requires a separate threat model, signature/trust policy, sandbox boundary, permission model, and explicit user approval.
