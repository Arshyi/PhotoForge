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
- No AI model is loaded or downloaded by Phase 3.
- Guided requests are matched locally by fixed Rust rules. They are not hidden prompts and are never sent to a model or service.
- Remembered guided requests are optional, limited to 25 strings in local WebView storage, and cleared when prompt history is disabled. They contain no image pixels or generated analysis payload.

The packaged application was observed with zero TCP connections during the Phase 1.1 startup/idle smoke test. That observation supports the source and CSP review; it is not a claim that Windows or WebView2 itself can never perform unrelated operating-system activity.

Development builds can print technical failures to local development tooling. User-facing errors contain concise messages rather than stack traces.

Phase 3 introduces no networking dependency, telemetry, remote resource, runtime download, or new filesystem capability. `generate_edit_plan` reads only a bounded request string and the cached analysis for the active document. `validate_guided_plan` accepts only the typed plan schema. Neither command receives a path, performs filesystem work, touches pixels, or executes arbitrary content.

Any future cloud integration must be visibly identified, disabled by default, and explicitly opted into. Any future local model download must show its source, size, resource requirements, and obtain approval before downloading.
