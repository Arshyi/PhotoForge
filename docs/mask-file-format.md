# PhotoForge mask file format

PhotoForge mask files are local UTF-8 JSON documents conventionally named `*.photoforge-mask.json`. Version 1 is data only: it contains no path, URL, code, script, or remote reference.

```json
{
  "format": "photoforge-mask",
  "version": 1,
  "id": "2e6c9d0e-9fd7-4a54-9fd5-16454042d1c1",
  "name": "Foreground",
  "mask": {
    "version": 1,
    "width": 4000,
    "height": 3000,
    "encoding": "base64_rle_u8",
    "data": "...",
    "checksum": "fnv1a64:7e522b7298047785"
  },
  "metadata": {
    "createdAt": "2026-08-05T12:00:00.000Z",
    "modifiedAt": "2026-08-05T12:04:00.000Z",
    "sourceTool": "polygon"
  }
}
```

## Coverage and encodings

Coverage is row-major, left-to-right then top-to-bottom, one unsigned byte per pixel. Black/`0` means unselected, white/`255` means fully selected, and intermediate values are partial coverage.

`base64_u8` is the unpadded RFC 4648 Base64 encoding of the coverage bytes. `base64_rle_u8` is the unpadded Base64 encoding of repeated five-byte records:

```text
byte 0: coverage value
bytes 1–4: unsigned little-endian run length
```

Runs follow row-major order and may cross row boundaries. Encoders select RLE only when it is smaller than raw coverage. Decoders reject zero-length runs, incomplete records, total runs that do not exactly equal `width × height`, and compressed input larger than the bounded raw representation.

`checksum` is `fnv1a64:` followed by 16 lowercase hexadecimal digits. FNV-1a 64-bit is calculated over decoded coverage bytes using offset basis `0xcbf29ce484222325` and prime `0x100000001b3`. This is corruption detection, not authentication or a cryptographic signature.

## Validation and safety

- Width and height must be positive and their checked product must not exceed 100,000,000 pixels.
- All multiplication, allocation, Base64, run-length, decoded-length, version, identity, and checksum checks happen before a mask is accepted.
- Files larger than the bounded encoded representation plus metadata are rejected.
- Import/export paths must be absolute local paths, cannot use URI syntax, UNC/network paths, or `..` traversal, and are chosen through native dialogs.
- Export creates a collision-resistant sibling temporary file with create-new semantics, flushes and synchronizes it, then replaces the destination. Cancellation or failure before replacement preserves any existing destination and removes the temporary file. A failure in one mask does not modify the source image.
- Unknown JSON fields are ignored for forward compatibility; an unknown format or version is rejected.

## Grayscale PNG interchange

PhotoForge also imports and exports grayscale PNG masks. PNG intensity maps directly to coverage: black is `0`, white is `255`, and intermediate values remain partial. Dimensions are inspected and bounded before decode. Other formats are rejected for mask interchange.

JSON and PNG interchange is request-scoped to the open document. File bytes, output rows, decoded coverage, checksum, and diagnostic scans report real numerical work units when their totals are known; parser and codec-only phases remain explicitly indeterminate instead of inventing percentages. Cancellation is checked between bounded chunks and before destination replacement, and stale document/request results are discarded.

## Version 0.7.1 compatibility

PhotoForge 0.7.1 does not change the standalone mask format: JSON masks and their embedded snapshots remain version 1, and Phase 7 files are loaded without rewriting them. A mask file describes coverage at one image stage; it intentionally does not contain a geometry pipeline or source-image pixels. Import therefore requires the mask dimensions to match the current stage. Once accepted into a document, subsequent supported geometry edits remap that in-memory mask transactionally; a later export writes the remapped coverage and new dimensions as another version-1 file.

The 0.7.1 selection-session schema is separate from this interchange format. Session schema 2 adds current-stage dimensions and a canonical geometry fingerprint so restored masks can be checked against the edit pipeline. This does not alter, wrap, or migrate `.photoforge-mask.json` files.
