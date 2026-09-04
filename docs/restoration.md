# Restoration

Every restoration tool in PhotoForge is deterministic and runs locally with no
model installed. This page says what each one does, how it behaves inside the
tiled renderer, and where it stops being useful.

## How these tools are judged

Restoration is the easiest area in which to claim an improvement and the
hardest in which to justify one. Every number here comes from
`image_processing::metrics` measured against a known-clean fixture, and two
metrics are always reported together:

- **PSNR / SSIM** — how close the result is to the original.
- **`edge_retention`** — how much of the clean image's gradient energy
  survives. A denoiser can always win on error by erasing the picture, so this
  is what stops that.
- **`flat_area_noise`** — residual noise only where the clean image is flat, so
  a filter cannot win by leaving the noise alone.
- **`blocking_energy`** — the ratio of the mean step across 8-aligned
  boundaries to the mean step elsewhere. One means the compression grid is
  indistinguishable from the picture.

The fixtures are synthetic and honest about it. A Gaussian field is not a
sensor and a block-quantised image is not a JPEG encoder. They prove an
algorithm does what it claims on the degradation it was handed.

## The tools

| Tool | Algorithm | Space | Locality | Halo | Masks | Batch |
| --- | --- | --- | --- | --- | --- | --- |
| Denoise | Bilateral on luma, luma-guided bilateral on chroma, in YCoCg | Linear | Halo-dependent | Chroma radius, up to 10 px | Yes | Yes |
| Dust & hot pixels | Ring-median outlier repair, per channel | Linear | Halo-dependent | 1 px | Yes | Yes |
| Deconvolution | Richardson-Lucy against a named kernel | Linear | Halo-dependent, usually global | 2 x reach x iterations | Yes | Yes |
| Mild deblur | Unsharp mask | Linear | Halo-dependent | Kernel reach | Yes | Yes |
| Edge-aware sharpen | Thresholded unsharp mask | Linear | Halo-dependent | Kernel reach | Yes | Yes |
| JPEG cleanup | Gradient-gated grid deblocking | Linear | **Global** | — | Yes | Yes |
| Local contrast | Windowed luminance contrast | Linear | Halo-dependent | Half the window | Yes | Yes |
| Uneven lighting | Large-radius background estimate | Linear | Halo-dependent | The radius | Yes | Yes |
| Document enhance | Luminance normalisation | Linear | Halo-dependent | — | Yes | Yes |
| Auto white balance | Whole-image statistic | Linear | **Global** | — | Yes | Yes |

Halos are derived from the implementations, not from the control labels. A tool
whose reach exceeds the tile-halo cap is declared global and rendered whole
rather than clamped, because a clamped halo seams in proportion to how much of
the effect the user asked for.

## Denoise

Splits luminance from chroma in YCoCg — exact in floating point — and treats
them differently, because the eye resolves luminance detail and almost no
colour detail.

- **Strength** is how much noise to assume, and sets both the spatial window
  (1 to 5 px) and the range kernel.
- **Detail** narrows the range kernel, so a tap has to be closer in value
  before it may influence the result. This is what protects an edge.
- **Colour noise** filters chroma over roughly twice the luma radius, guided by
  luminance: colour smooths freely inside an object and stops at its border.

The operation is a pure function of its parameters. An earlier version measured
the noise from the image and sized the kernel to it, which scored better and
broke tiling: the tiled renderer hands the operation one tile at a time, so
every tile measured its own noise and chose its own sigma, seaming by 0.12.
Noise estimation belongs to analysis, which sees the whole image.

Measured on 256 px fixtures, against the previous implementation:

| Case | Before | After | Edges before | Edges after |
| --- | --- | --- | --- | --- |
| Gaussian, light | 31.93 dB | 33.76 dB | 0.85 | 1.00 |
| Gaussian, heavy | 26.39 dB | 25.90 dB | 0.69 | 1.00 |
| Colour noise | 27.38 dB | 35.29 dB | 0.64 | 0.98 |

Heavy noise loses half a decibel and gains 0.31 of edge retention. PSNR rewards
blurring, which is why it is never the only number.

