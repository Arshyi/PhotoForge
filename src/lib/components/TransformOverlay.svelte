<script lang="ts">
  /**
   * The on-canvas transform box for the active pixel layer.
   *
   * Everything is drawn in document coordinates inside an SVG whose viewBox is
   * the canvas, so the box tracks the image at any viewport zoom without the
   * zoom ever becoming part of the stored transform. Handle sizes are divided
   * by the on-screen scale so they stay a constant number of screen pixels.
   *
   * The arithmetic lives in `layers/transformTool`; this component only turns
   * pointer and key events into calls on it.
   */
  import {
    applyTransformGesture,
    beginTransformGesture,
    handleCursor,
    hitTestHandle,
    nudgeTransform,
    transformFrame,
    NUDGE_STEP,
    NUDGE_STEP_LARGE,
    type Point,
    type TransformGesture,
    type TransformHandle
  } from '../layers/transformTool';
  import type { LayerTransform } from '../layers/types';

  export let transform: LayerTransform;
  export let layerWidth: number;
  export let layerHeight: number;
  export let canvasWidth: number;
  export let canvasHeight: number;
  export let aspectLocked = false;
  export let disabled = false;
  export let layerName = 'layer';
  /** Called for every pointer move in a drag, so the preview can follow. */
  export let onpreview: (transform: LayerTransform) => void;
  /** Called once when a gesture finishes, so history records one step. */
  export let oncommit: (transform: LayerTransform) => void;
  /** Called when a gesture is abandoned, so the start transform is restored. */
  export let oncancel: () => void;

  let surface: HTMLButtonElement;
  let gesture: TransformGesture | null = null;
  let activeHandle: TransformHandle | null = null;
  let hoverHandle: TransformHandle | null = null;
  let pointerId: number | null = null;

  /** Handle radius and grab tolerance in screen pixels. */
  const HANDLE_SCREEN_RADIUS = 5;
  const GRAB_SCREEN_TOLERANCE = 11;
  const GRIP_SCREEN_DISTANCE = 26;

  /** Document pixels per screen pixel, so handles keep a constant size. */
  let documentPerScreen = 1;

  function measure() {
    if (!surface || !canvasWidth) return;
    const bounds = surface.getBoundingClientRect();
    documentPerScreen = bounds.width > 0 ? canvasWidth / bounds.width : 1;
  }

  $: handleRadius = HANDLE_SCREEN_RADIUS * documentPerScreen;
  $: grabTolerance = GRAB_SCREEN_TOLERANCE * documentPerScreen;
  $: gripDistance = GRIP_SCREEN_DISTANCE * documentPerScreen;
  $: frame = transformFrame(transform, layerWidth, layerHeight, gripDistance);
  $: outline = frame.corners.map((corner) => `${corner.x},${corner.y}`).join(' ');
  $: cursor = handleCursor(
    activeHandle ?? hoverHandle ?? 'move',
    transform.rotationDegrees
  );

  /** Maps a pointer event to a document-space point. */
  function documentPoint(event: PointerEvent): Point | null {
    const bounds = surface?.getBoundingClientRect();
    if (!bounds || bounds.width <= 0 || bounds.height <= 0) return null;
    if (!Number.isFinite(event.clientX) || !Number.isFinite(event.clientY)) return null;
    return {
      x: ((event.clientX - bounds.left) / bounds.width) * canvasWidth,
      y: ((event.clientY - bounds.top) / bounds.height) * canvasHeight
    };
  }

  function handlePointerDown(event: PointerEvent) {
    if (disabled || event.button !== 0) return;
    measure();
    const point = documentPoint(event);
    if (!point) return;
    const grabbed = hitTestHandle(frame, point, grabTolerance);
    if (!grabbed) return;
    event.preventDefault();
    event.stopPropagation();
    activeHandle = grabbed;
    gesture = beginTransformGesture(grabbed, transform, layerWidth, layerHeight, point, {
      aspectLocked,
      fromCentre: event.altKey
    });
    pointerId = event.pointerId;
    surface.setPointerCapture(event.pointerId);
    surface.focus();
  }

  function handlePointerMove(event: PointerEvent) {
    const point = documentPoint(event);
    if (!point) return;
    if (!gesture) {
      if (!disabled) hoverHandle = hitTestHandle(frame, point, grabTolerance);
      return;
    }
    event.preventDefault();
    onpreview(
      applyTransformGesture(gesture, point, { shift: event.shiftKey, alt: event.altKey })
    );
  }

  /**
   * Ends the drag.
   *
   * The commit happens here rather than on every move, so a drag of a hundred
   * pointer events becomes one undo step. A release outside the box still lands
   * here because the pointer is captured.
   */
  function handlePointerUp(event: PointerEvent) {
    if (!gesture) return;
    const point = documentPoint(event);
    const final = point
      ? applyTransformGesture(gesture, point, { shift: event.shiftKey, alt: event.altKey })
      : transform;
    releasePointer();
    oncommit(final);
  }

  function releasePointer() {
    if (pointerId !== null && surface?.hasPointerCapture?.(pointerId)) {
      surface.releasePointerCapture(pointerId);
    }
    pointerId = null;
    gesture = null;
    activeHandle = null;
  }

  /** Abandons an in-flight drag, as when the pointer is cancelled by the system. */
  function abandon() {
    if (!gesture) return;
    releasePointer();
    oncancel();
  }

  function handleKeys(event: KeyboardEvent) {
    if (disabled) return;
    if (event.key === 'Escape') {
      event.preventDefault();
      if (gesture) {
        // A drag in flight is abandoned here and the box stays open, so the
        // key must not also reach the host and close it.
        event.stopPropagation();
        abandon();
      } else {
        // Nothing to abandon, so let the host close the box.
        oncancel();
      }
      return;
    }
    if (event.key === 'Enter') {
      // Finishing the interaction is the host's business; the layer keeps its
      // transform and is not rasterized. The event keeps bubbling so the host
      // can close the box.
      event.preventDefault();
      oncommit(transform);
      return;
    }
    const step = event.shiftKey ? NUDGE_STEP_LARGE : NUDGE_STEP;
    const nudges: Record<string, [number, number]> = {
      ArrowLeft: [-step, 0],
      ArrowRight: [step, 0],
      ArrowUp: [0, -step],
      ArrowDown: [0, step]
    };
    const delta = nudges[event.key];
    if (!delta) return;
    event.preventDefault();
    // Found in packaged-desktop testing: without this the window-level handler
    // nudges the same layer again, so one arrow key moved it two pixels.
    event.stopPropagation();
    oncommit(nudgeTransform(transform, delta[0], delta[1]));
  }
