<script lang="ts">
  /**
   * Numeric controls for the active layer's transform.
   *
   * Every field is in document pixels or degrees; the viewport's zoom is never
   * part of what is stored. Values are pushed through `withMetric`, which
   * clamps and repairs them, so a typed NaN or a value far outside the
   * renderer's range can never reach the document.
   */
  import {
    documentBounds,
    intersectsCanvas,
    isIdentityTransform,
    transformMetrics,
    withMetric,
    type TransformMetric
  } from '../layers/transformTool';
  import { interpolationModes, type LayerInterpolation, type LayerTransform } from '../layers/types';

  export let transform: LayerTransform;
  export let layerWidth: number;
  export let layerHeight: number;
  export let canvasWidth: number;
  export let canvasHeight: number;
  export let layerName = '';
  export let active = false;
  export let disabled = false;
  export let busy = false;
  export let aspectLocked = true;

  export let onchange: (transform: LayerTransform, label: string) => void;
  export let onaspectchange: (locked: boolean) => void;
  export let ontoggle: () => void;
  export let onreset: () => void;
  export let onrasterize: () => void;
  export let onflip: (axis: 'horizontal' | 'vertical') => void;

  $: metrics = transformMetrics(transform, layerWidth, layerHeight);
  $: bounds = documentBounds(transform, layerWidth, layerHeight);
  $: onCanvas = intersectsCanvas(transform, layerWidth, layerHeight, canvasWidth, canvasHeight);
  $: transformed = !isIdentityTransform(transform);

  function commit(metric: TransformMetric, raw: string, label: string) {
    const value = Number(raw);
    if (raw.trim() === '' || !Number.isFinite(value)) return;
    onchange(withMetric(transform, layerWidth, layerHeight, metric, value, aspectLocked), label);
  }

  function field(event: Event): string {
    return (event.currentTarget as HTMLInputElement).value;
  }

  function setInterpolation(mode: LayerInterpolation) {
    onchange({ ...transform, interpolation: mode }, 'Layer sampling');
  }

  function round(value: number, places = 2): number {
    const factor = 10 ** places;
    return Math.round(value * factor) / factor;
  }
</script>

