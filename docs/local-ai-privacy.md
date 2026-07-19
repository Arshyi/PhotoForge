# Local AI privacy

PhotoForge is local-first. Ollama support is optional, disabled by default as the active planner, and restricted to a configured loopback HTTP endpoint.

## What is sent

PhotoForge contacts the configured local Ollama endpoint only after **Test Connection**, **Refresh Models**, **Generate Plan**, or **Compare Planners**. Plan generation sends:

- the text the user entered in Guided Edit;
- the approved deterministic scalar image-analysis summary;
- the supported-operation list and parameter ranges;
- the strict JSON response schema.

## What is never sent

- image pixels, thumbnails, or encoded image/file contents;
- source or export paths, filenames, or unrestricted metadata;
- usernames or account information;
- PhotoForge configuration other than using the configured endpoint as the destination;
- operating-system details or environment variables;
- plugin data, plugin manifests, local model-directory contents, or shell commands;
- analytics, telemetry, crash reports, or remote logs.

PhotoForge includes no cloud fallback and contacts no cloud service. Loopback validation, disabled proxies, and disabled redirects prevent the client from routing an Ollama request to a remote host through application configuration. Users remain responsible for software they place behind the configured local endpoint; a separately configured Ollama installation or reverse proxy has its own privacy and retention behavior.

## Prompt history and diagnostics

Prompt history is optional, capped at 25 entries, tagged `Rule` or `Ollama`, and stored in the local WebView profile. Turning history off clears it. Diagnostics retains only status, timings, counters, selected model name, adapter version, and a concise last error. It does not retain prompts, responses, analysis summaries, images, or personal identifiers.

## Pixel authority

Ollama returns an untrusted JSON proposal. PhotoForge validates it, shows it for review, validates the reviewed plan again, and sends only supported typed operations to the deterministic engine after the user presses Apply. Ollama never receives or modifies pixels and cannot bypass this boundary.

See [ollama-provider.md](ollama-provider.md) for the transport and validation design and [privacy.md](privacy.md) for the application-wide policy.
