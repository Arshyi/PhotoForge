# Color pipeline and precision boundaries — Phase 10

New documents use linear-sRGB/D65 f32 RGBA through sources, layers, masks,
groups, adjustments, transforms, merge, project persistence and final export.
Projects without explicit precision metadata keep the original encoded-sRGB
RGBA8 renderer. Conversion of an old document is explicit and may change its
appearance; expanding an old byte source does not restore lost precision.

| Stage | Representation / boundary |
| --- | --- |
| DNG sensor | Bounded u16 photosites; actual bit depth and black/white levels |
| RAW development | CFA white balance once, demosaic, camera matrix, float tone |
| Ordinary PNG/JPEG/WebP | Embedded RGB ICC honored; untagged input assumes sRGB |
| Native PNG16 | Decoded directly to float, without an RGBA8 intermediate |
| Authoritative store | Typed immutable encoded RGBA8 or straight linear RGBA f32 |
| Linear document | Transfer-decodes ordinary byte sources; no byte bridge in final rendering |
| Masks | Existing u8 scalar coverage, converted to float opacity; not color data |
| Display/thumbnail | Bounded sRGB-encoded RGBA8 PNG; intentionally clipped/quantized |
| PNG16 export | Float layered composite, output-space conversion, one u16 quantization |
| PNG8/JPEG/WebP | Intentional 8-bit quantization; optional deterministic ordered dither |

## Alpha and blend semantics

Sources and stored results use straight alpha in [0,1]. Compositing,
interpolation, isolated groups and pass-through crossfades use premultiplied
linear values internally. See [high-precision-rendering.md](high-precision-rendering.md)
for equations. Zero-alpha samples become transparent black when unassociated;
blur/resampling weight color by alpha rather than leaking invisible RGB.

All 16 existing modes are supported. Normal, multiply, screen, darken, lighten,
difference and exclusion extend their arithmetic to finite out-of-range linear
values. Overlay, dodge, burn, hard/soft light use bounded SDR blend inputs;
their nonlinear semantics are not an HDR artistic standard. Hue, saturation,
color and luminosity explicitly encode float sRGB, apply the existing W3C
nonseparable equations, and decode again. They do not apply HSL to linear RGB.
Alpha never participates in color-space transfer functions. Invalid finite/
alpha inputs are rejected rather than silently switching to the byte renderer.

Exposure, white balance, convolution and alpha filtering operate in linear
light. Brightness, contrast, gamma, curves, levels, HSL, selective color and
sepia deliberately use encoded **float** sRGB for familiar control behavior.
Local contrast/lighting use alpha-weighted linear luminance. SDR tone controls
and SDR blend definitions may intentionally bound color; this is not a blanket
promise of unlimited HDR preservation through every artistic operation.

## Source, working, display and output are separate

Working primaries are fixed to sRGB under D65. Float RGB may be negative or
above one, allowing colors outside the sRGB gamut without early gamut clipping.
Display P3 uses P3 primaries, D65 and the sRGB transfer curve. Adobe RGB (1998)
uses its published primaries, D65 and exponent 563/256. Conversion uses XYZ
matrices, not relabeling. Bradford adaptation is tested for D50/D65.
Reference constants: [W3C color conversion code](https://www.w3.org/TR/css-color-4/#color-conversion-code)
and [Adobe RGB (1998) specification](https://www.adobe.com/digitalimag/pdfs/AdobeRGB1998.pdf).

Embedded RGB ICC input uses **moxcms 0.8.1**, pinned in Cargo, with float
transforms and extended-range support. This pure-Rust backend was already an
image dependency; Phase 10 enables it directly without another DLL or runtime
download. Its upstream license choice is BSD-3-Clause or Apache-2.0; the BSD
notice ships with the artifacts. [Upstream](https://github.com/awxkee/moxcms).
qcms/MPL-2.0 was considered; using the already-locked float-capable backend
keeps the Windows build smaller in scope. This is not an exhaustive CMS survey.

Profiles are bounded to 4 MiB, 256 tags, 1 MiB CLUTs and 16,384 TRC entries;
non-RGB, malformed and oversized profiles fail import. Backend TRC lookup
interpolation has finite precision (roughly 14-bit tables), not infinite
analytic ICC accuracy. Controlled sRGB/P3/Adobe fixtures and malformed-profile
tests exercise the selected subset; arbitrary third-party profile coverage is
not exhaustively certified.

Output selection supports sRGB, Display P3 and Adobe RGB. All three transform
pixels **and embed the corresponding RGB ICC profile** in PNG/JPEG/WebP. PNG
supports 8/16 bits; JPEG/WebP are 8-bit. JPEG flattens transparency against white
in linear light. Export strips source EXIF/GPS and embeds only the chosen ICC.
Ordered 4x4 Bayer dither affects RGB in 8-bit output only, not alpha, endpoints
or 16-bit output. Generated profile header timestamps are fixed to the profile
definition date (2026-09-04), not the export time. Same build/settings/input
produce deterministic bytes; input profiles are never rewritten in place.

## Explicit limits

Windows/WebView2 own the final monitor presentation. PhotoForge sends sRGB
previews but does not select/calibrate a monitor ICC profile or certify the
complete display chain. No printer soft proofing, CMYK editing, arbitrary
user-selected output ICC, HDR display pipeline or general TIFF import is
implemented. TIFF is parsed only inside the supported DNG subset.

A DNG without a supported camera matrix is uncalibrated camera RGB assigned to
the working interpretation, not a verified camera-to-sRGB conversion. The RAW
command reports colorManaged=false. Monochrome LinearRaw uses neutral white
balance. Broad real-camera color accuracy beyond the tested Canon fixture is
not certified; a supported file format does not guarantee a calibrated source.

The standalone compatibility command export_raw_layer_png16 still exists,
but document export uses the actual layered compositor. Reopening a project
restores its exact typed developed buffers; RAW source hashes/parameters are
retained for explicit re-development, not silently replayed with a newer decoder.
Decoder version 2 corrects a double white-balance application and preserves
negative camera-transform components; only explicit re-development changes
old cached source appearance.

See [phase-10-results.md](phase-10-results.md) for tested versus unverified claims.

## Phase 11 render path

Linear documents are composed by the Phase 11 tiled CPU plan at the same
linear-f32 precision described above. Tile boundaries do not introduce a color
conversion: each tile samples straight-alpha source values, composes in
premultiplied linear values, and is converted to the requested display/export
space only at the output boundary. Neighbourhood operations receive a derived
halo; global operations use the full-frame reference so their statistics are
not normalised independently per tile.

An optional Vulkan compute path accelerates only eligible full-frame Gaussian
blur after the pixels are already in linear-f32 form. It does not replace the
working-space transform, masks, blend semantics, ICC conversion, or 16-bit
output conversion. Any unsupported or failed GPU operation falls back to the
same CPU implementation. The detailed backend scope and tolerances are in
[gpu-rendering.md](gpu-rendering.md).
