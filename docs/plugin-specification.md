# Plugin specification

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
