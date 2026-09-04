# Phase 9 results — RAW development and high-precision colour

PhotoForge 0.9.0 adds genuine camera RAW ingestion. A DNG file can be opened
locally, demosaiced, developed non-destructively, saved as a PhotoForge
project, reopened with its development state, and exported at full resolution
as a true 16-bit PNG — without the original file ever being written to.

No cloud service, telemetry, account, generative feature, neural inference,
GPU renderer, or PSD support was added. No model, decoder, or camera profile is
downloaded at any point.

Baseline for the completion pass was `824df98`.

---

# Capability table

| Capability | Implemented | Tested | Limitation |
| --- | --- | --- | --- |
| DNG decode, uncompressed | Yes | Yes — real Canon file and synthetic fixtures at 8/10/12/14/16 bits | — |
| DNG decode, lossless JPEG | Yes | Yes — bit-identical to the uncompressed encoding of the same photograph | — |
| DNG decode, lossy JPEG | Refused by name | Yes | Stores demosaiced samples, not CFA |
| DNG tiled / LinearRaw | Yes | **No** | Implemented, no fixture available |
| CR2, CR3, NEF, ARW, RAF, ORF, RW2 | **No** | — | Recognised, not decodable. Every mature Rust decoder is LGPL/AGPL |
| X-Trans and non-Bayer CFA | Refused by name | Yes | Bayer only |
| Black-level normalisation, per CFA position | Yes | Yes | — |
| Highlight headroom above white level | Yes | Yes — on a real photograph | — |
| Demosaic — bilinear (preview) | Yes | Yes | — |
| Demosaic — Malvar-He-Cutler (final) | Yes | Yes — measured against ground truth on all four Bayer layouts | — |
| White balance — As Shot from camera metadata | Yes | Yes | — |
| White balance — Auto (grey world) | Yes | Yes | An estimate, labelled as one |
| White balance — Custom multipliers | Yes | Yes | — |
| White balance — Temperature/Tint | Yes | Yes | Offsets from as-shot, **not** absolute Kelvin |
| Camera colour matrix to linear sRGB | Yes | Yes | Absent matrix reported as not colour managed |
| Source-backed projects (linked) | Yes | Yes | — |
| Missing / changed source detection and relink | Yes | Yes | — |
| Embedded RAW in project | Schema only | — | Readable if a later release writes one; not produced here |
| Preview architecture (decimate + fast demosaic) | Yes | Yes — preview matches full render on a real photograph | — |
| Full-resolution 16-bit PNG export | Yes | Yes — precision proven beyond 8-bit representability | — |
| Open a DNG from the interface | Yes | Yes — routing tested; the packaged GUI flow is not | Only DNG is routed |
| RAW metadata panel | Yes | Yes | Shows only fields the file carried |
| Place a RAW as a layer in an existing document | **No** | — | The Open flow handles RAW; "Place image as layer" still uses the 8-bit loader |
| Sensor above 40 megapixels | **No** | Refusal tested | Application-wide pixel ceiling; see limitations |
| Float compositing | **No** | — | Deferred; boundary documented |
| Display P3 / Adobe RGB | **No** | — | Not offered rather than mislabelled |
| ICC profiles | **No** | — | Deferred to Phase 10 |
| Batch RAW development | **No** | — | Deferred |
| Packaged desktop GUI / Windows DPI | **No** | — | Unchanged from 0.8.2 |

---

# Completed

## Decoder choice

Every maintained Rust RAW decoder is copyleft: `rawloader`, `rawler`, and
`quickraw` are LGPL-2.1; `dng` and `zenraw` are AGPL-3.0; LibRaw is LGPL-2.1 or
CDDL-1.0 with a C++ build and a DLL to package. PhotoForge's README states that
all rights are reserved, so linking any of them into the shipped binary would
be a licence violation the repository owner has not chosen to make.

`rawkit` (MIT or Apache-2.0) is licence-compatible and has exactly the right
API shape, but it decodes **Sony ARW only** and no ARW fixture was available to
test against. Support that cannot be tested is not support, so it was not
added; it remains the recommended path for ARW later.

DNG is a published specification, so PhotoForge reads it directly. The full
comparison is in `docs/raw-development.md`.