<section class="transform-panel" aria-labelledby="transform-heading">
  <div class="transform-heading">
    <h3 id="transform-heading">Transform</h3>
    <button
      type="button"
      class="mode-toggle"
      class:active
      {disabled}
      aria-pressed={active}
      title="Show the transform box on the canvas (Ctrl+T)"
      on:click={ontoggle}>{active ? 'Transforming' : 'Transform'}</button
    >
  </div>

  {#if !layerWidth || !layerHeight}
    <p class="transform-note">Select a pixel layer to move, scale, or rotate it.</p>
  {:else}
    <p class="transform-note">
      {#if transformed}
        <strong>{layerName || 'This layer'}</strong> is transformed. Rasterize to bake it into the
        pixels, or reset to put it back.
      {:else}
        <strong>{layerName || 'This layer'}</strong> sits on its own grid. Transforms stay editable
        until you rasterize.
      {/if}
    </p>

    <div class="transform-grid">
      <label>
        <span>Center X</span>
        <input
          type="number"
          step="1"
          {disabled}
          value={round(metrics.centerX)}
          on:change={(event) => commit('centerX', field(event), 'Move layer')}
        />
      </label>
      <label>
        <span>Center Y</span>
        <input
          type="number"
          step="1"
          {disabled}
          value={round(metrics.centerY)}
          on:change={(event) => commit('centerY', field(event), 'Move layer')}
        />
      </label>
      <label>
        <span>Width</span>
        <input
          type="number"
          step="1"
          min="0"
          {disabled}
          value={round(metrics.width)}
          on:change={(event) => commit('width', field(event), 'Scale layer')}
        />
      </label>
      <label>
        <span>Height</span>
        <input
          type="number"
          step="1"
          min="0"
          {disabled}
          value={round(metrics.height)}
          on:change={(event) => commit('height', field(event), 'Scale layer')}
        />
      </label>
      <label>
        <span>Scale X</span>
        <input
          type="number"
          step="0.01"
          {disabled}
          value={round(metrics.scaleX, 4)}
          on:change={(event) => commit('scaleX', field(event), 'Scale layer')}
        />
      </label>
      <label>
        <span>Scale Y</span>
        <input
          type="number"
          step="0.01"
          {disabled}
          value={round(metrics.scaleY, 4)}
          on:change={(event) => commit('scaleY', field(event), 'Scale layer')}
        />
      </label>
      <label class="angle">
        <span>Rotation</span>
        <input
          type="number"
          step="0.5"
          min="-360"
          max="360"
          {disabled}
          value={round(metrics.rotationDegrees)}
          on:change={(event) => commit('rotationDegrees', field(event), 'Rotate layer')}
        />
      </label>
      <label class="lock">
        <input
          type="checkbox"
          {disabled}
          checked={aspectLocked}
          on:change={(event) => onaspectchange((event.currentTarget as HTMLInputElement).checked)}
        />
        <span>Keep proportions</span>
      </label>
    </div>

    <fieldset class="sampling">
      <legend>Sampling</legend>
      {#each interpolationModes as mode (mode.id)}
        <button
          type="button"
          class:active={transform.interpolation === mode.id}
          {disabled}
          title={mode.hint}
          aria-pressed={transform.interpolation === mode.id}
          on:click={() => setInterpolation(mode.id)}>{mode.label}</button
        >
      {/each}
    </fieldset>

    <div class="transform-actions">
      <button type="button" {disabled} title="Mirror this layer left to right" on:click={() => onflip('horizontal')}
        >Flip H</button
      >
      <button type="button" {disabled} title="Mirror this layer top to bottom" on:click={() => onflip('vertical')}
        >Flip V</button
      >
      <button type="button" disabled={disabled || !transformed} on:click={onreset}>Reset</button>
      <button type="button" disabled={disabled || busy} title="Bake the transform into the layer's pixels" on:click={onrasterize}
        >Rasterize</button
      >
    </div>

    <p class="bounds-readout">
      Bounds {Math.round(bounds.left)}, {Math.round(bounds.top)} to {Math.round(bounds.right)},
      {Math.round(bounds.bottom)} in a {canvasWidth} by {canvasHeight} canvas.
      {#if !onCanvas}<b> Entirely off canvas.</b>{/if}
    </p>
  {/if}
</section>

<style>
  .transform-panel { display: grid; gap: 8px; }
  .transform-heading { display: flex; align-items: center; justify-content: space-between; gap: 8px; }
  .transform-heading h3 { margin: 0; font-size: .72rem; }
  .mode-toggle.active { border-color: var(--accent); color: var(--accent); background: rgba(192,231,126,.1); }
  .transform-note { margin: 0; color: var(--ink-faint); font-size: .62rem; line-height: 1.45; }
  .transform-note strong { color: var(--ink); }
  .transform-grid { display: grid; grid-template-columns: 1fr 1fr; gap: 6px; }
  .transform-grid label { display: grid; gap: 3px; font-size: .6rem; color: var(--ink-soft); }
  .transform-grid label.lock { grid-column: span 2; display: flex; align-items: center; gap: 6px; }
  .transform-grid input[type='number'] { width: 100%; }
  .sampling { display: flex; align-items: center; gap: 4px; margin: 0; padding: 0; border: 0; }
  .sampling legend { float: left; margin-right: 6px; padding: 0; color: var(--ink-soft); font-size: .6rem; font-weight: 700; }
  .sampling button { flex: 1; padding: 4px 6px; font-size: .58rem; }
  .sampling button.active { border-color: var(--accent); color: var(--accent); background: rgba(192,231,126,.1); }
  .transform-actions { display: flex; flex-wrap: wrap; gap: 4px; }
  .transform-actions button { flex: 1 1 auto; min-width: 62px; padding: 5px 7px; font-size: .58rem; }
  .bounds-readout { margin: 0; color: var(--ink-faint); font-size: .58rem; line-height: 1.4; }
  .bounds-readout b { color: #e8ca94; }
</style>
