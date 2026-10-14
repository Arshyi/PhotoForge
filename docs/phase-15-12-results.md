# Phase 15.12 — the window at other widths and scales, and the panel's own controls

[15.9.1](phase-15-9-1-results.md) and [15.9.2](phase-15-9-2-results.md) left two things open for
the packaged window: the Layers panel's controls used a different type scale from the cards, and
nothing had been looked at other than at the default 1,360 × 860 at 100 %. Looking at the
rest turned up more than a type scale.

Status: **implemented and verified as listed; integrated for 0.15.1.** Developed on
`phase-15-10-clipboard-window` with Phases 15.10 and 15.11, from `main` (`242f6f9`). Interface
only: two style files and no logic. The later command-palette repair and current integration
checks are recorded separately in [0.15.1](release-0-15-1.md).

## What was found

A measure of every control in the right panel (computed style) and the layout at a matrix of
window sizes, on the 0.15.0 build:

1. **Most of the panel's own controls were the browser's.** The Layers, Transform and Selections &
   Masks panels' buttons were light grey with black text (white inputs in Transform, disabled
   buttons grey on grey), and the "Adjustments go to" select was 16 px against the cards' 9.6 px.
   41 to 43 controls per window were in the browser's own look; the panel's controls used **12
   different font sizes, 8.3 to 16 px**.
2. **Under about 1,350 px the whole app was wider than its window.** The shell had no column of
   its own, so its width was the header's content (about 1,350 px with every label); in a narrower
   window the right panel and the Settings button were drawn past the edge and could not be
   reached (measured: at 900 px Settings ended at 1,002 and at 1,100 px at 1,215). A 1,280 px
   window, which is a 1920 × 1080 display at 150 %, is in the same range and was not measured
   before the fix. 15.9.2 had fixed this for exactly 1,360.

## What changed

* `src/styles.css`
  * `.app-shell` has one column that cannot be wider than the window, and `.topbar` may shrink;
  * up to 1,400 px the header buttons keep their icon and lose their word (the name stays as the
    tooltip and the accessible name; *Open* keeps its word). The existing 920 px and 700 px rules
    are unchanged;
  * the editing panel's buttons, selects and text inputs have the cards' look and scale
    (0.6 rem, `--surface-soft`, the line colour) unless a component styles them itself.
* `src/lib/components/LayersPanel.svelte`: the select no longer inherits the panel's 16 px, and
  the panel's text sizes are the cards' (0.6 rem for a control and a label, 0.58 rem for help, 0.66
  rem for a layer's name; they ranged from 0.55 to 0.72).

**A first attempt did not work and the matrix said so.** The rule was written with `:where()` to
keep its specificity at zero; the global `button, input { font: inherit }` beats a zero-specificity
rule, so 45 controls stayed at 16 px. It uses ordinary element selectors now (a component's own
class is more specific, so every component rule still wins).

## Verification

**The layout matrix** (`tools/validate/layout_matrix.ps1`, packaged executable, a document open):
8 sizes (760 × 560, which is the minimum window, 900 × 700, 1,100 × 760, 1,280 × 720, 1,366 × 768,
1,400 × 800, 1,536 × 864, 1,920 × 1,080) at 4 device pixel ratios (1, 1.25, 1.5, 2): **32 of 32
cells clean** (no sideways scroll, no header button past the edge, no control outside the panel or
clipping its text, no control in the browser's own look). On the 0.15.0 build, the same probe found
41 to 43 problems in every cell. Screenshots of both are in the work directory
(`shots/layout-before-*`, `layout-*`).

| Gate | Result |
| --- | --- |
| `npx tsc --noEmit`, `npm run check` | 0 errors, 0 warnings (524 files) |
| `npx vitest run` | **1,392** tests in 94 files passed (1,389 and 93 before: three tests of the layout rules in `src/styles.layout.test.ts`). This historical run had an unhandled `CommandPalette` error, subsequently fixed and rechecked for [0.15.1](release-0-15-1.md) |
| `npm run build` | succeeds |
| Mutation checks (`tools/mutate_layout.py`) | **5 of 5 caught**: the shell with no column, the header with no `min-width`, the header keeping its labels, the panel controls not matched, the panel controls at 1 rem |
| Rust gates | see 15.11 (no Rust changed for this) |

## What was not verified

* **Windows' own display scaling.** Changing the scale is a system setting and was not touched. The
  matrix shows the page at the same CSS size and device pixel ratio, which is what the layout
  depends on; it does not show how Windows scales the window frame, the title bar, or WebView2's
  own UI at 125 to 200 %, nor a window dragged between two monitors of different scales.
* **The high-contrast theme** (`data-high-contrast`): the new rules use the theme's own colours;
  it was not looked at.
* **Every panel in every state.** The matrix has one document open and the panels that shows. The
  dialogs (Settings, the memory dialog, the RAW chooser) were not run through it; they style their
  own controls.
* **Reading the result as a person would.** The check is numeric plus a few screenshots looked at
  (1,280 × 720 and 760 × 560); nobody used the window at those sizes.

## Known and left

* The Adjustments sliders' reset buttons are 10.9 px, another scale (a separate set of controls,
  consistent among themselves).
* Below 700 px the right panel is hidden by an existing rule; the window's minimum is 760 px, so
  it is reached only with the page zoomed.
