# Ollama local planner provider

PhotoForge 0.5.0 adds an optional Ollama adapter for converting a user request and an approved image-analysis summary into an untrusted JSON edit proposal. Rule Planner remains the startup default. Ollama is never required to open, analyze, edit, preview, undo, redo, or export an image.

## Trust boundary

```text
User request + approved scalar analysis
  → deterministic prompt
  → local Ollama HTTP API
  → bounded UTF-8 response
  → strict wire-schema parsing
  → existing EditPlan validation
  → read-only inspection and human review
  → existing deterministic engine
```

Ollama cannot call image-processing functions, execute code or shell commands, browse files, access plugins, mutate history, select an export path, or apply a plan. The adapter returns data only. The existing deterministic engine remains the only component that modifies pixels.

## Explicit connection model

The default endpoint is `http://127.0.0.1:11434`. Only explicit HTTP loopback hosts are accepted: `127.0.0.1`, `localhost`, and `::1`. User information, paths, queries, fragments, HTTPS endpoints, redirects, proxy routing, and non-loopback hosts are rejected. PhotoForge opens no Ollama connection at startup or when the Components page is opened.

Network access occurs only after one of these user actions:

- **Test Connection** calls `GET /api/version`;
- **Refresh Models** calls `GET /api/tags`;
- **Generate Plan** calls `POST /api/generate`;
- **Compare Planners** runs Rule Planner locally and then calls `POST /api/generate` for the requested comparison.

No action downloads, installs, or pulls a model.

## HTTP implementation

`OllamaClient` uses a small asynchronous `reqwest` client configured per saved local settings. It has:

- an explicit connection timeout and total request/response timeout;
- redirects disabled;
- environment/system proxies disabled;
- no retry loop;
- success-status enforcement;
- `Content-Length` preflight and incremental response-size checks;
- a configurable 1 KiB–2 MiB response ceiling;
- explicit UTF-8 decoding before JSON parsing;
- typed errors for refusal, timeout, unreachable host, invalid endpoint, missing model, HTTP status, malformed JSON, schema failure, cancellation, planner busy, oversized response, and unsupported version.

The generation future is raced against a 20 ms cancellation watcher. Typing into the prompt, opening another image, changing the document, pressing Cancel, or issuing a newer request invalidates the generation. Dropping the HTTP future closes the in-flight work from PhotoForge's side, and stale responses are checked again before and after validation.

## Deterministic prompt

The prompt is a stable JSON serialization containing exactly five top-level fields:

1. `userRequest`;
2. `imageAnalysisSummary`;
3. `supportedOperations`;
4. `parameterRanges`;
5. `jsonSchema`.

The analysis summary contains deterministic brightness, luminance spread, color-cast estimate, noise, sharpness/blur estimate, local contrast, edge density, white-background ratio, and document likelihood. It contains no pixels, thumbnail, filename, path, image bytes, username, application configuration, operating-system data, or environment variables.

Generation sets `stream: false`, `temperature: 0`, and `seed: 0`, and supplies the strict JSON schema through Ollama's structured-output `format` field. These settings reduce randomness but do not make model output trustworthy; every response still passes the complete validation pipeline.

## Validation pipeline

The model-facing schema requires only `summary`, `confidence`, `warnings`, and `operations`. `additionalProperties` is false at the plan and operation levels. PhotoForge inspects and reports rejected field paths before deserialization, parses into a dedicated deny-unknown-fields wire type, converts only supported operations, creates operation explanations locally, and finally calls the existing `validate_edit_plan` function.

Validation rejects:

- missing or unknown fields and wrong JSON types;
- unknown operations or parameter names;
- non-finite or out-of-range values;
- empty or over-240-character summary/warnings;
- more than eight warnings;
- empty plans or plans over the configured one-to-eight operation limit;
- duplicate/conflicting operations;
- unsupported operation ordering;
- grayscale/saturation conflicts;
- oversized or non-UTF-8 responses.

The raw-response view is read-only. It shows the original response, locally normalized validated response, rejected field paths, validation errors, and measured validation time. A rejected plan cannot be applied. The UI offers **Use Rule Planner Instead**, but never switches silently.

## Model discovery and settings

Refresh Models reads installed-model metadata from `/api/tags`: name, byte size, modified date, and capabilities/details when available. The user selects one installed Planner Model and saves it locally. An empty response displays `No compatible local models found.`

Components exposes only the documented endpoint, timeout, maximum response bytes, selected model, maximum generated operations, and Reset Ollama defaults. Existing Phase 4 engine-model directories and plugin-manifest settings remain separate and do not participate in Ollama planning.

## Diagnostics and comparison

Diagnostics reports connected/disconnected state, last error, last response/connection/generation/validation/rule/comparison latency, selected model, adapter version, successful/rejected/validation-failure/cancelled counts, and the one-megabyte local-client memory estimate. It stores no prompt, analysis values, model response, image identifier, username, or endpoint in diagnostic counters.

Planner comparison displays each provider's summary, warnings, operations, confidence, error, and execution time. It deliberately chooses no winner and applies neither plan.

## Automated verification

The Rust suite includes a deterministic TCP mock server used only by tests. It covers version checks, empty and populated model lists, healthy generation, timeout, malformed and non-UTF-8 JSON, HTTP errors, redirects, oversized fixed and streamed bodies, missing models, and connection failure without requiring Ollama to be installed.

## Limitations

- PhotoForge does not install, start, update, or manage Ollama.
- A selected model may be slow, unsuitable for structured output, or unavailable later.
- Cancellation stops PhotoForge from waiting and accepting a result; Ollama may take a short time to notice the disconnected request.
- Only the non-streaming local generate API is supported.
- No remote LAN endpoint, TLS endpoint, cloud provider, tool calling, image input, model download, or automatic planner fallback is supported.
