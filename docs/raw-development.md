# RAW development

PhotoForge decodes camera RAW files locally and develops them without ever
writing to the original. This page says exactly which formats that covers, how
the decoder was chosen, and what the pipeline does in what order.

## Decoder choice (reviewed 2026-09-04)

The existing in-house DNG subset remains the only RAW decoder. Phase 10 does
not expand to proprietary formats. This is a scoped engineering/distribution
decision, not a claim about every Rust RAW library.

| Candidate / reviewed source | License stated by upstream | Capability / decision |
| --- | --- | --- |
| rawloader 0.37.1 Cargo manifest | LGPL-2.1 | Multi-camera sensor extraction; not integrated because a distribution/compliance choice and new fixture coverage remain outside this phase |
| rawler 0.8.0 Cargo manifest | LGPL-2.1 | Multi-format RAW extraction; same scope/distribution reason |
| LibRaw About page, reviewed 2026-09-04 (no version selected) | LGPL-2.1 or CDDL-1.0 | Broad RAW data/metadata extraction; not selected because native integration, packaging and distribution obligations were not resolved |
| PhotoForge DNG decoder version 2 | Repository distribution terms | Existing implementation retained and hardened; no additional decoder library |

Sources: [rawloader manifest](https://raw.githubusercontent.com/pedrocr/rawloader/master/Cargo.toml),
[rawler manifest](https://raw.githubusercontent.com/dnglab/dnglab/main/rawler/Cargo.toml),
[LibRaw licensing](https://www.libraw.org/about).
These license labels are not a legal conclusion that static linking is
automatically forbidden. The earlier blanket statement to that effect was
incorrect and has been removed. Other candidates listed in the older report
were not revalidated in this review and are not presented as current findings.

For ICC, a separate problem from RAW decoding, Phase 10 selects the pinned
pure-Rust moxcms 0.8.1 backend; see [color-pipeline.md](color-pipeline.md) and
the bundled third-party notice.

## Format capability

| Format | Status | Notes |
| --- | --- | --- |
| DNG, uncompressed | **Tested** | Verified against Canon EOS 5D Mark III output and synthetic fixtures at 8, 10, 12, 14, and 16 bits |
| DNG, lossless JPEG (compression 7) | **Tested** | Verified bit-identical to the uncompressed encoding of the same photograph |
| DNG, lossy JPEG (compression 34892) | **Refused by name** | Stores three demosaiced samples per pixel, not CFA data |
| DNG, tiled | **Synthetic format tests** | Multi-tile placement and partial edge tiles match exact samples; no real-camera tiled validation |
| DNG, LinearRaw photometric | **Synthetic monochrome test only** | One-channel linear samples bypass Bayer demosaic; three-channel LinearRaw is explicitly unsupported |
| DNG, deflate / packbits / VC-5 | **Unsupported** | Refused by compression number |
| CR2, CR3, NEF, ARW, RAF, ORF, RW2, and others | **Recognised, not decodable** | The extension is understood so the interface can explain itself; no decoder is bundled. Convert with Adobe DNG Converter to open them today |
| X-Trans and other non-Bayer CFAs | **Refused by name** | The demosaic implements Bayer only |

"Recognised" is never reported as supported. `inspect_raw` returns `decodable`
only for files this build can actually read, and the capability list contains
only DNG.

## The development graph

The order is fixed in `raw::develop`, not assembled from the order a user
happens to touch the controls:

```
RAW bytes
  -> decode sensor and metadata          raw::dng
  -> subtract black, scale to white      raw::demosaic::normalize
  -> white balance, on the CFA           raw::demosaic::apply_white_balance
  -> demosaic                            raw::demosaic::demosaic
  -> camera RGB to linear sRGB           raw::demosaic::apply_camera_matrix
  -> exposure, contrast, tone            color::apply_development
  -> working FloatImage (linear)
  -> display or export transform         first and only clipping boundary
```

Two ordering choices are deliberate:

**White balance happens before demosaicing.** Interpolating channels that are
still unbalanced mixes a strong green into a weak red and leaves colour fringes
on edges that nothing downstream can remove.

**Nothing clips before the display transform.** A photosite brighter than the
nominal white level is real captured signal. It survives normalisation, the
colour matrix, and the tone controls, which is the only reason reducing
exposure can bring a highlight back. This is verified on a real photograph:
the Canon sample has samples above its 15000 white level, they are still above
1.0 after development, and one stop down recovers them.

## Sensor description

Read from the file, never assumed:

- CFA pattern (RGGB, BGGR, GRBG, GBRG), rejected if not a 2x2 Bayer block
- Black level, per CFA position where the file gives one
- White level
- Bit depth (8, 10, 12, 14, 16)
- Active area and the camera's default crop
- Orientation
- As-shot neutral, and the colour matrix
- Camera make and model, lens, ISO, shutter, aperture, focal length, capture time

Real files justify each of these. The Canon sample records a white level of
**15000**, not a power of two — a decoder that assumed 65535 would develop it
more than two stops too dark. Its black level is **2047 on three CFA positions
and 2048 on the fourth**; averaging them leaves a colour cast in the shadows.

## Demosaicing

| Algorithm | Used for | Why |
| --- | --- | --- |
| Bilinear | Previews | Cheap, correct, and indistinguishable at preview size on a decimated sensor |
| Malvar-He-Cutler | Full renders and export | Gradient-corrected; measurably more accurate |

"High quality" is a measured claim, not a label. The test mosaics a known image
through each Bayer layout, demosaics it, and compares against the original:
Malvar-He-Cutler has lower mean squared error than bilinear on all four
layouts. The benchmark scene has correlated channels, as photographs do,
because that correlation is the assumption gradient correction relies on — on
an artificial image whose channels are unrelated, bilinear wins, and saying so
is part of describing the algorithm honestly.

## Preview architecture

A 45-megapixel sensor cannot be demosaiced from scratch on every pointer move.
Interactive work decimates the sensor by whole 2x2 blocks — which preserves the
CFA phase, so the reduced plane is still a valid Bayer image — until the longest
edge is at most 1600 pixels, then demosaics it bilinearly. Export and final
render use every photosite and the better algorithm.

The preview is the same picture, not merely a smaller one: white balance,
colour matrix, and tone are identical, and a test asserts the mean of the
preview matches the full render on a real photograph.

## Source-backed projects

A RAW layer stores the file it came from, not just the raster it was developed
into. `Layer.raw` carries the source reference, its SHA-256, the development
parameters, and the camera metadata. The developed raster in the project is a
cache; reopening develops the photograph again.

**Linked** is implemented: the project records where the file lives and checks
it on reopen.

- **Available** — present, hash matches.
- **Missing** — nothing readable at the recorded path. The interface offers to
  locate it; nothing is substituted.
- **Changed** — a file is there but hashes differently. PhotoForge refuses to
  use it. Binding a project to a different photograph silently is worse than
  failing, so a relink verifies the hash and reports the mismatch.

**Embedded** is in the schema so a later release can write self-contained
projects and this build can still read them. It is not produced here, and
nothing silently switches between the two modes.

## Security

A RAW file is hostile input. Every size, offset, and count inside one is
attacker-controlled.

- File size, declared dimensions, and pixel count are bounded before allocation
- TIFF: bounded IFD count, nesting depth, entries per IFD, value length, and
  value count; every offset checked against the buffer; a self-referencing IFD
  chain terminates on a walk budget
- Lossless JPEG: declared frame geometry bounded before allocation; the bit
  reader cannot read past its buffer; Huffman codes longer than 16 bits refused
- Black and white levels validated; a black level at or above white is refused
- Strip and tile tables bounded; exact segment counts, decoded geometry, truncation, and offsets/lengths checked against the file
- Unsupported predictors, floating samples, planar variants and LinearizationTable are rejected explicitly
- Structured deterministic mutations cover offsets, dimensions, bit-depth wraparound and tile tables; this is bounded property testing, not an exhaustive fuzz campaign
- No RAW file causes a network request, a process launch, or a library load

Tested against empty files, random bytes, truncated files, header-only files,
a JPEG named `.dng`, all-zero and all-ones buffers, absurd declared dimensions,
strips pointing outside the file, invalid CFA codes, and invalid levels. Every
prefix of a valid file is also decoded, to prove truncation at any point fails
rather than panics.

Error messages are written for people: a refusal names the reason, and the
tests assert no stack trace, pointer, or oversized diagnostic reaches the
interface.

## Test fixtures

Unit and integration tests use DNG files the test code writes itself, so every
sample has a known expected value and no photograph anyone else owns is
committed.

Real camera files are covered separately and optionally. `raw_real_files.rs`
skips itself unless `PHOTOFORGE_RAW_FIXTURES` names a directory holding the
samples, so **`cargo test` never touches the network**. To run them:

```powershell
./scripts/fetch-raw-fixtures.ps1
$env:PHOTOFORGE_RAW_FIXTURES = "$PWD/raw-fixtures"
cargo test --manifest-path src-tauri/Cargo.toml --test raw_real_files
```

The fixtures are Canon EOS 5D Mark III files from
[raw.pixls.us](https://raw.pixls.us), released under Creative Commons Zero.
The same photograph is published uncompressed and with lossless JPEG, which is
what makes the strongest available check possible: both must decode to
bit-identical sensor data, and they do.

## Offline

Decoding and development are entirely local. No model, decoder, or camera
profile is downloaded, at any point, including the first time a format is seen.
The optional fixture script is the only thing in the repository that touches
the network, it is never run by a test, and it downloads sample photographs
rather than anything the application uses.

## Phase 10 layers and batch

DNG placement retains float source pixels, SHA-256, metadata and development
parameters. The selected-source panel applies a full-resolution re-development
as one undoable edit; pending/stale document requests cannot publish into a new
session. RAW files are read-only. Decoder version 2 fixes white balance being
applied twice and keeps negative camera-transform results. Old project caches
remain exact until the user explicitly re-develops.

Batch processes one image at a time under the shared CPU admission gate:
default sensor development, then the selected working-space edit workflow,
then selected output space/profile and bit depth. A workflow RawDevelopment
operation is a post-demosaic working-RGB adjustment, not a sensor preset.
Progress/failures/cancellation remain per-file; output folders must be outside
the input folder, duplicate names are claimed once, and existing targets are
skipped unless overwrite is selected. PNG16 writes atomically, row by row.
Cancellation is checked around decode/development and between operation/export
rows; the existing sensor decoder itself is not interruptible mid-segment.

See [Phase 10 results](phase-10-results.md) for current real-file and packaged evidence.
