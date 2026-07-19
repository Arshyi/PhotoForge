# PhotoForge plugin manifest specification

PhotoForge 0.4.0 validates plugin metadata only. It does not install, load, import, spawn, evaluate, or execute plugin entries. A valid manifest is still reported with `executionAllowed: false`.

## Location and discovery

The default directory is `%LOCALAPPDATA%\PhotoForge\plugins`; users may choose another local directory in **Settings → Components**. Discovery is shallow and deterministic:

- only regular `.json` files directly inside the directory are considered;
- paths are sorted and at most 64 manifests are inspected;
- each manifest is limited to 64 KiB;
- non-JSON files and nested folders are ignored;
- a missing directory is safe and loads nothing.

The repository's `plugins/` directory documents the format and provides an inert example. It is not copied into the application-data directory by the installer.

## Schema version 1

```json
{
  "schemaVersion": 1,
  "name": "Example Planner",
  "version": "1.0.0",
  "type": "planner",
  "provider": "example-local",
  "entry": "adapters/example-planner.disabled",
  "memoryEstimateMb": 128,
  "capabilities": ["guided_editing", "offline"]
}
```

| Field | Type | Validation |
| --- | --- | --- |
| `schemaVersion` | integer | Must be exactly `1`. |
| `name` | string | 1–80 characters after validation; must not be blank. |
| `version` | string | Two or three dot-separated numeric segments, each 1–6 digits. |
| `type` | enum | `planner` or `restoration_engine`. |
| `provider` | string | 1–80 ASCII letters, digits, `-`, `_`, or `.`. |
| `entry` | string | 1–260 characters; relative path only, with no root, prefix, or `..` traversal. |
| `memoryEstimateMb` | integer | Metadata only; 0–262,144 MiB. |
| `capabilities` | string array | Optional; at most 32 unique bounded identifiers using the provider character set. |

Unknown fields are rejected. Capability names are descriptive metadata and grant no permission. The `entry` path is validated as inert text and is never opened or executed.

## Result semantics

A scan returns the manifest path, validity, parsed manifest or error, and `executionAllowed: false`. Valid and invalid records appear in Diagnostics. Validation never registers a provider as installed and never makes it selectable.

Future versions may revise this format by introducing a new `schemaVersion`; they must not silently reinterpret version 1. Executable third-party components are outside Phase 4 and require a separate security design.
