# Plugin specification

> **Two different formats are called "plugin" in PhotoForge.** This page and
> [`plugins/README.md`](../plugins/README.md) describe the **component manifest**
> (`schemaVersion` 1: a `planner` or `restoration_engine` that PhotoForge discovers,
> lists in Diagnostics and **never runs**). That format is unchanged. The executable
> `.photoforge-plugin` package introduced in 0.14.0 — WebAssembly filters, commands,
> panels and tools, installed from the Plugins manager — is a separate format with its
> own security model: see [plugins.md](plugins.md), [plugin-api.md](plugin-api.md),
> [plugin-package.md](plugin-package.md) and [plugin-security.md](plugin-security.md).
> The two are told apart by their first field: the component manifest has
> `schemaVersion`, the plugin manifest has `format`. A component manifest is never
> offered the authority a plugin package can be granted, and is still never executed.

PhotoForge 0.4.0 defines and validates a versioned JSON plugin manifest. It does **not** execute plugins. The authoritative field table and inert example are in [`plugins/README.md`](../plugins/README.md) and [`plugins/example-planner.json`](../plugins/example-planner.json).

## Phase 4 security contract

- Discovery is shallow and limited to 64 regular `.json` files.
- Each manifest is limited to 64 KiB and unknown fields are rejected.
- `schemaVersion` must be `1`; `type` must be `planner` or `restoration_engine`.
- Names, numeric semantic versions, providers, relative entries, memory metadata, and capability identifiers are bounded and validated.
- Absolute entries and parent traversal are rejected.
- Capabilities are descriptive metadata and grant no permission.
- Manifest entries are never opened, imported, loaded, spawned, evaluated, or executed.
- A valid record always returns `executionAllowed: false` and never becomes an installed provider.

The scan result contains its directory, a bounded list of per-file records, and a summary. Each record contains the path, validity, parsed manifest or validation error, and the false execution flag. Invalid records are shown in Component Diagnostics.

Adding executable third-party code is explicitly outside Phase 4. It requires a new threat model, permission and sandbox design, trust/signature policy, resource controls, and explicit user approval; the Phase 4 manifest scanner must not be treated as that execution system.
