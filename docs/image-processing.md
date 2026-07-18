# Image processing

PhotoForge applies operations in the exact order shown by the pipeline. The source is converted to 8-bit RGBA once, then every operation returns a new RGBA buffer and preserves alpha unless geometry changes pixel placement.

## Implemented operations

### Brightness

Adds `amount × 255` to each RGB channel and clamps the result to `[0, 255]`. Accepted amount: `-1.0…1.0`; the UI intentionally exposes a narrower range.

### Contrast

Scales RGB distance from the midpoint 127.5 by `1 + amount`. Negative values compress tonal range and positive values expand it.

### Saturation

Computes Rec. 709 luminance (`0.2126 R + 0.7152 G + 0.0722 B`) and interpolates each channel away from or toward that luminance using `1 + amount`.

### Gamma

Normalizes each channel and evaluates `255 × input^(1 / gamma)`. Gamma above 1 brightens midtones; gamma below 1 darkens them. Accepted range: `0.2…3.0`.

### Grayscale and sepia

Grayscale uses Rec. 709 luminance. Sepia uses the conventional 3×3 sepia matrix. Both clamp results and preserve alpha.

### Reflection and rotation

Reflection reverses each row. Rotation is limited to multiples of 90 degrees, avoiding interpolation and preserving exact pixels.

### Gaussian blur

Uses the `image` crate's Gaussian blur implementation with a validated radius of `0…20`.

### Sharpen

Uses unsharp masking: a Gaussian-softened copy is subtracted from the original, the difference is multiplied by strength, and the result is added back. Strength is limited to `0…2`.

Sharpening increases local edge contrast. It does not reconstruct information absent from the source and is not presented as deblurring or genuine detail recovery.

## Preview versus export

Interactive previews are generated from a decoded source copy whose longest dimension is at most 1600 pixels. Full-resolution export reruns the same validated operation sequence against the cached original. The source file is not decoded for each adjustment.

## Export policy

- PNG: lossless RGBA; alpha is preserved.
- WebP: lossless RGBA; alpha is preserved.
- JPEG: quality 90; transparency is explicitly composited onto white before encoding.
- Output dimensions and operation order match the full-resolution pipeline. The original input bytes are never rewritten.

PhotoForge detects the output format from the selected `.png`, `.jpg`/`.jpeg`, or `.webp` extension. It does not silently choose an encoder default.

## Color limitations

Phase 1 processes 8-bit channel values in the decoded image crate representation. Most arithmetic is performed directly on those encoded channel values rather than in linear light, so it is deterministic but not a color-managed photographic workflow. Embedded ICC profiles, EXIF metadata, HDR depth, and animation are not preserved. These constraints are candidates for later phases.