</script>

<svelte:window on:resize={measure} />

<button
  bind:this={surface}
  type="button"
  class="transform-surface"
  class:dragging={Boolean(gesture)}
  style={`cursor: ${disabled ? 'default' : cursor}`}
  aria-label={`Transform ${layerName}. Drag inside to move, drag a handle to scale, drag the round grip to rotate. Arrow keys nudge, Shift and an arrow key nudge further, Escape cancels.`}
  {disabled}
  on:pointerdown={handlePointerDown}
  on:pointermove={handlePointerMove}
  on:pointerup={handlePointerUp}
  on:pointercancel={abandon}
  on:lostpointercapture={abandon}
  on:keydown={handleKeys}
>
  <svg viewBox={`0 0 ${canvasWidth} ${canvasHeight}`} preserveAspectRatio="none" aria-hidden="true">
    <polygon class="bounds" points={outline} stroke-width={documentPerScreen} />
    <line
      class="tether"
      x1={frame.rotateAnchor.x}
      y1={frame.rotateAnchor.y}
      x2={frame.rotate.x}
      y2={frame.rotate.y}
      stroke-width={documentPerScreen}
    />
    <circle
      class="pivot"
      cx={frame.centre.x}
      cy={frame.centre.y}
      r={handleRadius * 0.7}
      stroke-width={documentPerScreen}
    />
    <line
      class="pivot-cross"
      x1={frame.centre.x - handleRadius}
      y1={frame.centre.y}
      x2={frame.centre.x + handleRadius}
      y2={frame.centre.y}
      stroke-width={documentPerScreen}
    />
    <line
      class="pivot-cross"
      x1={frame.centre.x}
      y1={frame.centre.y - handleRadius}
      x2={frame.centre.x}
      y2={frame.centre.y + handleRadius}
      stroke-width={documentPerScreen}
    />
    {#each frame.handles as handle (handle.id)}
      <rect
        class="handle"
        class:active={activeHandle === handle.id}
        data-handle={handle.id}
        x={handle.point.x - handleRadius}
        y={handle.point.y - handleRadius}
        width={handleRadius * 2}
        height={handleRadius * 2}
        stroke-width={documentPerScreen}
      />
    {/each}
    <circle
      class="grip"
      class:active={activeHandle === 'rotate'}
      data-handle="rotate"
      cx={frame.rotate.x}
      cy={frame.rotate.y}
      r={handleRadius * 1.1}
      stroke-width={documentPerScreen}
    />
  </svg>
</button>

<style>
  .transform-surface {
    position: absolute;
    inset: 0;
    /* Above the selection canvas at z-index 9: while the box is showing it owns
       the pointer, or its handles cannot be grabbed at all. */
    z-index: 12;
    padding: 0;
    border: 0;
    background: transparent;
    touch-action: none;
  }
  .transform-surface:focus-visible {
    outline: 2px solid var(--accent);
    outline-offset: -2px;
  }
  svg {
    width: 100%;
    height: 100%;
    display: block;
    overflow: visible;
    pointer-events: none;
  }
  .bounds {
    fill: none;
    stroke: var(--accent, #c0e77e);
    vector-effect: non-scaling-stroke;
    stroke-dasharray: 6 4;
  }
  .tether,
  .pivot-cross {
    stroke: var(--accent, #c0e77e);
    vector-effect: non-scaling-stroke;
  }
  .pivot {
    fill: none;
    stroke: var(--accent, #c0e77e);
    vector-effect: non-scaling-stroke;
  }
  .handle,
  .grip {
    fill: #10140c;
    stroke: var(--accent, #c0e77e);
    vector-effect: non-scaling-stroke;
  }
  .handle.active,
  .grip.active {
    fill: var(--accent, #c0e77e);
  }
  .dragging {
    cursor: grabbing;
  }
</style>
