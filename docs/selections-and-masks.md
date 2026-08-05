# Selections and masks

PhotoForge 0.7.0 provides a local, non-generative selection system. The canonical mask is an image-sized row-major array of unsigned 8-bit coverage values: `0` is unselected, `255` is fully selected, and intermediate values blend feathered edges. The source image is never mutated.

## Tools and coordinates

Rectangle and ellipse tools drag in image space and support fixed 1:1 geometry and drawing from center. Freehand lasso records pointer samples, removes redundant samples deterministically, and closes the path. Polygon lasso uses clicks, double-click or Enter to close, Backspace to remove the latest point, and Escape to cancel. Self-intersections use the even-odd rule. Geometry is anti-aliased with deterministic subpixel coverage.

The selection brush uses diameter, hardness, and opacity. Strokes are interpolated by image-space distance, not animation frames, so rapid movement does not leave gaps. The eraser uses the same rasterizer with Subtract composition. Pointer pressure is not used in 0.7.0.

Magic wand uses an iterative bounded queue, 4- or 8-neighbor connectivity, tolerance, optional non-contiguous matching, optional anti-alias falloff, alpha-aware comparison, cancellation checks, and deterministic traversal. It never recurses. Color range compares samples in documented linear-sRGB HSV-like features: relative luminance, circular hue, and saturation. Hue weight approaches zero for low-saturation colors. Fully transparent pixels are unselected; partial alpha scales coverage. Sequential Shift/Add or Alt/Subtract samples provide deterministic multi-sample behavior.

Viewport coordinates are converted from the transformed interaction surface into full-resolution image coordinates. Zoom, CSS scaling, and high-DPI display scaling therefore do not change recorded geometry. Split comparisons disable selection input. A mask may be resampled to the bounded preview only when its aspect ratio still matches the current processing stage.

## Composition

All tools share four modes:

| Mode | Coverage result |
| --- | --- |
| Replace | incoming |
| Add | `max(existing, incoming)` |
| Subtract | `existing × (1 - incoming)` |
| Intersect | `existing × incoming` |

Shift temporarily selects Add, Alt selects Subtract, and Shift+Alt selects Intersect. The toolbar mode is used when no modifier is held. Selection eraser always subtracts.

## Mask operations and refinement

Select All, Deselect, Invert, Feather, Expand, Contract, Smooth, Fill Holes, Remove Small Islands, and Border are undoable. Radius, border, and morphology inputs are full-resolution image pixels and are clamped to bounded ranges. Expand and contract use deterministic square morphology; feather uses a bounded separable box kernel. Fill Holes and island cleanup use iterative connected-component traversal.

Refine Selection applies smooth, feather, contrast, and shift-edge parameters, then uses local transparent-aware Sobel gradients to increase mask-edge contrast where the boundary aligns with an image edge. This is classical image processing, not AI, semantic segmentation, or invented detail. Decontaminate Colors is intentionally omitted because it would alter image color rather than only mask coverage. Overlay backgrounds and selection undo provide review; 0.7.0 does not include a separate before/after refinement dialog.

## Overlays

Overlay state is UI-only and is never exported into the edited image. Modes are marching ants, translucent color, grayscale, black background, white background, and mask-only. Color and opacity are configurable. Rendering is bounded to the preview canvas while the canonical mask remains full resolution.

## Selective adjustments

The adjustment scope is Global, Inside selection, or Outside selection. Global preserves all Phase 1–6 behavior and serialization. Inside and Outside wrap a supported base `EditOperation` with an immutable mask snapshot; the deterministic engine renders the base adjustment, then blends RGB channels by mask coverage while preserving source alpha.

Brightness, contrast, saturation, gamma, grayscale, sepia, blur, sharpen, auto white balance, local contrast, denoise, deblock, edge-aware sharpen, mild deblur, document enhancement, uneven-lighting correction, curves, levels, point balance, HSL, temperature/tint, and selective color support masks. Dimension-changing or coordinate-warping edits—reflect, rotate, crop, straighten, perspective, and lens correction—remain global. A masked edit used after a stage with a changed aspect ratio fails rather than applying to the wrong coordinates.

## Named masks and history

Named masks have a stable identifier independent of display name, timestamps, source-tool metadata, dimensions, visibility, lock state, and immutable coverage. The panel supports create, rename, duplicate, delete, lock, visibility, reorder, load, replace from active, combine using the current composition mode, JSON export, and grayscale PNG export/import.

Selection history stores logical completed operations, not pointer frames. A continuous brush stroke is one undo action. History is capped at 60 entries and approximately 64 MiB; old entries are evicted to keep memory bounded. Edit and selection actions participate in the application Undo/Redo ordering. Named-mask thumbnails are compact UI glyphs in 0.7.0 rather than generated bitmap previews.

## Session persistence and limitations

Tool settings, overlay settings, active selection, named masks, composition mode, adjustment scope, and panel state are associated with a document fingerprint and restored from local WebView storage when the encoded session fits a 3.5-million-character ceiling. Coherent masks usually use compact run-length encoding. If a noisy mask set exceeds the ceiling, editing continues and PhotoForge asks the user to export important masks explicitly. PhotoForge does not yet have a general project-file system, so a bounded mask session is not a substitute for an exported mask library.

Named-mask visibility is persisted, but the canvas renders the active selection rather than compositing every visible named mask. Masks do not remap through existing crop, perspective, straighten, or 90-degree rotation operations; place masked adjustments before dimension-changing geometry or re-create the mask for the transformed document.

## Keyboard behavior

| Shortcut | Action |
| --- | --- |
| `Ctrl+A` | Select All |
| `Ctrl+D` | Deselect |
| `Ctrl+Shift+I` | Invert |
| `M` | Cycle rectangle/ellipse |
| `L` | Cycle freehand/polygon lasso |
| `W` | Magic wand |
| `B` | Selection brush |
| `E` | Selection eraser |
| `Q` | Toggle overlay |
| `Enter` | Close polygon |
| `Backspace` | Remove polygon point |
| `Escape` | Cancel active polygon or processing |

Shortcuts do not fire inside inputs, textareas, selects, or editable content. `C` remains the established Compare shortcut, so Color Range has no single-key shortcut.
