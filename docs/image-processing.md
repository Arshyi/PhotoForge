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

## Phase 2 restoration operations

All restoration operations are deterministic tagged pipeline entries. Strength zero is exact identity, floating-point parameters must be finite, kernels and regions are bounded, and source alpha is preserved—including document grayscale mode. They operate on 8-bit encoded channel values and are not substitutes for a color-managed workflow.

### Automatic white balance

**Use:** photographs or color scans with a broad warm/cool/green cast.

**Algorithm:** transparent pixels are ignored; per-channel 5%-trimmed means feed a gray-world estimate. Correction gains are luminance-normalized, clamped to `0.67…1.5`, and blended by strength `0…1`.

**Limitations:** a scene dominated by one legitimate color may be over-corrected. It is not a calibrated camera/profile correction.

**Performance:** linear in pixel count with three 256-bin histograms. It does not amplify spatial noise. Alpha is preserved and preview/export use the same algorithm.

### Local contrast

**Use:** flat photographs, faded scans, and locally weak page contrast.

**Algorithm:** a linear-time sliding box filter estimates local luminance. A clipped difference from that estimate is blended into luminance rather than processing RGB channels independently. Parameters are strength `0…1`, tile size `8…128` pixels, and clip limit `0.5…4`. This is a conservative local-luminance normalization, not a complete interpolated CLAHE implementation.

**Limitations:** strong settings can emphasize noise or produce an overly crisp appearance. Color is preserved approximately through equal channel deltas.

**Performance:** linear time and two one-byte luminance buffers. Alpha is preserved. Preview/export semantics are identical, though pixel-sized regions naturally cover different physical areas at different resolutions.

### Edge-preserving denoise

**Use:** moderate sensor/scanner grain and chroma variation.

**Algorithm:** a bounded 3×3 or 5×5 spatial filter weights neighbors by both distance and RGB difference. Edge preservation controls the range sensitivity, and strength blends the filtered value with the source.

**Limitations:** this conservative filter does not remove severe noise and can soften fine texture at high strength.

**Performance:** at most 25 bounded samples per pixel; it is the most expensive single Phase 2 preview tool. No heavyweight dependency is used. Alpha is preserved.

### JPEG cleanup

**Use:** decoded images showing mild 8×8 block discontinuities. The source does not need to still be a JPEG file.

**Algorithm:** pixels immediately across 8×8 boundaries are blended conservatively only when their luminance discontinuity is moderate (`4…80` channel levels). Major edges, transparent boundaries, and already-smooth boundaries are left alone.

**Limitations:** it cannot restore discarded JPEG coefficients and deliberately avoids aggressive ringing removal. Strong use may soften legitimate detail aligned to block boundaries.

**Performance:** touches block-boundary rows/columns rather than every neighborhood. Alpha is preserved.

### Edge-aware sharpening

**Use:** photographs and documents needing restrained edge clarity.

**Algorithm:** a bounded local luminance estimate produces a high-pass detail signal. Detail below threshold is ignored; remaining correction is scaled by strength and capped to suppress halos. Strength is `0…2`, radius `0.5…4`, threshold `0…0.25`.

**Limitations:** low thresholds can amplify noise. It improves contrast at captured edges but does not restore missing detail.

**Performance:** linear-time luminance background plus one output buffer. Alpha is preserved.

### Mild deblur

**Use:** slight softness, mild camera shake, mild optical blur, and slightly blurred scans.

**Algorithm:** two conservative thresholded high-pass passes with different strengths and radii. This is a stable clarity-restoration approximation, not Richardson–Lucy/Wiener deconvolution. Strength is `0…1`; radius is `0.5…3`.

**Limitations:** strong settings may amplify noise or create halos and are warned about in the UI. It cannot reconstruct missing information or correct substantial motion blur.

**Performance:** two linear-time sharpen passes and bounded intermediate buffers. Alpha is preserved.

