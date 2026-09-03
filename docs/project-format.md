# PhotoForge project format

PhotoForge 0.8.0 introduces a real editable document format. A project stores
the layer tree, not a flattened image.

```text
Extension:      .photoforge
Container:      PhotoForge Container v1 (magic "PFORGE\r\n")
Format version: 1
```

## Why not ZIP

The suggested layout for a project of this kind is an archive with a manifest
and directories for layers and masks. PhotoForge implements exactly that
structure, but as its own bounded container rather than a ZIP file.

The reason is the threat model. A project file is untrusted input, and a general
archive format brings decompression bombs, entry-name traversal, duplicate
entries, and multi-gigabyte declared sizes along with it. PhotoForge's container
has no general-purpose decompression stage at all: every stored payload is a PNG
whose dimensions are declared in the manifest, checked before decoding, and
decoded through the same bounded decoder the image importer already uses. There
is nothing that can expand without limit. It also adds no new dependency.

The trade-off is honest: a `.photoforge` file cannot be opened with a ZIP tool.
It is a PhotoForge format, not an interchange format.

## Byte layout

All integers are little-endian.

```text
offset  size  field
0       8     magic  "PFORGE\r\n"
8       4     u32  container format version (1)
12      8     u64  manifest length in bytes
20      N     manifest JSON (UTF-8)
+       4     u32  entry count
        ...   entries
+       8     u64  FNV-1a-64 of every byte before this trailer
```

Each entry:

```text
2   u16  name length
N        name (UTF-8)
1   u8   encoding: 0 = raw, 1 = PNG
8   u64  payload length
8   u64  FNV-1a-64 of the payload
N        payload
```

The trailing CR/LF inside the magic makes a file mangled by a text-mode transfer
fail immediately instead of decoding into nonsense.

## Manifest

```json
{
  "formatVersion": 1,
  "application": "PhotoForge",
  "applicationVersion": "0.8.0",
  "createdAt": "...",
  "modifiedAt": "...",
  "document": { "schemaVersion": 1, "canvasWidth": 0, "canvasHeight": 0,
                "layers": [], "activeLayerId": null },
  "masks":  [ { "layerId": "...", "entry": "masks/....png",
                "width": 0, "height": 0, "enabled": true, "inverted": false } ],
  "pixels": [ { "pixelId": "...", "entry": "layers/....png",
                "width": 0, "height": 0 } ],
  "documentOperations": [ ]
}
```

The manifest carries the complete layer tree **with masks removed**; coverage
bitmaps live in their own PNG entries so the manifest stays small enough to
parse eagerly. Masks are reattached to their layers on load, and a mask
referencing an unknown layer is rejected.

`documentOperations` preserves the document-level pipeline: global adjustments
plus crop, straighten, perspective, and lens correction.

The manifest is parsed with `deny_unknown_fields`, so an unexpected key is a
rejection rather than something silently ignored.

## What a project preserves

Canvas dimensions; the full layer tree including nesting; every layer's stable
identifier, name, type, visibility, lock, opacity, blend mode, transform, mask
(with its enabled and inverted flags), collapse state, timestamps, and custom
metadata; adjustment-layer parameters; the active layer; the document operation
pipeline; the writing application version; and creation and modification times.

A project never stores only flattened pixels. Exporting a flattened image does
not flatten the project.

## Entry names

Entry names are container-internal labels; they are never used as filesystem
paths. They are nevertheless validated as if they were:

- no absolute root (`/name`) and no trailing slash
- no backslashes, so a Windows path or drive letter cannot appear
- no empty segments (`a//b`), no `.` and no `..` segments
- characters restricted to `A–Z a–z 0–9 . - _ /`
- at most 128 characters
- duplicate names rejected

Traversal is therefore structurally impossible *and* explicitly rejected, and
the rejection is tested with `../escape.png`, `/absolute.png`,
`layers/../../escape.png`, `layers\windows.png`, `C:/absolute.png`,
`layers//double.png`, and `layers/./same.png`.

## Bounds

| Limit | Value |
| --- | --- |
| Whole file | 1 GiB |
| Manifest | 32 MiB |
| Entries | 4,096 |
| Single entry payload | 256 MiB |
| Entry name | 128 characters |
| Decoded image | 20,000 px per side, 40 MP, 256 MiB decoder allocation |

Every structural field is bounds-checked against the bytes actually present
before any allocation follows it, so a declared length larger than the file
cannot drive a large allocation.

## Integrity

Two levels of checksum, both FNV-1a-64:

- each entry payload, checked before it is decoded
- the whole file before the trailer

A single flipped byte anywhere is detected. Tests corrupt bytes at four
positions across the file and assert each is rejected, and truncation is tested
at four cut points.

## Rejections

A project is rejected, safely and with a typed error, when it has a wrong magic,
an unsupported format version, a manifest larger than its ceiling, an unreadable
or unknown-field manifest, an out-of-range entry count, an entry declaring more
bytes than the file holds or than the ceiling allows, a failed checksum, trailing
data after the last entry, a disallowed or duplicate entry name, an unsupported
encoding byte, a missing referenced entry, an embedded image whose decoded size
disagrees with the manifest, impossible declared dimensions, a payload that is
not a readable image, a mask for an unknown layer, a document referencing pixels
the file omits, or a layer tree that fails validation.

Reading a project file causes **no** network access, executable loading, script
execution, plugin loading, or shell command. The format contains no code, path,
command, or URL field of any kind.

## Atomic saving

A save writes to a temporary file beside the destination, flushes it to disk,
and then renames it over the destination. A failure part way through therefore
never truncates an existing project. Encoding validates the document and pulls
pixel buffers from the session store *before* the temporary file is created, so
an invalid save fails before touching the filesystem. A test asserts that a
failed save leaves the previous project byte-identical and leaves no temporary
file behind.

Saving requires the `.photoforge` extension and an absolute local path without
parent traversal.

## Versioning and migration

`formatVersion` (the container) and `schemaVersion` (the layer tree) are
separate and both checked. A version newer than this build supports is rejected
with `unsupported_project_version` or `unsupported_layer_schema` rather than
being partially read.

Opening ordinary PNG, JPEG, and WebP images continues to work exactly as before
and produces a document with one background pixel layer. Phase 6 and 7 workflow
files, mask files, and selection sessions are unchanged and are still read by
0.8.0 — none of them are wrapped, rewritten, or migrated by the project format.

## Recovery

While a layer document has unsaved changes, 0.8.0 writes a recovery snapshot
every 90 seconds under `%LOCALAPPDATA%\PhotoForge\recovery`. The snapshot uses
the distinct `.photoforge-recovery` extension and contains an ordinary validated
project payload; a small JSON sidecar records its display name, timestamp, and
source project path when one exists. At most three snapshots are retained.

The project payload is written through a temporary file and synchronized before
it is persisted. A missing or unreadable sidecar is skipped, and a corrupt
snapshot is rejected by the same checksummed reader as an ordinary project. On
startup PhotoForge offers the newest readable snapshot. Recovered work remains
unsaved until the user saves it, while saving a project, accepting a recovery,
or explicitly discarding it removes the applicable recovery data. The user's
project file is never overwritten by recovery and nothing is uploaded.
