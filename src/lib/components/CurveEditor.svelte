<script lang="ts">
  import {
    addCurvePoint,
    curveChannels,
    identityCurve,
    isIdentityCurve,
    MAX_CURVE_POINTS,
    moveCurvePoint,
    removeCurvePoint,
    sampleCurve
  } from '../layers/adjustments';
  import type { CurvePoint, CurveSet } from '../types/editor';

  export let curves: CurveSet;
  export let onchange: (curves: CurveSet, coalesceKey?: string) => void;
  export let disabled = false;

  /** Editing area in SVG user units; the viewBox keeps it resolution free. */
  const SIZE = 100;

  let channel: keyof CurveSet = 'rgb';
  let draggingIndex: number | null = null;
  let surface: HTMLDivElement;

  $: points = curves[channel];
  $: pathData = buildPath(points);

  const channelLabels: Record<keyof CurveSet, string> = {
    rgb: 'RGB',
    red: 'Red',
    green: 'Green',
    blue: 'Blue'
  };

  const channelColors: Record<keyof CurveSet, string> = {
    rgb: 'var(--accent)',
    red: '#e8776f',
    green: '#7fd18a',
    blue: '#7aa8e8'
  };

  /** Samples the curve densely so a multi-segment shape reads as one line. */
  function buildPath(current: CurvePoint[]): string {
    const steps = 64;
    const commands: string[] = [];
    for (let step = 0; step <= steps; step += 1) {
      const input = step / steps;
      const output = sampleCurve(current, input);
      commands.push(
        `${step === 0 ? 'M' : 'L'}${(input * SIZE).toFixed(2)},${(SIZE - output * SIZE).toFixed(2)}`
      );
    }
    return commands.join(' ');
  }

  function toCurveSpace(event: { clientX: number; clientY: number }): {
    input: number;
    output: number;
  } {
    const bounds = surface.getBoundingClientRect();
    return {
      input: (event.clientX - bounds.left) / (bounds.width || 1),
      output: 1 - (event.clientY - bounds.top) / (bounds.height || 1)
    };
  }

  function update(next: CurvePoint[], coalesceKey?: string) {
    onchange({ ...curves, [channel]: next }, coalesceKey);
  }

  /**
   * Dragging captures the pointer on the point itself, so movement continues to
   * reach it even once the cursor leaves the small circle. That keeps every
   * event handler on a genuinely interactive element.
   */
  function handlePointerDown(event: PointerEvent, index: number) {
    if (disabled) return;
    event.stopPropagation();
    event.preventDefault();
    // Alt-click removes an interior point, matching the hint under the grid.
    if (event.altKey) {
      update(removeCurvePoint(points, index));
      return;
    }
    draggingIndex = index;
    (event.currentTarget as Element).setPointerCapture?.(event.pointerId);
  }

  function handlePointerMove(event: PointerEvent) {
    if (draggingIndex === null || disabled) return;
    event.preventDefault();
    const { input, output } = toCurveSpace(event);
    update(
      moveCurvePoint(points, draggingIndex, input, output),
      `curve:${channel}:${draggingIndex}`
    );
  }

  function handlePointerUp(event: PointerEvent) {
    if (draggingIndex === null) return;
    (event.currentTarget as Element).releasePointerCapture?.(event.pointerId);
    draggingIndex = null;
  }

  /** Nudging so a point is adjustable without a pointer. */
  function handlePointKeys(event: KeyboardEvent, index: number) {
    if (disabled) return;
    const step = event.shiftKey ? 0.05 : 0.01;
    const point = points[index];
    let handled = true;
    if (event.key === 'ArrowUp') update(moveCurvePoint(points, index, point.input, point.output + step));
    else if (event.key === 'ArrowDown') update(moveCurvePoint(points, index, point.input, point.output - step));
    else if (event.key === 'ArrowLeft') update(moveCurvePoint(points, index, point.input - step, point.output));
    else if (event.key === 'ArrowRight') update(moveCurvePoint(points, index, point.input + step, point.output));
    else if (event.key === 'Delete' || event.key === 'Backspace') update(removeCurvePoint(points, index));
    else handled = false;
    if (handled) event.preventDefault();
  }

  /** Adds a point where the grid was clicked. */
  function handleGridClick(event: MouseEvent) {
    if (disabled || draggingIndex !== null) return;
    // A keyboard-activated click reports detail 0 and carries no useful
    // coordinates, so it falls back to splitting the widest gap.
    if (event.detail === 0) {
      addAtWidestGap();
      return;
    }
    const { input, output } = toCurveSpace(event);
    const next = addCurvePoint(points, input, output);
    if (next !== points) update(next);
  }

  /**
   * An SVG shape gets no native Enter-to-click, so the grid needs its own
   * keyboard path rather than relying on the browser synthesising one.
   */
  function handleGridKeys(event: KeyboardEvent) {
    if (disabled) return;
    if (event.key !== 'Enter' && event.key !== ' ') return;
    event.preventDefault();
    addAtWidestGap();
  }

  function addAtWidestGap() {
    let widest = 0;
    let input = 0.5;
    for (let index = 1; index < points.length; index += 1) {
      const gap = points[index].input - points[index - 1].input;
      if (gap > widest) {
        widest = gap;
        input = points[index - 1].input + gap / 2;
      }
    }
    const next = addCurvePoint(points, input, sampleCurve(points, input));
    if (next !== points) update(next);
  }

  function resetChannel() {
    update(identityCurve());
  }