**Cost.** A full-resolution 24 MP denoise takes about 2.1 s on eight threads.
That is substantially slower than the filter it replaces. Interactive work goes
through the preview scale and the tiled renderer, not the full frame.

## Dust and hot pixels

A bilateral filter cannot repair an impulse: its range kernel is centred on the
damaged pixel, so a hot pixel rejects every correct neighbour and survives.
Measured, denoise on a sensor-defect fixture *lost* 4.8 dB.

This tool estimates from the eight surrounding pixels with the centre excluded,
and repairs a pixel only when both conditions hold: it deviates from the ring
median by more than **Sensitivity** times the ring's own spread, and the ring
agrees with itself. A star sits on its own gradient and a fine line has
neighbours that share its value, so both fail the second test.

| Case | Denoise | This tool |
| --- | --- | --- |
| Sensor defects | 30.17 dB | 45.28 dB |
| Impulse noise | 25.17 dB | 30.92 dB |
| Dust | 28.45 dB | 31.34 dB |

Running it on an undamaged image costs under 0.02 dB.

**Limitation.** It repairs isolated pixels. A speck several pixels across is not
isolated, which is why the dust fixture gains less than the others.

## Deconvolution

Richardson-Lucy against a named point-spread function: maintain an estimate,
blur it with the kernel, compare against the observation, back-project the
ratio.

**It needs the kernel.** PhotoForge does not estimate an unknown one and the
interface asks which blur is present rather than guessing. The result is only as
good as that answer.

| Case | Unsharp mask | Deconvolution |
| --- | --- | --- |
| Defocus blur | 23.35 dB | 25.11 dB |
| Motion blur | 22.58 dB | 26.94 dB |

**Ringing control** constrains each pixel to the span of its neighbourhood in
the observation, widened by one minus the setting. At 0.8 the flat regions
beside a hard edge come back to their true levels exactly, where undamped
overshoots by 0.03, and the edge is still 1.17x steeper than the blurred input.

Relaxing the per-iteration step — the obvious reading of "damping" — was tried
first and does not control ringing: it changes the convergence path while the
overshoot *grows*.

**Iterations** are capped at 40, cancellation is checked every iteration, and a
diverged estimate returns the observation unchanged rather than writing NaN into
the document. A pixel's dependency grows by twice the kernel reach per
iteration, so most useful settings exceed the halo cap and render whole.

## JPEG cleanup

A block boundary is recognisable: the image is smooth on both sides and steps
only at the seam, while a real edge keeps going. The filter compares the
interior gradients either side against the step across the boundary and acts
only when the step dominates, then spreads the correction over three pixels
either side, turning a step into a ramp.

| Metric | Clean | Blocked | Restored |
| --- | --- | --- | --- |
| Grid energy | 3.60 | 3.77 | 3.64 |
| PSNR | — | 35.80 dB | 35.79 dB |
| Edge retention | 1.00 | 1.01 | 1.00 |

It removes about 74% of the added grid energy. The implementation it replaced
averaged the two pixels at each seam, which lost 0.84 dB and 5% of the edge
energy.

It is **global**: the 8-pixel grid is anchored to the image origin, so a tile
not starting on a multiple of eight would filter the wrong columns.

**Limitation.** It reduces the visibility of blocking. It does not recover
what the encoder discarded, and PhotoForge does not read JPEG quantisation
tables to guess what that was.

## The legacy 8-bit renderer

Documents saved before 0.10.0 use the original encoded-8-bit renderer, which is
kept working rather than modernised. It refuses `remove_defects` and
`deconvolve`: the first compares a pixel against a neighbourhood spread that at
8 bits is quantised to the same order as its own threshold, and the second
compounds quantisation error through every iteration. Refusing is honest; a
worse result under the same name would not be.

## What is not here

No restoration tool in PhotoForge invents content. Nothing infers detail that
was not in the source, and nothing fills a region from a model. Where a tool
cannot recover something, it says so rather than approximating.

See [local-inference.md](local-inference.md) for the optional neural path, and
[phase-12-results.md](phase-12-results.md) for what was and was not verified.
