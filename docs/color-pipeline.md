# Colour pipeline and precision boundaries

PhotoForge mixes a high-precision development path with an 8-bit compositor
inherited from Phase 8. That is a deliberate, bounded arrangement rather than an
oversight, and this page says exactly where precision is kept and where it is
given up, so nobody has to guess.

## Where precision lives

| Stage | Representation | Precision |
| --- | --- | --- |
| RAW sensor samples | `u16` | 8-16 bits, as the camera recorded |
| Black-level normalisation | `f32` | Full float; **not** clipped at 1.0 |
| White balance (on the CFA) | `f32` | Full float |
| Demosaic | `f32` RGB | Full float |
| Camera RGB to linear sRGB | `f32` RGB | Full float; negatives clamped, highlights kept |
| Exposure and tone controls | `FloatImage` (`f32` RGBA) | Full float, scene-linear, unclipped |
| **Layer pixel store** | `RgbaImage` (`u8`) | **8 bits — quantisation boundary** |
| **Compositor and blend modes** | `u8` in, `f32` internally, `u8` out | **8 bits at every layer boundary** |
| Preview to screen | `u8` PNG data URL | 8 bits |
| 8-bit export (PNG/JPEG/WebP) | `u8` | 8 bits |
| **16-bit PNG export** | `u16` straight from `FloatImage` | **16 bits — no 8-bit stage** |

## The two paths, stated plainly

**RAW to 16-bit PNG never passes through 8 bits.** `export_raw_layer_png16`
decodes the original again, develops it at full sensor resolution with the
high-quality demosaic, and quantises once, at the file. This is verified rather
than asserted: every 8-bit value maps to a multiple of 257 in 16 bits, so the
test counts samples that are *not* multiples of 257 and requires most of them
to be, which an 8-bit image expanded into a 16-bit container could never
satisfy.

**RAW inside a layered document is quantised to 8 bits when it becomes a
layer.** The pixel store holds `RgbaImage`, so a developed RAW is rounded to 8
bits per channel before it can be composited with other layers, masked, or
blended. Re-developing with new parameters goes back to the original file, so
the *decision* stays non-destructive — but the composite the screen shows, and
any 8-bit export of it, carries 8-bit layer data.

That is the honest summary: **the development is high precision; the compositor
is not.**

## Highlight headroom

Nothing clips before the display or export transform. A photosite brighter than
the nominal white level normalises to a value above 1.0 and stays there through
white balance, demosaic, the colour matrix, and the tone controls.

Verified on a real photograph: the Canon EOS 5D Mark III sample records samples
up to 15464 against a white level of 15000, 305 pixels are still above 1.0
after a full development, and reducing exposure by one stop brings them back.
A synthetic test pins the same behaviour against what an early 8-bit conversion
would have kept, and shows the linear path recovers strictly more.

## sRGB transfer function

The encode and decode functions implement IEC 61966-2-1 exactly, including the
linear segment below the 0.0031308 / 0.04045 threshold rather than a pure 2.2
power approximation. Reference values at 0, the threshold region, 0.18, 0.5,
and 1.0 are pinned by test, along with round-trip tolerance.

## Working colour space

The working space is **linear sRGB primaries under D65**. The camera-to-working
transform is derived from the DNG `ColorMatrix` tag: the XYZ-to-camera matrix is
composed with sRGB-to-XYZ, its rows are normalised so a camera-neutral signal
maps to sRGB white — the convention the DNG specification describes, and the
reason a grey card comes out grey — and the result is inverted. A singular
matrix is refused rather than producing infinities.

A file with no colour matrix is developed in the camera's own RGB and
`colorManaged` comes back false. The interface says so; it does not imply the
result is colour managed when it is not.

## Display P3 and Adobe RGB

**Not implemented.** The enumeration in `color::WorkingColorSpace` exists, but
no conversion to or from Display P3 or Adobe RGB has been written, so neither
is offered in the interface. A dropdown label is not colour management, and
exposing one would be a false claim.

Adding them is bounded work — both are matrix-plus-transfer-function
conversions with published constants — and is deferred rather than abandoned.

## ICC profiles

**Not implemented, and deliberately not half-implemented.** Arbitrary ICC
support means parsing untrusted profile data, which is a decoder in its own
right with its own attack surface. The options assessed:

| Option | Assessment |
| --- | --- |
| Little-CMS via bindings | Mature and correct, but a C library to build, package, verify, and license-notice on Windows — the same cost that ruled out LibRaw for RAW |
| `qcms` (Firefox's) | MPL-2.0, pure Rust, much smaller surface. The realistic candidate |
| Hand-written ICC parser | Rejected. A half-correct colour-management implementation is worse than none |

Embedded ICC profiles in opened files are neither read nor honoured today.
Deferred to Phase 10 with `qcms` as the recommended starting point.

## Float compositing

Evaluated, not implemented. Moving the compositor to `f32` RGBA would end the
8-bit layer boundary above, but it quadruples every layer buffer, changes the
memory ceiling that bounds a document, and would need every blend-mode test
re-derived at a new precision. Phase 8's compositor is heavily tested and
stable; destabilising it to remove a quantisation step that only affects
layered RAW work did not seem a good trade inside this phase.

The boundary is documented above rather than quietly moved, so the cost is
visible and the change can be made deliberately later.

## Known quantisation limitations

1. A RAW layer inside a layered document is 8-bit once composited.
2. Display P3 and Adobe RGB have no implementation.
3. Embedded ICC profiles are ignored.
4. Blend modes operate on encoded sRGB values, not linear light — unchanged
   from Phase 8 and documented in `docs/compositing.md`.
