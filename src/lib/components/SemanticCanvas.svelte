<script lang="ts">
  import { tick } from 'svelte';
  import type { ShapeGeometry, TextContent } from '../layers/types';

  export let tool: 'none' | 'text' | 'rectangle' | 'ellipse' | 'line' | 'polygon' | 'star' = 'none';
  export let canvasWidth: number;
  export let canvasHeight: number;
  export let disabled = false;
  /** Recreating this component on document change discards all transient input. */
  export let edit: { id: string; content: TextContent } | null = null;
  export let ontext: (point: { x: number; y: number }) => void;
  export let onshape: (geometry: ShapeGeometry) => void;
  export let oncommittext: (text: string) => void;
  export let oncanceltext: () => void;

  let surface: HTMLButtonElement;
  let editor: HTMLTextAreaElement;
  let start: { x: number; y: number } | null = null;
  let end: { x: number; y: number } | null = null;
  let draft = '';
  let editId: string | null = null;
  let composing = false;
  let gestureTool = tool;
  let capturedPointer: number | null = null;
  $: if ((edit?.id ?? null) !== editId) {
    editId = edit?.id ?? null;
    draft = edit?.content.text ?? '';
    if (edit) void tick().then(() => editor?.focus());
  }
  $: if (disabled || tool !== gestureTool || tool === 'none' || edit) cancelGesture();
  $: bounds = start && end ? {
    x: Math.min(start.x, end.x), y: Math.min(start.y, end.y),
    width: Math.abs(end.x - start.x), height: Math.abs(end.y - start.y)
  } : null;

  function point(event: PointerEvent) {
    const rect = surface.getBoundingClientRect();
    if (rect.width <= 0 || rect.height <= 0 || !Number.isFinite(event.clientX) || !Number.isFinite(event.clientY)) return null;
    return { x: Math.max(0, Math.min(canvasWidth, (event.clientX - rect.left) * canvasWidth / rect.width)),
      y: Math.max(0, Math.min(canvasHeight, (event.clientY - rect.top) * canvasHeight / rect.height)) };
  }
  function down(event: PointerEvent) {
    if (disabled || event.button !== 0 || edit) return;
    const p = point(event);
    if (!p) return;
    event.preventDefault();
    surface.focus();
    if (tool === 'text') { ontext(p); return; }
    if (tool === 'none') return;
    gestureTool = tool;
    start = end = p;
    capturedPointer = event.pointerId;
    surface.setPointerCapture(event.pointerId);
  }
  function move(event: PointerEvent) {
    if (start) end = point(event) ?? end;
  }
  function up(event: PointerEvent) {
    if (!start) return;
    const a = start, b = point(event);
    start = end = null;
    if (surface.hasPointerCapture?.(event.pointerId)) surface.releasePointerCapture(event.pointerId);
    capturedPointer = null;
    if (!b || disabled || Math.hypot(b.x - a.x, b.y - a.y) < 1) return;
    const x = Math.min(a.x, b.x), y = Math.min(a.y, b.y);
    const width = Math.abs(b.x - a.x), height = Math.abs(b.y - a.y);
    const cx = x + width / 2, cy = y + height / 2;
    if (tool !== 'line' && (width < 1 || height < 1)) return;
    switch (tool) {
      case 'rectangle': onshape({ type: 'rectangle', x, y, width, height, cornerRadius: 0 }); break;
      case 'ellipse': onshape({ type: 'ellipse', cx, cy, rx: width / 2, ry: height / 2 }); break;
      case 'line': onshape({ type: 'line', x1: a.x, y1: a.y, x2: b.x, y2: b.y }); break;
      case 'polygon': onshape({ type: 'polygon', cx, cy, radius: Math.min(width, height) / 2, sides: 6, rotationDegrees: 0 }); break;
      case 'star': onshape({ type: 'star', cx, cy, outerRadius: Math.min(width, height) / 2, innerRadius: Math.min(width, height) / 4, points: 5, rotationDegrees: 0 }); break;
    }
  }
  function keys(event: KeyboardEvent) {
    // Native textarea owns selection, clipboard, bidi caret navigation and typing undo.
    // IME confirmation must never be mistaken for committing the document edit.
    if (event.isComposing || composing || disabled) return;
    if (event.key === 'Escape') { event.preventDefault(); event.stopPropagation(); oncanceltext(); }
    if (event.key === 'Enter' && (event.ctrlKey || event.metaKey)) {
      event.preventDefault(); event.stopPropagation(); oncommittext(draft);
    }
  }
  function cancelGesture() {
    if (capturedPointer !== null && surface?.hasPointerCapture?.(capturedPointer)) surface.releasePointerCapture(capturedPointer);
    capturedPointer = null;
    start = end = null;
  }
</script>

{#if edit}
  <div class="text-editor" role="group" aria-label="Canvas text editor">
    <label>Editable text <textarea bind:this={editor} bind:value={draft} aria-label="Canvas text content" dir="auto" {disabled}
      on:keydown={keys} on:compositionstart={() => composing = true} on:compositionend={() => composing = false}></textarea></label>
    <p>Ctrl+Enter commits one undo step. Escape cancels. The rendered font and layout update after committing.</p>
    <button disabled={disabled || composing} on:click={() => oncommittext(draft)}>Commit text</button>
    <button on:click={oncanceltext}>Cancel text</button>
  </div>
{:else if tool !== 'none'}
  <button bind:this={surface} type="button" class="semantic-canvas" {disabled} aria-label={`${tool} layer canvas`}
    on:pointerdown={down} on:pointermove={move} on:pointerup={up} on:pointercancel={cancelGesture}
    on:keydown={(event) => { if (event.key === 'Escape') { event.stopPropagation(); cancelGesture(); } }}>
    <svg viewBox={`0 0 ${canvasWidth} ${canvasHeight}`} preserveAspectRatio="none" aria-hidden="true">
      {#if bounds && start && end}
        {#if tool === 'line'}<line x1={start.x} y1={start.y} x2={end.x} y2={end.y} />
        {:else if tool === 'ellipse'}<ellipse cx={bounds.x + bounds.width / 2} cy={bounds.y + bounds.height / 2} rx={bounds.width / 2} ry={bounds.height / 2} />
        {:else}<rect x={bounds.x} y={bounds.y} width={bounds.width} height={bounds.height} />{/if}
      {/if}
    </svg>
  </button>
{/if}

<style>
  .semantic-canvas { position: absolute; inset: 0; width: 100%; height: 100%; padding: 0; border: 0; background: transparent; cursor: crosshair; touch-action: none; }
  svg { width: 100%; height: 100%; fill: #8ab84a33; stroke: #7ac542; stroke-width: 1.5; }
  svg :global(*) { vector-effect: non-scaling-stroke; }
  .text-editor { position: absolute; z-index: 20; top: 8px; left: 8px; width: min(360px, 90%); padding: 12px; background: var(--surface, #222); border: 1px solid var(--accent, #8ab84a); box-shadow: 0 8px 30px #0008; }
  textarea { display: block; width: 100%; min-height: 100px; resize: vertical; font: 16px sans-serif; }
  p { font-size: 12px; }
</style>
