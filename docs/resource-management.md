# Resource management

PhotoForge used to have one resource policy: a compile-time constant of 1 GiB per
working image, which set a 67.1-megapixel ceiling on every machine alike. That
conflated two different questions and answered both with a number that fitted none
of the machines it ran on. It is gone. Asking "how big may an image be" is now a
call that returns **this machine's** answer.

## Two kinds of limit

**Structural limits** say whether a file or a document is *well formed enough to do
arithmetic on*. They depend on nothing but the code, and no budget, however large,
lifts them:

| Limit | Value | Why |
| --- | --- | --- |
| Pixels in a canvas | 2³¹ (2,147,483,648) | Render code indexes pixels with 32-bit arithmetic; above 2³² it would wrap silently in a release build. At 2³¹ both signed and unsigned index math are exact. |
| Longest canvas edge | 131,072 | Text and vector coordinates are `f32`; at this size an `f32` still resolves 1/64 of a pixel, enough for antialiasing to be right. |
| Longest edge of a *source file* described | 2³¹ − 1 | A source is only ever read through a bounded window, so it may far exceed anything that could be opened whole; the tiling code's `i32` coordinates set the limit. |

**The budget** says how much memory PhotoForge may *spend*. It depends on the machine
and on the person, and it is the only thing that varies.

## The budget

| Mode | Value |
| --- | --- |
| **Automatic** | Of what is left of installed memory after a **reserve**, and of what is free right now less half the reserve, take the smaller and keep **85%** of it — headroom for allocator fragmentation and for the transient peak of an operation that briefly needs more than its steady state. Never above 90% of installed memory, never below 512 MiB. |
| **Manual** | A figure the person chooses, clamped to what the machine can honestly support (at most 90% of installed memory, at least 512 MiB). If a request was moved, the budget says so. |
| **Unmeasured** | If the machine cannot be measured, Automatic falls back to **4 GiB**, which is exactly what the old constants implied, and the interface says the figure is a conservative default and not a measurement. A manual figure is then honoured up to 64 GiB. |

The **reserve** held back for Windows, WebView2, the GPU driver and whatever else is
running is the larger of 2 GiB and 15% of installed memory.

These are policy fractions of *measured* memory, not fixed sizes, and they are
choices — the 85%, the 15% and the quarter below were picked for headroom, not
derived from an optimisation. The measurements in `docs/phase-14-results.md` check
that the estimates built on them are conservative on this machine; they do not prove
the fractions optimal on others.

Worked from the formula, not special-cased:

| Machine | Free now | Automatic budget | Largest image opened whole |
| --- | --- | --- | --- |
| 8 GiB | 4 GiB | about 2.6 GiB | about 43 MP |
| 16 GiB | 9 GiB | about 6.6 GiB | about 111 MP |
| 128 GiB | 103 GiB | about 79 GiB | about 1,330 MP (below the 2,147 MP structural ceiling) |
| not measurable | — | 4 GiB | 67.1 MP, as before |

The Settings → **Memory** page shows installed and free memory, what PhotoForge is
using, the budget in force and where it came from, the manual range, and what the
budget lets a person open. A figure that could not be measured says so rather than
showing zero. **Video memory is not measured**: wgpu reports limits, not free
memory, and presenting a limit as if it were free memory would be worse than
reporting nothing, so GPU work is bounded by tile size instead.

### What the budget limits, and what is derived from it

Every ceiling in the application is a view of the one budget:

| Limit | Value |
| --- | --- |
| Largest single job | the whole budget |
| Largest single float working image | a quarter of the budget (never below 64 MiB), because opening one costs the image, a canvas frame, scratch and output |
| Pixel buffers the session retains, across layers and history | a quarter of the budget, or the working image size if larger |
| Largest single decoded project entry | the same |
| Memory one plugin call may use | what its manifest asks, capped at 2 GiB, and at most a quarter of the budget |

### The limits do not move while a document is open