## Real decoding

`raw::tiff` is a bounded TIFF/IFD reader. `raw::ljpeg` is a lossless JPEG
(SOF3) decoder — the compression Adobe's DNG Converter and every DNG-shooting
camera actually writes. `raw::dng` turns those into a sensor image with its
CFA pattern, per-position black levels, white level, active area, default crop,
orientation, as-shot neutral, colour matrix, and capture metadata.

**Validated against real camera output.** Adobe DNG Converter publishes the
same Canon EOS 5D Mark III photograph uncompressed and with lossless JPEG. Both
decode to bit-identical sensor data — all 23,384,000 samples, checked
individually and by checksum. One file exercises the packed-sample reader and
the other the entropy decoder; they can only agree if both are exactly right.

The file's own values are read, not assumed: a white level of **15000** rather
than a power of two (a decoder assuming 65535 would develop it more than two
stops too dark), and a black level of **2047 on three CFA positions and 2048 on
the fourth**, which averaging would turn into a shadow colour cast.

## Development graph

Fixed in code, not assembled from interface order:

```
RAW bytes -> decode -> black level -> normalise -> white balance (on the CFA)
  -> demosaic -> camera RGB to linear sRGB -> exposure -> tone -> working image
  -> display or export transform  (the first and only clipping boundary)
```

White balance runs before demosaicing so unbalanced channels are never
interpolated together. Nothing clips before the final transform, which is what
makes highlight recovery real: the Canon sample has 305 pixels above the white
level after development, and one stop down brings them back.

## Source-backed projects

`Layer.raw` records the source file, its SHA-256, the development parameters,
and the camera metadata. The raster in a project is a cache; reopening develops
the photograph again. The field defaults to absent, so pre-0.9.0 projects load
unchanged.

Linked sources distinguish **available**, **missing**, and **changed**. A
relink verifies the hash and refuses a different photograph with a reason,
because silently binding a project to someone else's picture is worse than
failing.

## Performance

Measured on this machine — Windows 11, `cargo run --release`. Nothing here is
estimated.

Synthetic sensors, uncompressed:

| Sensor | Decode | Initial preview | Exposure edit (preview) | Full render | 16-bit PNG encode |
| --- | --- | --- | --- | --- | --- |
| 6 MP (3000x2000) | 3.9 ms | 35.7 ms | 36.2 ms | 141 ms | 335 ms |
| 12 MP (4240x2832) | 8.0 ms | 41.5 ms | 40.1 ms | 282 ms | 671 ms |
| 24 MP (6000x4000) | 15.6 ms | 61.7 ms | 62.1 ms | 586 ms | 1327 ms |
| 45 MP (8256x5504) | — | — | — | — | — (refused, see limitations) |

Real Canon EOS 5D Mark III, 5920x3950 (23.4 MP), lossless JPEG:

| Operation | Time |
| --- | --- |
| Decode (entropy decoding) | 515 ms |
| Decode, same shot uncompressed | 15 ms |
| Initial preview | 60 ms |
| Exposure edit (preview) | 62 ms |
| Full render | 554 ms |
| 16-bit PNG encode (135 MB) | 1571 ms |

The interactive numbers are the ones that matter: an edit costs about **60 ms**
on a 24-megapixel file, because the preview path decimates the sensor by whole
2x2 blocks before demosaicing. Full-resolution work is reserved for export.

Entropy decoding dominates the compressed case — 515 ms against 15 ms for the
same photograph uncompressed. It happens once per open, not per edit.

## Memory

Counted, not guessed, for one full render at 24 MP:

| Buffer | Bytes |
| --- | --- |
| Sensor samples (`u16`) | 48 MB |
| Normalised CFA (`f32`) | 96 MB |
| Demosaiced RGB (`f32` x3) | 288 MB |
| Working image (`f32` RGBA) | 384 MB |
| Encoded 16-bit PNG | 95 MB |
| **Peak** | **911 MB** |

Preview work is a fraction of this because decimation happens before any
per-pixel allocation. The 6 MP figure is 232 MB and the 12 MP figure 462 MB, so
the cost scales linearly with sensor area as expected.

## Security

