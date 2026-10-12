# Phase 15.1d — Photoshop adjustment layers: Curves, Levels, Invert, Exposure

[15.1](phase-15-1-results.md) left adjustment layers out: they hold no pixels, so the layers
beneath were shown without them, which changes the picture (about 160 layers in the 316 test
files). This increment imports the four that PhotoForge can hold, as adjustment layers of its own,
and checks them against Photoshop's stored pictures.

Status: **implemented and verified as listed; not committed, not released.** The version is still
0.14.0. It sits on top of [15.1b](phase-15-1b-results.md) (live clipping, which clipped adjustment
layers need) and [15.1c](phase-15-1c-results.md).

## Completed

* **`src-tauri/src/psd/adjustment.rs`**: decodes the settings blocks `curv`, `levl`, `nvrt` and
  `expA` with every length checked, and returns the PhotoForge operation or a reason it cannot.
  * **Invert** → a curve, exactly.
  * **Levels** → PhotoForge's Levels when only the master settings are non-default (the same
    arithmetic), otherwise a curve per channel.
  * **Curves** → PhotoForge's curves: the composite curve as it is when no channel curve is set,
    otherwise composed per channel; a smooth curve sampled at up to 32 points.
  * **Exposure** → a curve of what Photoshop computes in linear light.
* **The importer** builds the adjustment layer with the file's opacity, visibility, blend mode and
  canvas-space mask, and clipping (live), and reports the sampling. A block it cannot read is left
  out with the reason.
* **The reader** keeps the settings block of these four keys (at most 64 KiB each).

## What Photoshop's pictures settled

The documentation does not say two things the conversion depends on. The psd-tools suite has a
folder of adjustment files, each with Photoshop's own flattened picture stored; the mean error
against it, on `curves_rgb.psd`, for each choice:

| Interpolation of the curve through its points | channel then composite | composite then channel |
| --- | --- | --- |
| **natural cubic spline** | **0.59** | 13.5 |
| monotone cubic (Fritsch–Carlson) | 9.3 | 19.9 |
| cardinal spline | — | 20.7 |
| straight lines | 13.7 | 22.4 |

So Photoshop's curve is a natural cubic spline, and **a channel's curve is applied before the
composite one**; likewise for Levels (0.32 against 15.4). The point order inside a `curv` block is
output first, then input (the other way gives 54 and refuses a curve as out of order). And the
`levl` block has no count: twenty-nine ten-byte records follow the version, which the first reading
of the specification got wrong and the real file corrected.

## Against real files

| File | Mean error (levels) | Within 2 levels |
| --- | --- | --- |
| `curves_rgb.psd` (4 curve layers, per-strip masks) | 0.53 | 94% |
| `curves_grayscale.psd` | 0.06 | 100% |
| `levels_rgb.psd` | 0.32 | 98% |
| `levels_grayscale.psd` | 0.27 | 98% |
| `invert_rgb.psd`, `invert_grayscale.psd` | 0.00 | 100% |
| `exposure_rgb.psd` | 1.49 | 75% (99% within 8) |
| `adjustment_clipping.psd` | 0.05 | 100% |
| `adjustment_nested_composition_1.psd` | 1.17 (was 8.82) | 59% (91% within 8) |
| `adjustment_nested_composition_4.psd` | 1.93 (was 56.68) | 99% |
| `adjustment_nested_composition_5.psd` | 0.07 (was 2.17) | 100% |

Whole set (316 files, not committed): 290 imported, 26 refused, 0 failed; **38 imports that
reported nothing approximated or left out, 37 of them close to Photoshop's picture** (the other is
the known blank flattened image); **212 with approximations, 127 still close** (15.1: 220 and 115;
15.1c: 218 and 120). In those files 29 Curves, 14 Exposure and 8 Levels layers are now adjustment
layers where they were left out.

## What is still left out, and why

* **Greyscale Exposure.** Photoshop's picture matches neither sRGB, a 2.2 power, nor the encoded
  values (11, 11 and 30 levels); the transfer curve it uses there is not known. Four layers.
* **Brightness/Contrast, Hue/Saturation, Posterize, Threshold, Color Balance, Gradient Map,
  Selective Color, Channel Mixer, Photo Filter, Black & White, Vibrance, Color Lookup.**
  - Hue/Saturation, Threshold and the colour mixers are not per-channel, so a curve cannot hold
    them; PhotoForge has no operation of that meaning to map them to.
  - Posterize is per-channel but needs two points per level and a curve holds 32 (the fixtures use
    13 and 21 levels).
  - Brightness/Contrast: the legacy form is a known formula, but the one file with it
    (`brightnesscontrast_legacy_rgb.psd`) is masked and its mapping could not be read off the
    picture in the time given; the current form's settings are in a different block
    (`CgEd` descriptor) which this reader does not parse.
* **A curve is 32 points.** A Photoshop curve with sharp features that falls between sampled points
  differs from the original there (the maximum error in `curves_rgb.psd` is 51 levels in a few
  pixels, against a mean of 0.5).

## Verification

Same machine and settings as 15.1 (dev profile, dependency debug info reduced per invocation).
The matrix below was run once, on the tree that holds 15.1b, 15.1c and 15.1d together; each of those
results documents quotes it.

| Gate | Result |
| --- | --- |
| `cargo fmt --check` | clean |
| `cargo clippy --all-targets --all-features -- -D warnings`, and the same with no GPU / no inference / no plugins | clean (all four) |
| `cargo test --all-targets --all-features --no-fail-fast` | **1,561 passed**, 0 failed, 43 test binaries (15.1 ended at 1,495) |
| No GPU / no inference / no plugins | **1,554 / 1,553 / 1,522 passed**, 0 failed |
| `npx tsc --noEmit`, `npm run check` | clean; 510 files, 0 errors, 0 warnings |
| `npx vitest run --maxWorkers=4` | 85 files, **1,285 passed**, 0 failed (15.1 ended at 1,264); one *unhandled error*, from a `CommandPalette` test that makes a command throw on purpose, a file and component this work does not touch |
| `npm run build` | built |

New tests: 11 in `adjustment.rs` (a real `curv` block from a Photoshop file, two-point curves exact,
channel before composite for curves and for levels, master-only levels becoming PhotoForge's
Levels, greyscale, exposure in linear light, a sweep that cuts every block at every length, and
refusals), 5 through the importer (an invert layer that renders inverted, opacity/visibility/blend/
clipping kept, a sampled curve reported, an unreadable block left out with its reason, an adjustment
not imported still named).

Each check was shown able to fail by breaking what it protects: applying the composite curve before
the channel's (2 fail); not carrying the clipped flag to an adjustment layer (4 fail); applying the
master levels before the channel's. That last one **survived** the first version of its test, whose
numbers happen to commute; the test was changed to numbers that do not and then caught it.