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
- No AI model is loaded or downloaded by Phase 2.

The packaged application was observed with zero TCP connections during the Phase 1.1 startup/idle smoke test. That observation supports the source and CSP review; it is not a claim that Windows or WebView2 itself can never perform unrelated operating-system activity.

Development builds can print technical failures to local development tooling. User-facing errors contain concise messages rather than stack traces.

Phase 2 introduces no networking dependency, telemetry, remote resource, runtime download, or new filesystem capability. `analyze_image` is the only added Tauri command and reads only the already decoded active image.

Any future cloud integration must be visibly identified, disabled by default, and explicitly opted into. Any future local model download must show its source, size, resource requirements, and obtain approval before downloading.
