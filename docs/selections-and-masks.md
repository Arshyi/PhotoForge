# Selections and masks

PhotoForge 0.7.1 provides a local, non-generative selection system. The canonical mask is an image-stage-sized row-major array of unsigned 8-bit coverage values: `0` is unselected, `255` is fully selected, and intermediate values blend feathered edges. The source image is never mutated.

## Tools and coordinates

Rectangle and ellipse tools drag in image space and support fixed 1:1 geometry and drawing from center. Freehand lasso records pointer samples, removes redundant samples deterministically, and closes the path. Polygon lasso uses clicks, double-click or Enter to close, Backspace to remove the latest point, and Escape to cancel. Self-intersections use the even-odd rule. Geometry is anti-aliased with deterministic subpixel coverage.

The selection brush uses diameter, hardness, and opacity. Strokes are interpolated by image-space distance, not animation frames, so rapid movement does not leave gaps. The eraser uses the same rasterizer with Subtract composition. Optional pen pressure is disabled by default. When enabled, finite Pointer Events pressure from a pen can affect size, opacity, or both, with user-controlled minimum factors. Mouse, touch, missing pressure, and malformed pressure retain the ordinary fixed brush behavior. Each accepted pen sample is resolved to finite, clamped diameter and opacity values before it crosses the command boundary, so replay never depends on live hardware input.

Magic wand uses an iterative bounded queue, 4- or 8-neighbor connectivity, tolerance, optional non-contiguous matching, optional anti-alias falloff, alpha-aware comparison, cancellation checks, and deterministic traversal. It never recurses. Color range compares samples in documented linear-sRGB HSV-like features: relative luminance, circular hue, and saturation. Hue weight approaches zero for low-saturation colors. Fully transparent pixels are unselected; partial alpha scales coverage. Sequential Shift/Add or Alt/Subtract samples provide deterministic multi-sample behavior.

Viewport coordinates are converted from the transformed interaction surface into current image-stage coordinates. Zoom, CSS scaling, and high-DPI display scaling therefore do not change recorded geometry. Selection input is disabled only while a comparison view is active. Canonical snapshots must exactly match their full-resolution pipeline stage; the preview path creates a temporary bilinear coverage mask at the corresponding bounded preview stage without mutating the stored snapshot.

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

Refine Selection applies smooth, feather, contrast, and shift-edge parameters, then uses local transparent-aware Sobel gradients to increase mask-edge contrast where the boundary aligns with an image edge. This coverage refinement is classical image processing, not AI, semantic segmentation, or invented detail. The dedicated modal keeps an immutable before snapshot and debounces preview requests. It supports split or before/after toggle comparison plus original-image, black, white, and mask-only backgrounds. Reset restores the dialog defaults; Cancel or Escape discards the preview; Apply, or Enter away from an interactive control when a valid preview exists, commits exactly one logical selection-history entry. Repeated preview changes do not populate Undo history.

Decontaminate Colors is a separate opt-in image-edit contract within the Refine dialog and is disabled by default. It considers only partial-coverage edge pixels, samples non-transparent pixels with confident foreground coverage (`224…255`) inside a circular radius, weights them deterministically by coverage and distance, blends RGB by strength `0…1`, and preserves the source alpha byte. Radius is an integer `1…32`; the implementation rejects work beyond its bounded neighborhood ceiling instead of attempting an unbounded correction. It is always wrapped in the refined immutable mask, is rejected as a global or unmasked operation, and supports inverted masked replay.

The dialog's After canvas uses the same circular sampling and integer blend at its bounded thumbnail resolution; the Before canvas is never decontaminated. This is a responsive, representative comparison, not a claim that a thumbnail-radius result is pixel-for-pixel identical to full resolution. Apply records the refined selection and, when decontamination is enabled, its masked image operation as one compound Undo/Redo event. The ordinary preview pipeline then renders a bounded-stage representation, while export evaluates the validated operation against the exact full-resolution mask stage.

## Overlays

Overlay state is UI-only and is never exported into the edited image. Modes are marching ants, translucent color, grayscale, black background, white background, and mask-only. Color and opacity are configurable. The stage combines the active mask and visible named masks for review while preserving each mask independently. Rendering is bounded to the preview canvas while canonical masks remain at their exact full-resolution stages.

## Selective adjustments

The adjustment scope is Global, Inside selection, or Outside selection. Global preserves all Phase 1–6 behavior and serialization. Inside and Outside wrap a supported base `EditOperation` with an immutable mask snapshot; the deterministic engine renders the base adjustment, then blends RGB channels by mask coverage while preserving source alpha.

Brightness, contrast, saturation, gamma, grayscale, sepia, blur, sharpen, auto white balance, local contrast, denoise, deblock, edge-aware sharpen, mild deblur, document enhancement, uneven-lighting correction, curves, levels, point balance, HSL, temperature/tint, selective color, and Decontaminate Colors support masks. Geometry operations remain global, but crop, 90°/180°/270° rotation, horizontal reflection, straighten, perspective, and lens correction now remap persistent masks to the same image content before the geometry transaction is committed.

## Geometry remapping

Geometry edits are committed transactionally across the edit pipeline, active mask, named masks, and persistent embedded workflow masks. Every item identifies its old and new pipeline stage. The Rust boundary validates the complete batch and returns every requested key, or the frontend commits nothing. Undo/Redo restores the edit and selection states as one compound action.

