# Privacy

PhotoForge is designed to work fully offline.

- Images are decoded and processed locally on the user's device.
- PhotoForge does not upload images or edit data.
- The initial version has no analytics, telemetry, crash reporting, or remote logging.
- Exported files are created only after the user explicitly chooses an output location.
- Original image files are not modified by default, and export rejects the original's canonical path.
- Interactive previews are local in-memory PNG data URLs; they are not sent to a service.
- No AI model is loaded or downloaded by Phase 1.

Development builds can print technical failures to local development tooling. User-facing errors contain concise messages rather than stack traces.

Any future cloud integration must be visibly identified, disabled by default, and explicitly opted into. Any future local model download must show its source, size, resource requirements, and obtain approval before downloading.
