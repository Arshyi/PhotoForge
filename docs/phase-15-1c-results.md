# Phase 15.1c — the blend modes Photoshop adds

[15.1](phase-15-1-results.md) imported eleven blend modes PhotoForge did not have as Normal and
named them. They were the most common reason a picture differed that was not a layer style or an
adjustment layer (80 layers in the 316 test files). This increment adds ten of them to the layer
model and all three renderers, so those layers import as themselves and a document can use them.

Status: **implemented and verified as listed; not committed, not released.** The version is still
0.14.0. It sits on top of [15.1b](phase-15-1b-results.md).

## Completed

* **Ten blend modes**: Linear Burn, Linear Dodge (Add), Darker Color, Lighter Color, Vivid Light,
  Linear Light, Pin Light, Hard Mix, Subtract and Divide — 26 in all. The formulas are in
  [compositing.md](compositing.md#blend-modes). They go through the one blend function every
  renderer already shared, so the 8-bit, float and tiled renderers agree on them (the random-tree
  checks now draw from all 26 modes).
* **The interface**: the Blend list, the macro editor and the registry's blend parameter offer all
  26, grouped as Photoshop groups them.
* **The importer** maps Photoshop's keys (`lbrn`, `lddg`, `dkCl`, `lgCl`, `vLit`, `lLit`, `pLit`,
  `hMix`, `fsub`, `fdiv`) to them. **Dissolve** stays unsupported and is reported.
* **A report line for a group's fill opacity** when its layers blend with what is beneath (below).

## Checked against Photoshop, not only against the formulas

The psd-tools test suite has two charts, `gray-blend-modes.psd` and `rgb-blend-modes.psd`: every
mode applied by a duck (a smart object, so the pixels Photoshop used are the ones stored) over a
coloured rectangle, with Photoshop's own flattened picture stored in the file. Importing and
rendering them and comparing, layer by layer, on the 2,417 pixels where the duck has coverage:

| | pixels differing by more than 3 levels — RGB chart / grey chart |
| --- | --- |
| Normal, Multiply, Screen, Overlay, Darken, Lighten, Hard Light, Difference, Exclusion, Saturation | 0 / 0 |
| **Linear Burn, Linear Dodge, Linear Light, Pin Light, Subtract** | **0 / 0** |
| Darker Color, Lighter Color | 6, 6 / — |
| Divide | 6 / 0 |
| Hard Mix | 12 / 6 |
| Vivid Light | 59 / 7 |
| Color Dodge, Color Burn, Soft Light (existing) | 91, 3, 8 / 3, 8, 0 |
| Hue, Color, Luminosity (existing) | 27, 24, 19 / — |

The whole RGB chart: mean error 0.20 levels, 99.3% of pixels within 2 (it was 2.07 and 97.7% before
the checks below moved three modes); the grey chart 0.30 and 99.8%.

The residue is not the formulas. Every remaining pixel is in a region where a mode has a gain of 4
or more (dodge, burn, vivid light, divide) or near black, and the differences are plus or minus a
few levels in both directions; they are what a one-level difference in the source or the backdrop
makes, and the duck is a scaled smart object that Photoshop may sample differently from the cache it
stores.

The comparison also found three things the formulas as documented got wrong, now fixed:

* **Darker Color and Lighter Color compare luminosity, not the sum of the channels.** Photoshop's
  help text says the sum. Its picture says luminosity: with the sum those cells were off by 20
  levels on average, with luminosity by 0.2. (The weights 0.3/0.59/0.11 and 0.299/0.587/0.114
  cannot be told apart on this data; the ones the Hue and Color modes use are used.)
* **Vivid Light with a source of exactly black or white is that colour**, whatever lies beneath. At
  the two 0/0 corners (white backdrop under black, black under white) Color Burn and Color Dodge
  say the opposite and Photoshop's Vivid Light does not; 318 mismatching pixels became 59.
* **Hard Mix keeps a black backdrop black under white.** It is otherwise the threshold on the sum
  its help text describes. 147 mismatching pixels became 12.

## A defect this made visible

`passthrough_fill_blendmode.psd` — a pass-through group at 47% fill whose layer is Lighter Color —
used to import as "approximated" because Lighter Color was unsupported. With the mode supported it
imported "clean" and did not match (mean error 3 levels, 51% within 2). Photoshop applies a group's
fill opacity to what is blended in a way that is not a cross-fade at that opacity — the data fit an
effective strength of about the fill squared — and one file is not enough to say what the rule is.
It is now reported (*approximated*) for any group with a fill opacity below 100% that is
pass-through or holds layers that blend; the real-file check that no "clean" import may differ
found it, which is what that check is for.

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

## What is not done

* **Dissolve.** A random dither; PhotoForge has no reproducible equivalent.
* **The group fill-opacity rule** (above) is reported, not reproduced.
* **The remaining mismatches in dodge, burn and vivid light** are unexplained to the level of "a
  one-level difference in the source", which was not proved. A Photoshop run on the same file, or
  another file with a source that is not a smart object, would settle it.
* **A document that uses these modes cannot be opened by an older PhotoForge**, which rejects a
  blend mode it does not know rather than treating it as Normal.

New tests: 3 in `blend.rs` (every new mode against hand-computed values, each one's neutral operand,
and the corners — divide by black, vivid light at black and white, hard mix over black, equal
luminosity), 3 in the importer (every Photoshop key maps to its mode with nothing reported, Dissolve
is reported, the group fill-opacity line). The random-tree comparisons of the tiled renderer with the
float one, and of the cache with a fresh render, now draw from all 26 modes.

One mutation did not fail a test: taking the explicit black/white source guard out of Vivid Light
changes nothing, because dividing by zero in `f32` and taking `min(1)` of the NaN already gives
Photoshop's answer at both corners. The guard stays, so the behaviour is stated and not left to
that, and the test that pins the corners passes either way.