- Crop uses the processor's normalized-coordinate floor/round/clamp rules and removes coverage outside the retained rectangle. A fully excluded mask becomes an empty mask at the new dimensions; identifiers and named-mask metadata remain intact.
- Exact quarter turns and horizontal reflection use discrete remapping, preserving byte coverage without interpolation. Exact inverses restore the original coverage.
- Straighten and general chain changes use one destination-to-source bilinear coverage sample. Out-of-bounds samples are zero and output is clamped to `0…255`.
- Perspective uses the processor-equivalent bilinear quadrilateral mapping with a bounded inverse. Non-finite, folded, near-singular, or non-convergent transforms fail before any state is committed.
- Lens correction uses the processor-equivalent normalized radial distortion map for the image's green/alpha sample. Vignetting changes intensity and chromatic aberration offsets red/blue independently, so those two settings do not move scalar mask coverage. Distortion is limited to `-0.16…1`, its inverse is iteration-bounded, zero distortion is an exact mask identity, and uncovered output samples become zero.

The remapper accepts at most 200 geometry steps and a bounded 256-item mask transaction. All geometry chains start from the same original document dimensions. A stale document, mismatched stage dimension, duplicate/missing result key, cancellation, folded or near-singular lens/perspective mapping, or other invalid transform fails closed rather than discarding or misaligning a mask.

## Named masks and history

Named masks have a stable identifier independent of display name, timestamps, source-tool metadata, dimensions, visibility, lock state, and immutable coverage. The panel supports create, rename, duplicate, delete, lock, visibility, reorder, load, replace from active, combine using the current composition mode, JSON export, and grayscale PNG export/import. File operations capture the document, request, pipeline, and selection generation before the native dialog and recheck them after it closes and after processing, so a stale result cannot overwrite newer workspace state.

Selection history stores logical completed operations, not pointer frames. A continuous brush stroke is one undo action. History is capped at 60 entries and approximately 64 MiB; old entries are evicted to keep memory bounded. Edit and selection actions participate in the application Undo/Redo ordering.

Named-mask rows use real grayscale thumbnails generated from decoded coverage with area averaging and aspect-fit dimensions. Rendering is lazy through visibility observation and idle scheduling. Cache keys include the mask checksum, source dimensions, and target dimensions, so a modified or remapped mask invalidates only its affected thumbnail. The shared least-recently-used cache is bounded to 96 entries and approximately 2 MiB, and thumbnail generation performs no disk or network access.

## Numerical progress and cancellation

Potentially visible mask computation publishes typed progress containing the document ID, request ID, operation, phase, completed work units, total work units, and state. Work units come from rows, pixels, passes, or geometry-output rows used by the implementation. The UI waits 180 ms before revealing the indicator, keeps a displayed percentage monotonic and bounded, ignores stale requests, and clears terminal completed, cancelled, or failed states. Cancellation changes the state to `cancelling` until the worker acknowledges it.

Rasterization/composition, Magic Wand, Color Range, feather/morphology/cleanup/refinement, geometry remapping, and JSON/PNG mask import/export use this mechanism. Mask file I/O runs asynchronously on a blocking worker. JSON reports actual file-byte, mask-decode, and validation work; PNG reports actual file-byte, conversion, mask-encoding, and row-encoding work. JSON parsing and PNG codec phases are labeled without invented local units because their libraries do not expose trustworthy incremental callbacks; the displayed overall percentage remains monotonic and is based on measured surrounding work. Cancellation is checked between real chunks and before atomic destination replacement, so cancellation preserves an existing destination and removes the incomplete secure temporary file. Very fast work therefore does not flash a progress bar.

## Session persistence and limitations

Tool settings, overlay settings, active selection, named masks, composition mode, adjustment scope, panel state, current canvas dimensions, and canonical geometry operations are associated with a document fingerprint and restored from local WebView storage when the encoded session fits a 3.5-million-character ceiling. The current document key hashes the normalized source path together with the original image dimensions; the storage key contains dimensions and the hash, never the plaintext path. Session schema 2 stores a geometry fingerprint and rejects incoherent dimensions or future schema versions. A valid schema-1 Phase 7 session is read and migrated in memory with empty geometry and conservative pressure defaults; only schema 2 is written. The older filename-and-dimensions key is a read-only fallback and loaded state is rekeyed in memory without an automatic write. Named-mask count is capped at 100.

Coherent masks usually use compact run-length encoding. If a noisy mask set exceeds the ceiling, editing continues and PhotoForge asks the user to export important masks explicitly. PhotoForge does not yet have a general project-file system, so a bounded mask session is not a substitute for an exported mask library. Document association hashes normalized path and dimensions rather than source pixels: moving or renaming a file changes its current identity, while different content written in place at the same dimensions retains it. Users should export important masks instead of treating this local convenience key as a content identity.

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
| `C` | Color Range |
| `Q` | Toggle overlay |
| `Enter` | Close polygon |
| `Backspace` | Remove polygon point |
| `Escape` | Cancel active polygon or processing |

Shortcuts do not fire inside inputs, textareas, selects, or editable content. The selection workspace handles `C` as Color Range when selection input owns the key; the application-level Compare action remains available through its configured control/shortcut context.