### Uneven-lighting correction

**Use:** photographed pages, whiteboards, or scans with broad illumination gradients.

**Algorithm:** a linear-time box estimate models low-frequency illumination. Each pixel receives a bounded additive luminance correction toward the transparent-aware global mean, blended by strength. Radius is `4…96` pixels. Additive correction avoids division by small dark values.

**Limitations:** shadows smaller than the selected radius may be treated as content; large settings can flatten intentional lighting. Color is preserved approximately.

**Performance:** linear time with two luminance buffers and one output. Alpha is preserved.

### Document enhancement

**Use:** photographed notes, receipts, worksheets, pages, whiteboards, and scans with moderate angle/lighting problems.

**Algorithm:** one explicit typed operation executes a documented fixed sequence: partial gray-world balance, uneven-lighting normalization, clipped local contrast, mild edge-preserving denoise, and thresholded text sharpening. Grayscale mode then applies Rec. 709 luminance; color mode retains colored annotations. Strength `0…1` scales every stage.

**Limitations:** this is enhancement, not OCR, perspective correction, background removal, or semantic document understanding. Faint gray marks are deliberately retained rather than hard-thresholded.

**Performance:** several bounded linear passes plus one small-neighborhood denoise. It is the most expensive combined operation. Alpha remains unchanged, even in grayscale mode.

## Image-quality analysis

Analysis calculates normalized average luminance, 5th–95th percentile spread, channel imbalance, Laplacian-style sharpness, high-frequency noise, local contrast, edge density, white-background ratio, and a likely-document heuristic. It runs once per document on the cached preview and is generation-protected/cached. Metrics are deterministic heuristics and never auto-apply edits or assert a definitive diagnosis.

## Preview versus export

Interactive previews are generated from a decoded source copy whose longest dimension is at most 1600 pixels. Full-resolution export reruns the same validated operation sequence against the cached original. There are no preview-only algorithm substitutions. Pixel-domain radii cover a physically smaller portion of a full-resolution image, but their mathematical meaning and operation ordering do not change. The source file is not decoded for each adjustment.

## Phase 3 guided planning

Guided Edit is not an image-processing algorithm. `RuleBasedPlanner` normalizes a bounded request, matches documented phrase groups, consults the already cached quality analysis, and proposes an `EditPlan`. The proposal contains only operations implemented by PhotoForge; no operation can contain generated pixels, executable code, a path, or a model instruction.

The planner can propose bounded brightness/contrast adjustments and existing Phase 2 restoration operations for color casts, weak local contrast, noise, JPEG blocks, slight softness, uneven illumination, and documents. It sorts proposals into a safe cleanup-to-detail order, deduplicates operation types, caps plans at eight operations, and adds limitations where restoration may amplify noise or cannot recover discarded information. Unknown intent is rejected instead of being guessed.

The user may remove, reorder, or adjust proposed operations. Every edited plan then crosses the Rust `validate_guided_plan` boundary again. Only validated `Vec<EditOperation>` enters the same preview and full-resolution export pipeline documented above. Guided planning therefore adds no new pixel semantics, approximation, encoder behavior, or alpha policy.

## Export policy

- PNG: lossless RGBA; alpha is preserved.
- WebP: lossless RGBA; alpha is preserved.
- JPEG: quality 90; transparency is explicitly composited onto white before encoding.
- Output dimensions and operation order match the full-resolution pipeline. The original input bytes are never rewritten.

PhotoForge detects the output format from the selected `.png`, `.jpg`/`.jpeg`, or `.webp` extension. It does not silently choose an encoder default.

## Color limitations

Phase 2 processes 8-bit channel values in the decoded image crate representation. Most arithmetic is performed directly on those encoded channel values rather than in linear light, so it is deterministic but not a color-managed photographic workflow. Embedded ICC profiles, EXIF metadata, HDR depth, and animation are not preserved. These constraints are candidates for later phases.
