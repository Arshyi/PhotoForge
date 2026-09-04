# RAW development

PhotoForge decodes camera RAW files locally and develops them without ever
writing to the original. This page says exactly which formats that covers, how
the decoder was chosen, and what the pipeline does in what order.

## Decoder choice

**PhotoForge decodes DNG with its own decoder, written against the published
Adobe DNG specification. No third-party RAW library is linked.**

That is a licensing decision as much as an engineering one, and it is worth
stating plainly because it bounds what PhotoForge can support.

| Candidate | Licence | Camera coverage | Native build | Verdict |
| --- | --- | --- | --- | --- |
| `rawloader` 0.37 | **LGPL-2.1** | Very broad | None (pure Rust) | Rejected on licence |
| `rawler` 0.8 (dnglab) | **LGPL-2.1** | Very broad, actively maintained | None | Rejected on licence |
| `quickraw` 0.2-alpha | **LGPL-2.1** | Moderate | None | Rejected on licence and maturity |
| `dng` 1.6 | **AGPL-3.0** | DNG only | None | Rejected on licence |
| `zenraw` 0.2 | **AGPL-3.0** or commercial | Moderate | None | Rejected on licence |
| LibRaw (via `libraw-sys`) | **LGPL-2.1 or CDDL-1.0** | Broadest available, incl. CR3 | C++ build, DLL to package | Rejected on packaging and licence risk |
| `rawkit` 0.1 | **MIT or Apache-2.0** | **Sony ARW only** | None (pure Rust) | Licence fine, coverage too narrow to test |
| **Own DNG decoder** | PhotoForge's own | DNG | None | **Chosen** |

The deciding constraint: PhotoForge's README states that *all rights are
reserved*. Statically linking an LGPL-2.1 crate into a distributed
closed-source binary is a licence violation the repository owner has not
chosen to make, and AGPL is stricter still. LibRaw's CDDL-1.0 option would be
workable, but it brings a C++ toolchain, a DLL to package and verify, and a
second licence notice to ship — a large cost for a phase whose brief warns
against destabilising the application.

DNG is a published specification, so it can be read without any of that. It is
also not a niche choice: Leica, Pentax, Ricoh, Sigma, DJI, and Apple ProRAW
write DNG natively, and Adobe's free DNG Converter turns any other camera's
RAW into one.

`rawkit` is the right thing to revisit for Sony ARW when a test fixture for it
is available; its licence is compatible and its API exposes everything needed.
It was not added here because ARW support that cannot be tested is not support.

## Format capability

| Format | Status | Notes |
| --- | --- | --- |
| DNG, uncompressed | **Tested** | Verified against Canon EOS 5D Mark III output and synthetic fixtures at 8, 10, 12, 14, and 16 bits |
| DNG, lossless JPEG (compression 7) | **Tested** | Verified bit-identical to the uncompressed encoding of the same photograph |
| DNG, lossy JPEG (compression 34892) | **Refused by name** | Stores three demosaiced samples per pixel, not CFA data |
| DNG, tiled | **Implemented, not tested** | Tile placement is implemented; no tiled fixture was available |
| DNG, LinearRaw photometric | **Implemented, not tested** | Accepted; no fixture available |
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
- Strip and tile tables bounded; offsets and lengths checked against the file
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