`LayerDocument::validate` consults these limits on every command that receives a
document. If they followed instantaneous free memory, a document that validated a
moment ago could fail later because a browser tab grew. So the limits come from a
**snapshot**, refreshed when a document opens and when the person changes the
setting, never in between.

* **Raising** the budget applies at once: limits that only grow cannot invalidate
  anything already open.
* **Lowering** it is saved and applies when the next document opens, because a
  document that is open was admitted under the old limits. The Memory page and the
  command say which happened.

Live free memory is used in one place only: the **admission planner**, below, which
treats it as advice and labels any refusal that rests on it as *momentary*.

The setting is saved in `%LOCALAPPDATA%\PhotoForge\settings\resources.json`. A missing,
unreadable, oversized or corrupt file means Automatic, which is also a first run; a
failure to *write* is reported, because the person asked for something that did not
stick.

## Pricing work before doing it

Estimates price a job in bytes, with checked arithmetic, **before anything is
allocated**:

* `ResourceEstimate::pipeline` — a render or edit: the resident source, scratch frames
  (16 bytes per pixel per frame an operation needs) and the output;
* `ResourceEstimate::decode` — developing a camera RAW (52 bytes per pixel while the
  sensor, normalised plane, demosaic and float image coexist);
* `ResourceEstimate::render` — a layer document at a scale, counting the layer tree's
  depth, adjustment frames and masks.

An estimate over the budget is refused with the numbers (`required`, `limit`), not a
generic error. For the decoders that read oversized sources the estimates are checked
against measured allocations by `tests/estimate_accuracy.rs`, which fails if a decoder
holds more than the planner priced (see [oversized-images.md](oversized-images.md)); it
found, and the work fixed, a WebP price that was less than half the truth. Full-frame jobs, batch included, are serialised by `acquire_job`, which
waits with cancellation, so two heavy jobs cannot each be admitted against the same free
memory.

## Admission: full, region, reduced, or refuse

A file can be perfectly decodable and still not be openable as a conventional
document on a given machine: a 100 MP JPEG is 400 MB of ordinary bytes and 1.6 GB of
working float. The old behaviour was one error for both "this is not an image we can
read" and "we can read it, but not like that". The **admission planner** keeps them
apart. It takes what the header says and what the decoder for that format can
honestly do, and the budget, and returns a verdict and the alternatives that
genuinely exist:

| Verdict | Meaning |
| --- | --- |
| Full resolution | Opens as an ordinary document with room to spare. |
| Full resolution with a warning | Opens, but its peak is more than half the budget; said once after opening. |
| Region required | Too large whole; a bounded rectangle can be opened. |
| Reduced copy recommended | Too large whole and a region cannot be decoded cheaply, but a smaller copy can. |
| Insufficient resources | Nothing bounded works. *Momentary*: the budget would allow it but free memory does not right now, and closing programs would change the answer. *Budget*: the budget itself is too small. |
| Unsafe | Refused outright: a zero dimension, a dimension no coordinate can hold, or a header that claims more raster than the file's compression could possibly hold (PNG 1,100:1, JPEG 4,096:1 — so the dimensions are not honest). |

What each option costs is computed, not guessed. For a region the planner exposes four
coefficients (a fixed and a per-pixel cost for *opening* and for *editing*), so the
region selector can price a rectangle while it is being dragged using the same formula
the planner used, with no second copy of the model to drift. The details per format —
what is decoded, what is held, what is *not* claimed — are in
[oversized-images.md](oversized-images.md).

**Out-of-core editing is not offered**, and the report says why in every case; see that
document.

## What this does not claim

* **Not a guarantee against out-of-memory.** The estimates are conservative on the
  machine measured (the results document gives the ratios) and the budget leaves
  headroom, but PhotoForge shares the machine with everything else, and memory a
  library allocates internally is only as predictable as the library.
* **Not a measure of the GPU.** See above.
* **Not live.** The budget is a snapshot by design; a machine that gets busier while a
  document is open is not re-judged until the next open.
* **Not tuned.** The fractions are conservative choices; none is claimed optimal.