A RAW file is treated as hostile input. Bounded IFD count, nesting depth,
entries per IFD, value length and count; every offset checked against the
buffer; a self-referencing IFD chain terminates on a walk budget; lossless-JPEG
frame geometry bounded before allocation; black and white levels validated;
strip and tile tables bounded.

Tested against empty files, random bytes, truncated files, header-only files, a
JPEG named `.dng`, all-zero and all-ones buffers, absurd declared dimensions,
strips pointing outside the file, invalid CFA codes, and invalid levels. Every
prefix of a valid file is decoded, so truncation at any byte fails rather than
panics. No RAW file causes a network request, process launch, or library load.

Error messages are written for people. Tests assert that no stack trace,
pointer, or oversized diagnostic reaches the interface.

## Tests

| Suite | Before | After |
| --- | --- | --- |
| Rust unit | 746 | 830 |
| Rust layer IPC | 39 | 39 |
| Rust RAW IPC | 0 | 21 |
| Rust real-file (optional, offline-skipped) | 0 | 6 |
| Frontend (vitest) | not claimable | 851 |

The frontend toolchain failure reported in the previous pass did not reproduce:
`npm test`, `npm run check`, `npm run build`, and `npx tsc --noEmit` all
complete cleanly.

Real camera fixtures are CC0 but too large to commit.
`scripts/fetch-raw-fixtures.ps1` obtains them on request, and the tests that
use them skip themselves when it has not been run, so **normal test runs stay
entirely offline**.

---

# Still incomplete and unverified

1. **Only DNG is decoded.** CR2, CR3, NEF, ARW, RAF, ORF, and RW2 are
   recognised so the interface can explain itself, but no decoder is bundled.
   This is a licensing constraint, not a technical one: the mature Rust
   decoders are LGPL/AGPL and the repository reserves all rights. Users of
   other cameras must convert with Adobe DNG Converter first.

2. **Sensors above 40 megapixels are refused.** PhotoForge has an
   application-wide 40,000,000-pixel decompression-bomb ceiling from Phase 1,
   shared by every decode path. That excludes 45 MP and 61 MP cameras. Raising
   it is a change to the security posture of the whole application, not a RAW
   change, so it was not made inside this phase. The refusal is clean and
   tested; it is not a crash.

3. **A RAW layer inside a layered document is quantised to 8 bits.** The
   development is high precision and the 16-bit export bypasses the compositor
   entirely, but the layer pixel store holds 8-bit rasters. Documented in
   `docs/color-pipeline.md`.

4. **Display P3 and Adobe RGB are not implemented** and are therefore not
   offered. **ICC profiles are not read.**

5. **Batch RAW development was not built.** The single-file path is complete;
   applying a development preset across many files is not.

6. **Tiled DNG and LinearRaw DNG are implemented but untested** — no fixture
   was available.

7. **Packaged desktop GUI behaviour and the Windows DPI matrix are unverified**,
   unchanged from 0.8.2. The 0.8.2 pass drove the packaged binary through its
   WebView2 debugging port; that was not repeated for RAW because the RAW
   interface surface added here is the command layer and its metadata panel,
   not new canvas interaction.

8. **The MSI lifecycle is unverified** — it is an all-users package requiring
   elevation, and UAC was not bypassed.

9. **No code signing.** All artifacts are `NotSigned`; no CA-issued identity or
   Trusted Signing account exists. No self-signed certificate was created.

10. **No zero-network claim is made.** As in earlier phases, the embedded
    WebView2 runtime performs its own diagnostics the application does not
    control. PhotoForge's own code makes no network request during RAW work.

---

# Deferred to Phase 10 and later

- Additional RAW formats, either by relicensing PhotoForge so an LGPL decoder
  can be linked, or by adding `rawkit` for ARW once a fixture exists
- Float compositing, which would remove the 8-bit layer boundary
- ICC profile support, with `qcms` (MPL-2.0, pure Rust) as the assessed
  starting point
- Display P3 and Adobe RGB conversions
- Batch RAW development
- Raising the application-wide pixel ceiling for high-resolution sensors, with
  the memory implications measured across every decode path
- GPU rendering, PSD, text and vector layers, plugins, and generative tools —
  all still explicitly out of scope