</script>

<div class="curve-editor" class:disabled>
  <div class="channel-tabs" role="group" aria-label="Curve channel">
    {#each curveChannels as entry}
      <button
        type="button"
        class:active={channel === entry}
        aria-pressed={channel === entry}
        {disabled}
        on:click={() => (channel = entry)}
      >
        {channelLabels[entry]}
        {#if !isIdentityCurve(curves[entry])}<i aria-label="edited">•</i>{/if}
      </button>
    {/each}
  </div>

  <div
    bind:this={surface}
    class="curve-surface"
    role="group"
    aria-label={`${channelLabels[channel]} curve with ${points.length} points`}
  >
    <svg viewBox={`0 0 ${SIZE} ${SIZE}`} preserveAspectRatio="none">
      <!-- The grid itself is the add-a-point control. -->
      <rect
        x="0"
        y="0"
        width={SIZE}
        height={SIZE}
        class="grid-background"
        role="button"
        tabindex={disabled ? -1 : 0}
        aria-label="Add a curve point"
        aria-disabled={disabled}
        on:click={handleGridClick}
        on:keydown={handleGridKeys}
      />
      {#each [25, 50, 75] as line}
        <line x1={line} y1="0" x2={line} y2={SIZE} class="grid-line" />
        <line x1="0" y1={line} x2={SIZE} y2={line} class="grid-line" />
      {/each}
      <line x1="0" y1={SIZE} x2={SIZE} y2="0" class="identity-line" />
      <path d={pathData} class="curve-path" style={`stroke: ${channelColors[channel]}`} />
      {#each points as point, index (index)}
        <circle
          cx={point.input * SIZE}
          cy={SIZE - point.output * SIZE}
          r="3"
          class="curve-point"
          class:anchor={index === 0 || index === points.length - 1}
          style={`fill: ${channelColors[channel]}`}
          role="slider"
          tabindex={disabled ? -1 : 0}
          aria-label={`Curve point ${index + 1} of ${points.length}`}
          aria-valuemin="0"
          aria-valuemax="100"
          aria-valuenow={Math.round(point.output * 100)}
          aria-valuetext={`input ${Math.round(point.input * 100)} percent, output ${Math.round(point.output * 100)} percent`}
          on:pointerdown={(event) => handlePointerDown(event, index)}
          on:pointermove={handlePointerMove}
          on:pointerup={handlePointerUp}
          on:pointercancel={handlePointerUp}
          on:keydown={(event) => handlePointKeys(event, index)}
        />
      {/each}
    </svg>
  </div>

  <div class="curve-footer">
    <small>
      Click the grid to add · drag to move · Alt-click or Delete to remove ·
      {points.length}/{MAX_CURVE_POINTS} points
    </small>
    <button type="button" {disabled} on:click={resetChannel}>Reset {channelLabels[channel]}</button>
  </div>
</div>

<style>
  .curve-editor { display: grid; gap: 7px; }
  .curve-editor.disabled { opacity: .6; }
  .channel-tabs { display: flex; }
  .channel-tabs button { flex: 1; min-width: 0; padding: 5px 4px; border-radius: 0; font-size: .58rem; }
  .channel-tabs button:first-child { border-radius: 6px 0 0 6px; }
  .channel-tabs button:last-child { border-radius: 0 6px 6px 0; }
  .channel-tabs button.active { border-color: var(--accent); color: var(--accent); background: rgba(192,231,126,.1); }
  .channel-tabs i { font-style: normal; }
  .curve-surface { width: 100%; height: 190px; overflow: hidden; border: 1px solid var(--line); border-radius: 7px; touch-action: none; }
  .curve-surface svg { display: block; width: 100%; height: 100%; }
  .grid-background { fill: var(--surface-raised); cursor: crosshair; }
  .grid-background:focus-visible { outline: 2px solid var(--accent); outline-offset: -2px; }
  .grid-line { stroke: var(--line); stroke-width: .4; }
  .identity-line { stroke: var(--line); stroke-width: .5; stroke-dasharray: 2 2; }
  .curve-path { fill: none; stroke-width: 1.4; vector-effect: non-scaling-stroke; pointer-events: none; }
  .curve-point { stroke: var(--surface); stroke-width: 1; cursor: grab; }
  .curve-point.anchor { stroke-dasharray: 1 1; }
  .curve-point:focus-visible { outline: none; stroke: var(--ink); stroke-width: 1.6; }
  .curve-footer { display: flex; align-items: center; justify-content: space-between; gap: 8px; }
  .curve-footer small { color: var(--ink-faint); font-size: .55rem; line-height: 1.35; }
  .curve-footer button { padding: 5px 8px; font-size: .56rem; white-space: nowrap; }
</style>
