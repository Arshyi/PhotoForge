# Phase 9 colour pipeline

Phase 9 introduces an opt-in, high-precision development boundary without
changing the established 0.8.x layer storage or legacy operation semantics.
`src-tauri/src/color.rs` owns the boundary and is intentionally independent of
Tauri, filesystem access, and model providers. The `raw_development` operation
and Professional workspace panel now use this boundary for deterministic raster
previews and exports; camera RAW import remains decoder-gated.

## Representation and boundaries

```text
encoded source (sRGB bytes)
  -> sRGB transfer decode
  -> straight-alpha linear-sRGB FloatImage (f32)
  -> development controls and future float compositing
  -> sRGB transfer encode + nominal-range clamp
  -> display preview / 8-bit export / true 16-bit PNG export
```

`FloatImage` is row-major and bounded at 40 million pixels, matching the
existing decoder ceiling. RGB values are allowed to be negative or above one
between the decode and encode boundaries; alpha remains a straight coverage
value and is not gamma transformed. The legacy `RgbaImage` compositor and
existing encoded-space operations remain unchanged. `raw_development` is an
explicit opt-in bridge: it converts the current image to `FloatImage`, applies
the development pass, and encodes only at the operation boundary, so old
projects do not silently change appearance.

The transfer functions are the IEC 61966-2-1 sRGB piecewise equations. Their
sign-preserving extension is used only for mathematical intermediates. Output
quantisation clamps to `[0, 1]` and rounds directly to either 8 or 16 bits.
`FloatImage::encode_png16` therefore produces a genuine 16-bit RGBA PNG rather
than expanding an 8-bit result.

## Development controls

`DevelopmentParameters` is separate from the legacy 8-bit adjustment-layer
schema. It currently contains:

- `WhiteBalance::AsShot` and `Custom` channel multipliers;
- deterministic `Auto` gray-world balancing (transparent pixels ignored);
- a normalised temperature/tint control, deliberately not labelled in Kelvin
  because no calibrated camera profile is present;
- stop-based exposure (`+1 EV` multiplies linear intensity by two);
- contrast around a linear middle-grey pivot; and
- smooth luminance-weighted highlights, shadows, whites, and blacks.

The pass is deterministic, validates finite bounded parameters, preserves
out-of-range float values, and performs no image generation. RGB/luminance
histograms are bounded by a caller-supplied sample ceiling and report values
below zero and above one as clipping indicators.

## Current scope and next boundary

This module is the tested numerical foundation for RAW development and future
float layer compositing. It is not yet the application's default renderer:
camera RAW decoding/demosaic, ICC transforms, and a float layer pixel store are
still deferred. The current 0.8.2 project format continues to store RGBA8 layer
PNGs, while development parameters are serialized as a validated operation and
the export command can write a genuine 16-bit PNG.
