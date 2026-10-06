<script lang="ts">
  import { formatBytes } from '../utils/format';
  import type { Rect, SourceAdmission, SourcePreview } from '../source/types';
  import {
    applyAspect,
    aspectPresets,
    aspectRatio,
    centerRect,
    clampRect,
    initialRegion,
    largestRegion,
    megapixels,
    moveRect,
    regionStatus,
    resizeRect,
    type AspectPreset,
    type Handle
  } from '../source/region';

  export let admission: SourceAdmission;
  /** `loading` while the bounded preview decodes; `unavailable` if it cannot be made. */
  export let previewState: 'loading' | 'ready' | 'unavailable' = 'loading';
  export let preview: SourcePreview | null = null;
  export let previewMessage = '';
  export let initial: Rect | null = null;
  export let onconfirm: (rect: Rect) => void = () => {};
  export let oncancel: () => void = () => {};
  export let onback: () => void = () => {};

  const sourceWidth = admission.width;
  const sourceHeight = admission.height;
  const handles: { id: Handle; label: string }[] = [
    { id: 'nw', label: 'top-left corner' },
    { id: 'n', label: 'top edge' },
    { id: 'ne', label: 'top-right corner' },
    { id: 'e', label: 'right edge' },
    { id: 'se', label: 'bottom-right corner' },
    { id: 's', label: 'bottom edge' },
    { id: 'sw', label: 'bottom-left corner' },
    { id: 'w', label: 'left edge' }
  ];

  let rect: Rect = initial
    ? clampRect(initial, sourceWidth, sourceHeight)
    : initialRegion(admission.report, sourceWidth, sourceHeight);
  let aspect: AspectPreset = 'free';
  let stage: HTMLDivElement;
  /** What assistive technology is told, updated when a change settles, not on every pixel of a drag. */
  let announcement = '';

  $: ratio = aspectRatio(aspect, sourceWidth, sourceHeight);
  $: status = regionStatus(rect, admission.report);
  $: style =
    `left:${(rect.x / sourceWidth) * 100}%;top:${(rect.y / sourceHeight) * 100}%;` +
    `width:${(rect.width / sourceWidth) * 100}%;height:${(rect.height / sourceHeight) * 100}%;`;

  function describe(): string {
    // Priced afresh rather than read from the reactive `status`: this runs
    // immediately after `rect` is assigned, before Svelte has recomputed
    // anything derived from it, so `status` would describe the rectangle the
    // user has just left.
    const current = regionStatus(rect, admission.report);
    const memory = current.peakBytes === null ? '' : `, about ${formatBytes(current.peakBytes)} of memory`;
    return (
      `Region ${rect.width} by ${rect.height} pixels at ${rect.x}, ${rect.y}. ` +
      `${megapixels(rect).toFixed(1)} megapixels${memory}. ` +
      (current.supported ? 'This region can be opened.' : current.reason)
    );
  }

  function settle() {
    announcement = describe();
  }

  /* ---------- pointer dragging ---------- */
  type Drag = { mode: 'move' | Handle; startX: number; startY: number; origin: Rect };
  let drag: Drag | null = null;

  function toSource(clientDelta: number): number {
    const box = stage.getBoundingClientRect();
    return box.width > 0 ? (clientDelta * sourceWidth) / box.width : 0;
  }

  function begin(event: PointerEvent, mode: 'move' | Handle) {
    if (event.button !== 0) return;
    drag = { mode, startX: event.clientX, startY: event.clientY, origin: rect };
    (event.currentTarget as HTMLElement).setPointerCapture?.(event.pointerId);
    event.preventDefault();
    event.stopPropagation();
  }

  function moveDrag(event: PointerEvent) {
    if (!drag) return;
    const dx = Math.round(toSource(event.clientX - drag.startX));
    const dy = Math.round(toSource(event.clientY - drag.startY));
    rect =
      drag.mode === 'move'
        ? moveRect(drag.origin, dx, dy, sourceWidth, sourceHeight)
        : resizeRect(drag.origin, drag.mode, dx, dy, sourceWidth, sourceHeight, ratio);
  }

  function endDrag(event: PointerEvent) {
    if (!drag) return;
    drag = null;
    (event.currentTarget as HTMLElement).releasePointerCapture?.(event.pointerId);
    settle();
  }

  function cancelDrag() {
    if (drag) rect = drag.origin;
    drag = null;
  }

  /* ---------- keyboard ---------- */
  /** One pixel; ten with Ctrl; a hundred with Alt. */
  function stepOf(event: KeyboardEvent): number {
    return event.altKey ? 100 : event.ctrlKey || event.metaKey ? 10 : 1;
  }

  const arrows: Record<string, [number, number]> = {
    ArrowLeft: [-1, 0],
    ArrowRight: [1, 0],
    ArrowUp: [0, -1],
    ArrowDown: [0, 1]
  };

  function bodyKeys(event: KeyboardEvent) {
    const direction = arrows[event.key];
    if (!direction) return;
    event.preventDefault();
    const step = stepOf(event);
    if (event.shiftKey) {
      // Shift resizes from the bottom-right, the reverse of how the arrows move.
      rect = resizeRect(rect, 'se', direction[0] * step, direction[1] * step, sourceWidth, sourceHeight, ratio);
    } else {
      rect = moveRect(rect, direction[0] * step, direction[1] * step, sourceWidth, sourceHeight);
    }
    settle();
  }

  function handleKeys(event: KeyboardEvent, handle: Handle) {
    const direction = arrows[event.key];
    if (!direction) return;
    event.preventDefault();
    event.stopPropagation();
    const step = stepOf(event);
    rect = resizeRect(rect, handle, direction[0] * step, direction[1] * step, sourceWidth, sourceHeight, ratio);
    settle();
  }

  /* ---------- numeric entry ---------- */
  function commit(field: 'x' | 'y' | 'width' | 'height', input: HTMLInputElement) {
    const value = Number.parseInt(input.value, 10);
    if (Number.isFinite(value)) {
      let next: Rect = { ...rect, [field]: value };
      if (ratio !== null) {
        if (field === 'width') next = { ...next, height: Math.max(1, Math.round(value / ratio)) };
        if (field === 'height') next = { ...next, width: Math.max(1, Math.round(value * ratio)) };
      }
      rect = clampRect(next, sourceWidth, sourceHeight);
    }
    // Whether the entry was taken, clamped or refused, the box shows the real
    // value afterwards, not what was typed.
    input.value = String(rect[field]);
    settle();
  }

  function chooseAspect(event: Event) {
    aspect = (event.currentTarget as HTMLSelectElement).value as AspectPreset;
    rect = applyAspect(rect, aspectRatio(aspect, sourceWidth, sourceHeight), sourceWidth, sourceHeight);
    settle();
  }

  function center() {
    rect = centerRect(rect, sourceWidth, sourceHeight);
    settle();
  }

  function maximise() {
    const largest = largestRegion(admission.report, sourceWidth, sourceHeight, ratio, rect);
    if (largest) rect = largest;
    settle();
  }

  const mp = (w: number, h: number) => ((w * h) / 1_000_000).toFixed(1);
