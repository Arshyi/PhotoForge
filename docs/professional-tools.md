# Professional tools

PhotoForge 0.6.0 adds deterministic professional editing without changing the non-destructive pipeline. Every adjustment is an `EditOperation`, validates before rendering, participates in undo/redo, can be recorded in a workflow, and is replayed identically by preview, full-resolution export, and batch processing.

## Tone and color

- Curves support independent RGB, red, green, and blue point sets. Each channel accepts 2–32 ordered normalized points. Linear, contrast, matte, and bright presets are included.
- Levels expose input black/white, gamma, and output black/white. Invalid or reversed ranges are rejected.
- HSL includes master, red, yellow, green, cyan, blue, and magenta adjustments.
- Temperature/tint is a manual operation separate from automatic white balance.
- Selective color applies bounded cyan, magenta, yellow, and black changes across an explicit hue range.
- White- and black-point pickers sample a visible pixel and create a typed point-balance operation.

## Geometry

- Crop uses normalized bounds, free or Square/16:9/4:3/A4/original ratios, and rule-of-thirds or golden-ratio composition overlays.
- Straighten supports ±45 degrees, an alignment grid, and a separate 90-degree snap.
- Perspective correction maps an explicit four-corner quadrilateral with deterministic bilinear resampling.
- Lens correction combines barrel/pincushion distortion, vignetting correction, and basic chromatic-aberration channel alignment.

Transforms never infer missing content. Pixels outside a transformed source are transparent and exports follow the existing alpha rules.

## Inspection and comparison

The live histogram command computes 256-bin red, green, blue, and Rec. 709 luminance scopes for before and after images, including shadow/highlight clipping counts. Histogram work is serialized behind a request gate and runs on the bounded preview copy.

The inspector reports coordinates, RGBA, HSV, image dimensions, sRGB color space, per-channel bit depth, alpha, file timestamps, and JPEG camera model when a safe EXIF Model tag is available. Zoom extends to 1600%, with pixel grid, crosshair, and coordinate measurement tools.

Comparison supports swipe, independent side-by-side geometry, blink, difference blending, and before/after histogram comparison.

## Export profiles

Web, Print, Archive, Lossless, High JPEG, and Maximum Compression profiles choose appropriate PNG, JPEG, or WebP output and quality policy. The last selected profile is remembered locally. The source path remains protected from overwrite.
