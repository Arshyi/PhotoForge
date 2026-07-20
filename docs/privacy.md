# Privacy

## Phase 6 professional workflow privacy

Professional tools, histograms, metadata inspection, workflow recording, batch processing, workspace layouts, shortcuts, comparison, and export profiles operate entirely on this device. Workflows contain edit parameters only; they contain no image pixels. Workspace and shortcut settings use local WebView storage. Batch inputs, outputs, progress, failures, and logs stay in user-selected local folders and process memory.

Phase 6 adds no network dependency, telemetry, cloud processing, hidden upload, external execution, model, model download, or neural inference. Safe EXIF inspection reads only the bounded local input file and does not alter metadata. The optional Phase 5 Ollama loopback planner remains the only networking exception and professional tools never invoke it.

PhotoForge is designed to work fully offline.

- Images are decoded and processed locally on the user's device.
- PhotoForge does not upload images or edit data.
- The initial version has no analytics, telemetry, crash reporting, or remote logging.
- Exported files are created only after the user explicitly chooses an output location.
- Original image files are not modified by default, and export rejects the original's canonical path.
- Interactive previews are local in-memory PNG data URLs; they are not sent to a service.
- Preview data is not written to a temporary file.
- The webview has no shell or unrestricted filesystem capability; user-selected paths cross only three typed commands.
- Paths are canonicalized at the Rust boundary, supported formats are detected from content, and exports require an absolute user-selected image path.
- Restoration algorithms and quality analysis run locally on decoded pixels and validated numeric parameters.
- Analysis results remain in process memory, do not identify people or content, and are discarded when the document is replaced or the app exits.
- No AI model is bundled, loaded into PhotoForge, installed, or downloaded by Phase 5.
- Rule Planner requests are matched locally by fixed Rust rules. Ollama requests are sent only after an explicit user action to the configured loopback endpoint.
- Remembered guided requests are optional, limited to 25 provider-tagged entries in local WebView storage, and cleared when prompt history is disabled. They contain no image pixels or generated analysis payload.

The packaged application was observed with zero TCP connections during the Phase 1.1 startup/idle smoke test. That observation supports the source and CSP review; it is not a claim that Windows or WebView2 itself can never perform unrelated operating-system activity.

Development builds can print technical failures to local development tooling. User-facing errors contain concise messages rather than stack traces.

Phase 5 adds one optional HTTP dependency for a user-managed local Ollama service. It adds no telemetry, cloud resource, runtime download, or executable plugin system. `generate_edit_plan` remains the offline Rule Planner command. `generate_ollama_plan` reads only a bounded request string and cached approved analysis summary. Preview and export still resolve the built-in deterministic processor through `RestorationEngine`; the pixel and path boundaries are unchanged.

Component configuration is stored locally in `%LOCALAPPDATA%\PhotoForge\components.json`. It contains selected provider identifiers, loopback endpoint text, timeouts, response limit, selected model name, operation limit, and user-configured engine-model/plugin directories; it contains no image pixels, prompt history, model content, or secrets. Malformed configuration falls back to safe defaults.

The default Ollama endpoint is `http://127.0.0.1:11434`, and PhotoForge never connects automatically. **Test Connection**, **Refresh Models**, **Generate Plan**, and **Compare Planners** are the only user actions that open an Ollama request. The client permits explicit HTTP loopback hosts only and disables proxy discovery, redirects, and retries. OpenAI and ONNX/Real-ESRGAN entries remain capability-declaring placeholders.

Ollama generation receives only the user prompt, approved scalar analysis summary, supported operations, parameter ranges, and strict JSON schema. It never receives pixels, thumbnails, file contents, paths, filenames, usernames, PhotoForge configuration, operating-system data, environment variables, or plugin data. Raw responses are bounded and retained in UI memory only for read-only inspection; they are not added to diagnostics or prompt history.

Local model discovery reads file names, extensions, sizes, and paths in explicitly configured directories. It does not read or download model content and marks every result unavailable. Plugin scanning reads bounded JSON manifests for validation; manifest entries are never opened or executed and `executionAllowed` is always false.

Any future cloud integration must be visibly identified, disabled by default, and explicitly opted into. Any future local model download must show its source, size, resource requirements, and obtain approval before downloading. See [local-ai-privacy.md](local-ai-privacy.md) for the focused policy.
