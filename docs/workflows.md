# Workflows

Workflows are reusable, local, typed edit pipelines introduced in PhotoForge 0.6.0. Recording a workflow copies the current operation list; it never stores image pixels or source paths.

## Library and editor

The workflow library supports save, rename, duplicate, delete, favorite, search, folders, JSON import/export, and deterministic replay. The editor can reorder, delete, duplicate, insert through JSON, and adjust any typed operation parameter. Applying or previewing a workflow commits an ordinary undoable pipeline.

The built-in library is stored in the application WebView's local storage under a versioned key and is bounded to 250 workflows. A workflow contains at most 200 operations. Local storage failures fall back to an empty library without affecting image editing.

## Versioned JSON

Exports use this envelope:

```json
{
  "schemaVersion": 1,
  "workflow": {
    "id": "restore-old-scan",
    "name": "Restore Old Scan",
    "description": "",
    "folder": "Restoration",
    "favorite": true,
    "operations": [
      { "type": "crop", "x": 0, "y": 0, "width": 1, "height": 1, "aspect_ratio": "original", "overlay": "rule_of_thirds" },
      { "type": "auto_white_balance", "strength": 0.5 },
      { "type": "levels", "input_black": 4, "input_white": 248, "gamma": 1.05, "output_black": 0, "output_white": 255 }
    ],
    "createdAt": "2026-07-20T00:00:00.000Z",
    "updatedAt": "2026-07-20T00:00:00.000Z"
  }
}
```

The Rust import boundary caps files at 2 MiB, validates the schema version, validates every operation and parameter, and rejects unknown operation types. Unknown envelope fields are ignored for forward compatibility, but unknown schema versions are rejected rather than guessed. Export uses a temporary sibling file followed by a rename.

Workflow JSON is data only. PhotoForge never evaluates scripts, loads plugins, follows paths from the workflow, or executes external programs.