</script>

<section class="region-selector" aria-label="Choose a region to open">
  <div class="stage-column">
    <div
      class="stage"
      bind:this={stage}
      style={`aspect-ratio:${sourceWidth} / ${sourceHeight}`}
      data-testid="region-stage"
    >
      {#if previewState === 'ready' && preview}
        <img src={preview.dataUrl} alt="Reduced preview of the whole source" draggable="false" />
      {:else}
        <div class="stage-note" role="status">
          {#if previewState === 'loading'}
            Preparing a bounded preview of the whole image…
          {:else}
            {previewMessage || 'No preview could be made within the memory budget.'}
            Enter the region numerically.
          {/if}
        </div>
      {/if}

      <!-- A positioned frame (not itself interactive) holding a real button for the
           region and real buttons for the handles. Handles are siblings of the
           region, not children, because buttons cannot nest. -->
      <div class="region" class:unsupported={!status.supported} {style} data-testid="region-frame">
        <button
          type="button"
          class="body"
          aria-label="Working region. Arrow keys move it, Shift with arrow keys resizes it, Ctrl moves ten pixels, Alt a hundred."
          data-testid="region-box"
          on:pointerdown={(event) => begin(event, 'move')}
          on:pointermove={moveDrag}
          on:pointerup={endDrag}
          on:pointercancel={cancelDrag}
          on:keydown={bodyKeys}
        ></button>
        {#each handles as handle (handle.id)}
          <button
            type="button"
            class={`handle ${handle.id}`}
            aria-label={`Resize ${handle.label}`}
            data-handle={handle.id}
            on:pointerdown={(event) => begin(event, handle.id)}
            on:pointermove={moveDrag}
            on:pointerup={endDrag}
            on:pointercancel={cancelDrag}
            on:keydown={(event) => handleKeys(event, handle.id)}
          ></button>
        {/each}
      </div>
    </div>
  </div>

  <div class="panel">
    <dl class="facts">
      <dt>Original</dt>
      <dd>
        {sourceWidth.toLocaleString('en-US')} × {sourceHeight.toLocaleString('en-US')} px · {mp(sourceWidth, sourceHeight)} MP
      </dd>
      <dt>Selected</dt>
      <dd data-testid="selected-summary">
        {rect.width.toLocaleString('en-US')} × {rect.height.toLocaleString('en-US')} px · {megapixels(rect).toFixed(1)} MP
      </dd>
      <dt>Estimated working memory</dt>
      <dd data-testid="estimate">{status.peakBytes === null ? 'Unknown' : formatBytes(status.peakBytes)}</dd>
    </dl>

    <p class="verdict" class:bad={!status.supported} data-testid="admission">
      <strong>Admission:</strong>
      {status.supported ? 'Supported' : 'Too large'}
      {#if !status.supported}<span class="reason">{status.reason}</span>{/if}
    </p>

    <div class="fields">
      <label>X <input type="number" min="0" max={sourceWidth - 1} value={rect.x} on:change={(e) => commit('x', e.currentTarget)} /></label>
      <label>Y <input type="number" min="0" max={sourceHeight - 1} value={rect.y} on:change={(e) => commit('y', e.currentTarget)} /></label>
      <label>Width <input type="number" min="1" max={sourceWidth} value={rect.width} on:change={(e) => commit('width', e.currentTarget)} /></label>
      <label>Height <input type="number" min="1" max={sourceHeight} value={rect.height} on:change={(e) => commit('height', e.currentTarget)} /></label>
    </div>

    <label class="aspect">
      Aspect ratio
      <select value={aspect} on:change={chooseAspect}>
        {#each aspectPresets as preset (preset.id)}
          <option value={preset.id}>{preset.label}</option>
        {/each}
      </select>
    </label>

    <div class="quick">
      <button type="button" on:click={center}>Center</button>
      <button type="button" on:click={maximise} title="The largest region that fits the memory budget">Maximize</button>
    </div>

    <p class="sr-only" role="status" aria-live="polite" aria-atomic="true" data-testid="announcement">{announcement}</p>

    <div class="actions">
      <button type="button" on:click={onback}>Back</button>
      <button type="button" on:click={oncancel}>Cancel</button>
      <button type="button" class="primary" disabled={!status.supported} on:click={() => regionStatus(rect, admission.report).supported && onconfirm(rect)}>
        Open region
      </button>
    </div>
  </div>
</section>

<style>
  .region-selector { display: grid; grid-template-columns: minmax(0, 1fr) 260px; gap: 16px; padding: 16px 21px 21px; }
  .stage-column { min-width: 0; }
  .stage { position: relative; width: 100%; max-height: 62vh; overflow: hidden; border: 1px solid var(--line-strong); border-radius: 8px; background: repeating-conic-gradient(#222 0% 25%, #2b2b2b 0% 50%) 50% / 16px 16px; touch-action: none; user-select: none; }
  .stage img { position: absolute; inset: 0; width: 100%; height: 100%; object-fit: fill; pointer-events: none; }
  .stage-note { position: absolute; inset: 0; display: grid; place-items: center; padding: 16px; color: var(--ink-soft); font-size: 0.68rem; text-align: center; }
  .region { position: absolute; box-sizing: border-box; border: 2px solid var(--accent); box-shadow: 0 0 0 9999px rgba(0, 0, 0, 0.55); pointer-events: none; }
  .region.unsupported { border-color: #e0795a; }
  .body { position: absolute; inset: 0; width: 100%; height: 100%; padding: 0; border: 0; background: transparent; cursor: move; pointer-events: auto; }
  .body:focus-visible { outline: 2px solid #fff; outline-offset: 2px; }
  .handle { position: absolute; pointer-events: auto; width: 14px; height: 14px; margin: -7px 0 0 -7px; padding: 0; border: 2px solid #152012; border-radius: 3px; background: var(--accent); }
  .handle:focus-visible { outline: 2px solid #fff; outline-offset: 1px; }
  .handle.nw { left: 0; top: 0; cursor: nwse-resize; }
  .handle.n { left: 50%; top: 0; cursor: ns-resize; }
  .handle.ne { left: 100%; top: 0; cursor: nesw-resize; }
  .handle.e { left: 100%; top: 50%; cursor: ew-resize; }
  .handle.se { left: 100%; top: 100%; cursor: nwse-resize; }
  .handle.s { left: 50%; top: 100%; cursor: ns-resize; }
  .handle.sw { left: 0; top: 100%; cursor: nesw-resize; }
  .handle.w { left: 0; top: 50%; cursor: ew-resize; }
  .panel { display: grid; align-content: start; gap: 12px; font-size: 0.66rem; }
  .facts { display: grid; grid-template-columns: auto 1fr; gap: 4px 10px; margin: 0; }
  .facts dt { color: var(--ink-faint); }
  .facts dd { margin: 0; color: var(--ink); font-variant-numeric: tabular-nums; }
  .verdict { margin: 0; padding: 8px 10px; border-radius: 7px; background: var(--accent-dim); color: var(--ink); }
  .verdict.bad { background: rgba(224, 121, 90, 0.16); }
  .reason { display: block; margin-top: 3px; color: var(--ink-soft); }
  .fields { display: grid; grid-template-columns: 1fr 1fr; gap: 8px; }
  .fields label, .aspect { display: grid; gap: 4px; color: var(--ink-soft); font-weight: 700; }
  .fields input, .aspect select { min-width: 0; padding: 7px 8px; border: 1px solid var(--line-strong); border-radius: 6px; color: var(--ink); background: var(--surface-soft); font: inherit; }
  .quick, .actions { display: flex; gap: 6px; }
  .actions { justify-content: flex-end; flex-wrap: wrap; }
  .quick button, .actions button { padding: 7px 11px; border: 1px solid var(--line-strong); border-radius: 7px; color: var(--ink-soft); background: var(--surface-raised); font-size: 0.62rem; font-weight: 700; cursor: pointer; }
  .actions button.primary { color: #152012; border-color: var(--accent); background: var(--accent); }
  .actions button:disabled { opacity: 0.45; cursor: not-allowed; }
  .sr-only { position: absolute; width: 1px; height: 1px; margin: -1px; padding: 0; overflow: hidden; clip: rect(0, 0, 0, 0); white-space: nowrap; border: 0; }
  @media (max-width: 700px) { .region-selector { grid-template-columns: 1fr; } }
</style>
