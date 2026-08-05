# Workflows

Workflows are reusable, local, typed edit pipelines introduced in PhotoForge 0.6.0 and extended with immutable mask snapshots in 0.7.0. Recording a workflow copies the current operation list; it never stores source image pixels or source paths.

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

The Rust import boundary caps files at 64 MiB, validates the schema version, validates every operation and parameter, and rejects unknown operation types. The larger ceiling permits bounded embedded masks while remaining below the standalone mask engine's allocation ceiling. Unknown envelope fields are ignored for forward compatibility, but unknown schema versions are rejected rather than guessed. Export checks the serialized size and uses a temporary sibling file followed by a rename.

## Masked operations

A Phase 7 workflow may wrap a mask-capable operation in a `masked` operation:

```json
{
  "type": "masked",
  "operation": { "type": "brightness", "value": 18 },
  "mask": {
    "version": 1,
    "width": 2,
    "height": 2,
    "encoding": "base64_u8",
    "data": "AP+A/w",
    "checksum": "fnv1a64:d865707bf628386d"
  },
  "invert": false,
  "mask_id": "subject"
}
```

The embedded snapshot is immutable and self-contained, so replay is independent of the currently selected named mask. Import validates its dimensions, encoding, decompressed length, checksum, and wrapped operation. Nested masked operations and geometry-changing masked operations are rejected. Preview may resample a mask only when image and mask aspect ratios agree; replay fails closed for incompatible dimensions.

Workflow JSON is data only. PhotoForge never evaluates scripts, loads plugins, follows paths from the workflow, or executes external programs. A mask snapshot is coverage data, not a source-image copy.
