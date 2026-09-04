# RAW development

## Current source boundary

PhotoForge 0.8.2 can now inspect a user-selected camera file without routing it
through the PNG/JPEG/WebP decoder. The `inspect_raw` Tauri command:

- recognises common extensions (`DNG`, `CR2`, `CR3`, `NEF`, `ARW`, `RAF`, `ORF`,
  `RW2`, and related families) case-insensitively;
- checks the file is a regular local file below the existing 750 MiB ceiling;
- checks DNG files for a little- or big-endian TIFF marker;
- hashes the immutable source with SHA-256 in 64 KiB chunks; and
- returns `recognizedDecoderUnavailable` when a format is known but no vetted
  decoder is bundled, or `unsupported` for unknown extensions.

Inspection never writes the source, launches a process, loads a plugin, or
contacts a network service. Dimensions are intentionally absent until a real
decoder has parsed and validated them. A random file renamed to `.dng` is
rejected rather than guessed at.

## Decoder research decision

The first candidate is the maintained pure-Rust [`rawloader`](https://github.com/pedrocr/rawloader)
project. Its documentation describes extraction of raw pixels, camera data,
black/white points, white-balance multipliers, a camera-to-XYZ matrix, and the
Bayer pattern, and its published support list includes DNG, CR2, NEF, ARW, RAF,
ORF, and RW2 families. It is LGPL-2.1 licensed. It does not list CR3 in its
support table. The [`rawloader` crate documentation](https://docs.rs/rawloader/latest/rawloader/)
also exposes a small decode API, but the dependency is not bundled in this
build until its exact version, Windows reproducibility, malformed-file
behaviour, and LGPL distribution obligations are reviewed in CI.

`libopenraw` currently exposes only thumbnail functionality through its Rust
crate and requires native C++ components for full decoding. `rawler`/dnglab
has broader format ambitions, including CR3, but its own project documentation
describes the API as unstable and warns against using it for untrusted files
when panics or aborts are unacceptable. LibRaw is mature and broad but is a
native C dependency. These trade-offs are recorded here so a decoder is not
added merely because a file extension is popular.

## Non-destructive document contract

`RawDevelopmentDocument` is a serialisable contract for the next project
schema. It keeps a user-approved linked-source filename, format, byte size,
SHA-256, original dimensions, development parameters, decoder identity/version,
and the linear-sRGB working-space identifier. Parameters are validated and
remain separate from legacy adjustment layers. The current `.photoforge`
schema still embeds RGBA8 layer PNGs and does not persist this source-backed
contract until a decoder and source-link policy are integrated. A separate,
validated `raw_development` operation is now available for raster previews and
workflows and is persisted wherever ordinary operations are stored; it is not a
camera RAW decoder.

When integrated, a RAW source must remain immutable and linked or embedded by an
explicit user choice. A missing or changed linked source must fail closed with a
clear recovery path; it must never silently substitute an 8-bit preview.

## Planned integration checklist

The remaining work is deliberately explicit:

1. vendor or depend on one vetted decoder and add corpus/fuzz tests for every
   claimed format;
2. parse camera/lens/orientation/WB metadata without making any field required;
3. demosaic through bounded cancellable preview/full-resolution workers;
4. persist development parameters and decoder identity in a versioned project
   extension;
5. route developed content through the existing layer/mask/workflow/batch
   boundaries; and
6. export from the float boundary with correct profile/metadata privacy policy.

Until those checks pass, PhotoForge reports RAW capability honestly as
metadata-only and keeps the ordinary raster workflow as the release path.
