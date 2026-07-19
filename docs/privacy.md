# Privacy

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
- No AI model is loaded or downloaded by Phase 4.
- Guided requests are matched locally by fixed Rust rules. They are not hidden prompts and are never sent to a model or service.
- Remembered guided requests are optional, limited to 25 strings in local WebView storage, and cleared when prompt history is disabled. They contain no image pixels or generated analysis payload.

The packaged application was observed with zero TCP connections during the Phase 1.1 startup/idle smoke test. That observation supports the source and CSP review; it is not a claim that Windows or WebView2 itself can never perform unrelated operating-system activity.

Development builds can print technical failures to local development tooling. User-facing errors contain concise messages rather than stack traces.

Phase 4 introduces no networking dependency, telemetry, remote resource, runtime download, or executable plugin system. `generate_edit_plan` still reads only a bounded request string and cached analysis and now resolves the built-in planner through `EditPlanner`. Preview and export resolve the built-in deterministic processor through `RestorationEngine`; the pixel and path boundaries are unchanged.

Component configuration is stored locally in `%LOCALAPPDATA%\PhotoForge\components.json`. It contains selected provider identifiers, the loopback endpoint text, timeout, and user-configured model/plugin directories; it contains no image pixels, prompt history, model content, or secrets. Malformed configuration falls back to built-in providers.

The default Ollama endpoint is displayed as `http://localhost:11434`, but PhotoForge never connects automatically. The explicit Phase 4 **Test Connection** action also performs no network operation because the adapter is not installed. Ollama/OpenAI and ONNX/Real-ESRGAN entries are capability-declaring placeholders only.

Local model discovery reads file names, extensions, sizes, and paths in explicitly configured directories. It does not read or download model content and marks every result unavailable. Plugin scanning reads bounded JSON manifests for validation; manifest entries are never opened or executed and `executionAllowed` is always false.

Any future cloud integration must be visibly identified, disabled by default, and explicitly opted into. Any future local model download must show its source, size, resource requirements, and obtain approval before downloading.
