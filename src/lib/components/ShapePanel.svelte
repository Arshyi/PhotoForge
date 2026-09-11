<script lang="ts">
  import type { ShapeContent, ShapeGeometry } from '../layers/types';
  export let content: ShapeContent;
  export let disabled = false;
  export let onchange: (content: ShapeContent) => void;
  export let onrasterize: () => void;
  $: geometry = content.geometry;
  $: fields = Object.entries(geometry).filter(([key, value]) => key !== 'type' && typeof value === 'number');
  function parameter(key: string, value: string) {
    if (value.trim() === '' || !Number.isFinite(Number(value))) return;
    onchange({ ...content, geometry: { ...geometry, [key]: Number(value) } as ShapeGeometry });
  }
</script>

<fieldset class="shape-panel" {disabled} aria-label="Shape properties">
  <legend>Editable {geometry.type}</legend>
  {#each fields as [key, value]}
    <label>{key}<input type="number" aria-label={`Shape ${key}`} value={Number(value)} step="any" on:change={(event) => parameter(key, event.currentTarget.value)} /></label>
  {/each}
  <label><input type="checkbox" checked={Boolean(content.fill)} on:change={(event) => onchange({ ...content, fill: event.currentTarget.checked ? { red: 0.2, green: 0.4, blue: 0.9, alpha: 1 } : null })} /> Fill</label>
  {#if content.fill}
    {#each ['red', 'green', 'blue', 'alpha'] as channel}
      <label>Fill {channel}<input aria-label={`Shape fill ${channel}`} type="number" min="0" max="1" step="0.01" value={content.fill[channel as keyof typeof content.fill]}
        on:change={(event) => onchange({ ...content, fill: { ...content.fill!, [channel]: Number(event.currentTarget.value) } })} /></label>
    {/each}
  {/if}
  <label><input type="checkbox" checked={Boolean(content.stroke)} on:change={(event) => onchange({ ...content, stroke: event.currentTarget.checked ? { red: 0, green: 0, blue: 0, alpha: 1 } : null,
    strokeStyle: content.strokeStyle ?? { width: 2, cap: 'round', join: 'round', miterLimit: 4 } })} /> Stroke</label>
  {#if content.stroke && content.strokeStyle}
    <label>Stroke width<input aria-label="Shape stroke width" type="number" min="0.01" max="4096" step="0.5" value={content.strokeStyle.width}
      on:change={(event) => onchange({ ...content, strokeStyle: { ...content.strokeStyle!, width: Number(event.currentTarget.value) } })} /></label>
    <label>Cap<select aria-label="Shape stroke cap" value={content.strokeStyle.cap} on:change={(event) => onchange({ ...content, strokeStyle: { ...content.strokeStyle!, cap: event.currentTarget.value as 'round' | 'butt' | 'square' } })}>
      <option>butt</option><option>round</option><option>square</option></select></label>
    <label>Join<select aria-label="Shape stroke join" value={content.strokeStyle.join} on:change={(event) => onchange({ ...content, strokeStyle: { ...content.strokeStyle!, join: event.currentTarget.value as 'round' | 'bevel' | 'miter' } })}>
      <option>round</option><option>bevel</option><option>miter</option></select></label>
  {/if}
  <p>Colors are linear working-space values. Geometry stays editable until explicitly rasterized.</p>
  <button on:click={onrasterize}>Rasterize shape to pixels</button>
</fieldset>

<style>
  .shape-panel { margin: 8px; display: grid; gap: 6px; }
  label { display: flex; align-items: center; justify-content: space-between; gap: 8px; font-size: 12px; }
  input[type=number] { width: 100px; }
  p { font-size: 11px; }
</style>
