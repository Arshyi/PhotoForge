# Batch processing

PhotoForge 0.6.0 can replay a saved workflow across PNG, JPEG, and WebP files in a local folder.

## Safety model

- Input and output folders must already exist and resolve to different canonical locations.
- Folder traversal ignores symbolic links and is capped at 10,000 supported files.
- Recursive discovery is opt-in.
- Workers are explicitly bounded from 1 to 8; the default is 2.
- Each worker decodes one image, applies the shared deterministic pipeline, exports it, and releases it before claiming another image.
- Duplicate generated output paths are claimed once. Existing outputs are skipped unless overwrite permission is explicitly enabled.
- Original paths remain protected by the same export boundary used by the interactive editor.
- Cancellation is cooperative and checked before each file. In-flight files finish safely; no new files are claimed.

## Preview and progress

Batch Preview discovers files, shows up to 12 sample output paths, reports existing-file skips, and estimates duration without writing output. A running batch exposes discovered, completed, skipped, failed, current-file, elapsed-time, and estimated-remaining fields. The UI polls local in-process status; it performs no network request.

## Naming and profiles

Templates accept `{name}`, `{index}`, `{ext}`, and `{workflow}`. Unsafe filename characters are replaced and the final name is bounded. Export profiles select JPEG for Web/Print/High JPEG, PNG for Archive/Lossless, and WebP for Maximum Compression.

Every non-preview batch writes `photoforge-batch-<id>.log` in the output folder. The log contains one tab-separated OK, SKIPPED, or FAILED row per processed input. The UI retains a bounded failure summary of the first 100 errors.
