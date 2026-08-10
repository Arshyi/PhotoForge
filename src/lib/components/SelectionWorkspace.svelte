<script lang="ts">
  import MaskThumbnail from './MaskThumbnail.svelte';
  import type { MaskProgressView } from '../selections/progress';
  import type {
    ApplyScope,
    CompositionMode,
    MaskOperation,
    OverlayMode,
    SelectionState,
    SelectionTool
  } from '../selections/types';

  export let state: SelectionState;
  export let disabled = false;
  export let busy = false;
  export let canUndo = false;
  export let canRedo = false;
  export let progress: MaskProgressView | null = null;
  export let onstatechange: (state: SelectionState, coalesceKey?: string) => void;
  export let onoperation: (operation: MaskOperation) => void;
  export let onrefine: () => void = () => undefined;
  export let onnamedaction: (
    action: 'create' | 'rename' | 'duplicate' | 'delete' | 'visible' | 'locked' | 'up' | 'down' | 'load' | 'combine' | 'replace' | 'export' | 'export_png',
    id?: string,
    value?: string
  ) => void;
  export let onimport: (format: 'json' | 'png') => void;
  export let onundo: () => void;
  export let onredo: () => void;
  export let oncancel: () => void;

  let operationRadius = 8;
  let minimumIsland = 32;
  let newMaskName = '';

  const tools: Array<{ id: SelectionTool; icon: string; label: string; shortcut: string }> = [
    { id: 'rectangle', icon: '▭', label: 'Rectangle', shortcut: 'M' },
    { id: 'ellipse', icon: '◯', label: 'Ellipse', shortcut: 'M' },
    { id: 'freehand', icon: '〰', label: 'Freehand lasso', shortcut: 'L' },
    { id: 'polygon', icon: '⬠', label: 'Polygon lasso', shortcut: 'L' },
    { id: 'brush', icon: '●', label: 'Selection brush', shortcut: 'B' },
    { id: 'eraser', icon: '◌', label: 'Selection eraser', shortcut: 'E' },
    { id: 'magic_wand', icon: '✧', label: 'Magic wand', shortcut: 'W' },
    { id: 'color_range', icon: '◉', label: 'Color range', shortcut: 'C' }
  ];

  const modes: Array<{ id: CompositionMode; label: string }> = [
    { id: 'replace', label: 'Replace' },
    { id: 'add', label: 'Add' },
    { id: 'subtract', label: 'Subtract' },
    { id: 'intersect', label: 'Intersect' }
  ];

  function update(patch: Partial<SelectionState>, coalesceKey?: string) {
    onstatechange({ ...state, ...patch }, coalesceKey);
  }

  function updateSettings(patch: Partial<SelectionState['settings']>, coalesceKey?: string) {
    update({ settings: { ...state.settings, ...patch } }, coalesceKey);
  }

  function updateOverlay(patch: Partial<SelectionState['overlay']>, coalesceKey?: string) {
    update({ overlay: { ...state.overlay, ...patch } }, coalesceKey);
  }

  function setTool(tool: SelectionTool) {
    update({ tool });
  }

  function createMask() {
    onnamedaction('create', undefined, newMaskName);
    newMaskName = '';
  }

</script>

<section class="tool-section selection-workspace" aria-labelledby="selection-heading">
  <div class="selection-heading">
    <h2 id="selection-heading"><span>⬚</span> Selections &amp; Masks</h2>
    <div class="mini-actions">
      <button type="button" title="Undo selection" aria-label="Undo selection" disabled={!canUndo || busy} on:click={onundo}>↶</button>
      <button type="button" title="Redo selection" aria-label="Redo selection" disabled={!canRedo || busy} on:click={onredo}>↷</button>
      {#if busy}<button class="cancel" type="button" on:click={oncancel}>Cancel</button>{/if}
    </div>
  </div>

  <p class="selection-summary">
    {#if state.activeDiagnostics}
      <strong>{(state.activeDiagnostics.averageCoverage * 100).toFixed(1)}%</strong> average coverage · {state.activeDiagnostics.selectedPixels.toLocaleString()} pixels
    {:else}
      No active selection. Global adjustments remain unchanged.
    {/if}
  </p>
  {#if state.activeMask && state.applyScope === 'global'}
    <p class="selection-warning" role="note">An active selection exists, but new adjustments are set to Global.</p>
  {/if}
  {#if state.activeMask && !state.overlay.visible}
    <p class="selection-warning" role="note">The active selection is preserved, but its overlay is hidden.</p>
  {/if}

  {#if progress?.visible}
    <div
      class="mask-progress"
      role={progress.percent === null ? 'status' : 'progressbar'}
      aria-label={progress.label}
      aria-valuemin={progress.percent === null ? undefined : 0}
      aria-valuemax={progress.percent === null ? undefined : 100}
      aria-valuenow={progress.percent ?? undefined}
    >
      <div><strong>{progress.label}</strong><span>{progress.percent === null ? progress.phase : `${progress.percent}%`}</span></div>
      <i class:indeterminate={progress.percent === null} style={progress.percent === null ? undefined : `--mask-progress:${progress.percent}%`}></i>
    </div>
  {/if}

  <div class="tool-grid" aria-label="Selection tools">
    {#each tools as tool}
      <button
        type="button"
        class:active={state.tool === tool.id}
        title={`${tool.label} (${tool.shortcut})`}
        aria-label={tool.label}
        aria-pressed={state.tool === tool.id}
        disabled={disabled || busy}
        on:click={() => setTool(tool.id)}
      ><span>{tool.icon}</span><small>{tool.label.replace('Selection ', '')}</small></button>
    {/each}
  </div>

  <div class="composition" aria-label="Selection composition mode">
    {#each modes as mode}
      <button type="button" class:active={state.mode === mode.id} aria-pressed={state.mode === mode.id} disabled={busy} on:click={() => update({ mode: mode.id })}>{mode.label}</button>
    {/each}
  </div>
  <p class="modifier-note">Shift adds · Alt subtracts · Shift+Alt intersects</p>

  {#if state.tool === 'rectangle' || state.tool === 'ellipse'}
    <div class="option-row">
      <label><input type="checkbox" checked={state.settings.fixedAspect} on:change={(event) => updateSettings({ fixedAspect: event.currentTarget.checked })} /> Fixed 1:1</label>
      <label><input type="checkbox" checked={state.settings.fromCenter} on:change={(event) => updateSettings({ fromCenter: event.currentTarget.checked })} /> From center</label>
    </div>
  {:else if state.tool === 'brush' || state.tool === 'eraser'}
    <label class="range-row"><span>Diameter <output>{state.settings.brushDiameter}px</output></span><input aria-label="Selection brush diameter" type="range" min="1" max="512" step="1" value={state.settings.brushDiameter} on:input={(event) => updateSettings({ brushDiameter: Number(event.currentTarget.value) }, 'brush-diameter')} /></label>
    <label class="range-row"><span>Hardness <output>{Math.round(state.settings.brushHardness * 100)}%</output></span><input aria-label="Selection brush hardness" type="range" min="0" max="1" step="0.01" value={state.settings.brushHardness} on:input={(event) => updateSettings({ brushHardness: Number(event.currentTarget.value) }, 'brush-hardness')} /></label>
    <label class="range-row"><span>Opacity <output>{Math.round(state.settings.brushOpacity * 100)}%</output></span><input aria-label="Selection brush opacity" type="range" min="0.01" max="1" step="0.01" value={state.settings.brushOpacity} on:input={(event) => updateSettings({ brushOpacity: Number(event.currentTarget.value) }, 'brush-opacity')} /></label>
    <div class="pressure-controls">
      <label><input type="checkbox" checked={state.settings.pressureEnabled} disabled={busy} on:change={(event) => updateSettings({ pressureEnabled: event.currentTarget.checked })} /> Pen pressure</label>
      {#if state.settings.pressureEnabled}
        <div class="option-row">
          <label><input type="checkbox" checked={state.settings.pressureAffectsSize} disabled={busy} on:change={(event) => updateSettings({ pressureAffectsSize: event.currentTarget.checked })} /> Size</label>
          <label><input type="checkbox" checked={state.settings.pressureAffectsOpacity} disabled={busy} on:change={(event) => updateSettings({ pressureAffectsOpacity: event.currentTarget.checked })} /> Opacity</label>
        </div>
        {#if state.settings.pressureAffectsSize}
          <label class="range-row compact"><span>Minimum size <output>{Math.round(state.settings.pressureMinSizeFactor * 100)}%</output></span><input aria-label="Minimum pressure brush size" type="range" min="0.05" max="1" step="0.05" value={state.settings.pressureMinSizeFactor} disabled={busy} on:input={(event) => updateSettings({ pressureMinSizeFactor: Number(event.currentTarget.value) }, 'pressure-min-size')} /></label>
        {/if}
        {#if state.settings.pressureAffectsOpacity}
          <label class="range-row compact"><span>Minimum opacity <output>{Math.round(state.settings.pressureMinOpacityFactor * 100)}%</output></span><input aria-label="Minimum pressure brush opacity" type="range" min="0.01" max="1" step="0.05" value={state.settings.pressureMinOpacityFactor} disabled={busy} on:input={(event) => updateSettings({ pressureMinOpacityFactor: Number(event.currentTarget.value) }, 'pressure-min-opacity')} /></label>
        {/if}
        <p class="modifier-note">Resolved pen size and opacity are stored in the mask result. Mouse and touch remain uniform.</p>
      {/if}
    </div>
  {:else if state.tool === 'magic_wand'}
    <label class="range-row"><span>Tolerance <output>{Math.round(state.settings.wandTolerance * 100)}%</output></span><input aria-label="Magic wand tolerance" type="range" min="0" max="1" step="0.01" value={state.settings.wandTolerance} on:input={(event) => updateSettings({ wandTolerance: Number(event.currentTarget.value) }, 'wand-tolerance')} /></label>
    <div class="option-row">
      <label><input type="checkbox" checked={state.settings.wandContiguous} on:change={(event) => updateSettings({ wandContiguous: event.currentTarget.checked })} /> Contiguous</label>
      <label><input type="checkbox" checked={state.settings.wandAntiAlias} on:change={(event) => updateSettings({ wandAntiAlias: event.currentTarget.checked })} /> Anti-alias</label>
    </div>
    <select aria-label="Magic wand connectivity" value={state.settings.wandConnectivity} on:change={(event) => updateSettings({ wandConnectivity: event.currentTarget.value as 'four' | 'eight' })}><option value="four">4-neighbor</option><option value="eight">8-neighbor</option></select>
  {:else if state.tool === 'color_range'}
    <label class="range-row"><span>Tolerance <output>{Math.round(state.settings.colorTolerance * 100)}%</output></span><input aria-label="Color range tolerance" type="range" min="0.01" max="1" step="0.01" value={state.settings.colorTolerance} on:input={(event) => updateSettings({ colorTolerance: Number(event.currentTarget.value) }, 'color-tolerance')} /></label>
    <label class="range-row compact"><span>Hue sensitivity</span><input aria-label="Hue sensitivity" type="range" min="0" max="1" step="0.05" value={state.settings.hueSensitivity} on:input={(event) => updateSettings({ hueSensitivity: Number(event.currentTarget.value) }, 'hue-sensitivity')} /></label>
    <label class="range-row compact"><span>Luminance sensitivity</span><input aria-label="Luminance sensitivity" type="range" min="0" max="1" step="0.05" value={state.settings.luminanceSensitivity} on:input={(event) => updateSettings({ luminanceSensitivity: Number(event.currentTarget.value) }, 'luminance-sensitivity')} /></label>
    <label class="range-row compact"><span>Saturation sensitivity</span><input aria-label="Saturation sensitivity" type="range" min="0" max="1" step="0.05" value={state.settings.saturationSensitivity} on:input={(event) => updateSettings({ saturationSensitivity: Number(event.currentTarget.value) }, 'saturation-sensitivity')} /></label>
  {/if}

  {#if state.tool === 'magic_wand' || state.tool === 'color_range'}
    <label class="sample-merged"><input type="checkbox" checked={state.settings.sampleMerged} on:change={(event) => updateSettings({ sampleMerged: event.currentTarget.checked })} /> Sample current rendered image</label>
  {/if}

  <div class="scope-row">
    <label for="selection-scope">New adjustments</label>
    <select id="selection-scope" value={state.applyScope} disabled={busy} on:change={(event) => update({ applyScope: event.currentTarget.value as ApplyScope })}>
      <option value="global">Global</option>
      <option value="inside" disabled={!state.activeMask}>Inside selection</option>
      <option value="outside" disabled={!state.activeMask}>Outside selection</option>
    </select>
  </div>

  <div class="mask-operations" aria-label="Mask operations">
    <button type="button" disabled={!state.activeMask || busy} on:click={() => onoperation({ type: 'invert' })}>Invert</button>
    <button type="button" disabled={!state.activeMask || busy} on:click={() => onoperation({ type: 'fill_holes' })}>Fill holes</button>
    <button type="button" title={`Remove islands smaller than ${minimumIsland} pixels`} disabled={!state.activeMask || busy} on:click={() => onoperation({ type: 'remove_small_islands', minimum_pixels: minimumIsland })}>Clean &lt;32px</button>
    <button type="button" disabled={busy} on:click={() => onoperation({ type: 'select_all' })}>Select all</button>
    <button type="button" disabled={!state.activeMask || busy} on:click={() => onoperation({ type: 'deselect' })}>Deselect</button>
  </div>
  <div class="numeric-operations">
    <input aria-label="Mask operation radius in pixels" type="number" min="0" max="256" bind:value={operationRadius} />
    <button type="button" disabled={!state.activeMask || busy} on:click={() => onoperation({ type: 'feather', radius: operationRadius })}>Feather</button>
    <button type="button" disabled={!state.activeMask || busy} on:click={() => onoperation({ type: 'expand', radius: operationRadius })}>Expand</button>
    <button type="button" disabled={!state.activeMask || busy} on:click={() => onoperation({ type: 'contract', radius: operationRadius })}>Contract</button>
    <button type="button" disabled={!state.activeMask || busy} on:click={() => onoperation({ type: 'smooth', radius: operationRadius })}>Smooth</button>
    <button type="button" disabled={!state.activeMask || busy} on:click={() => onoperation({ type: 'border', width: operationRadius })}>Border</button>
  </div>
  <button class="refine-toggle" type="button" disabled={!state.activeMask || busy} on:click={onrefine}>Refine selection <span>↗</span></button>

  <div class="overlay-controls">
    <label><input type="checkbox" checked={state.overlay.visible} on:change={(event) => updateOverlay({ visible: event.currentTarget.checked })} /> Show overlay (Q)</label>
    <select aria-label="Selection overlay mode" value={state.overlay.mode} on:change={(event) => updateOverlay({ mode: event.currentTarget.value as OverlayMode })}>
      <option value="marching_ants">Marching ants</option><option value="color">Color overlay</option><option value="grayscale">Grayscale</option><option value="black">Black background</option><option value="white">White background</option><option value="mask_only">Mask only</option>
    </select>
    <label class="range-row compact"><span>Opacity</span><input aria-label="Overlay opacity" type="range" min="0.05" max="1" step="0.05" value={state.overlay.opacity} on:input={(event) => updateOverlay({ opacity: Number(event.currentTarget.value) }, 'overlay-opacity')} /></label>
    <label class="color-control">Color <input aria-label="Overlay color" type="color" value={state.overlay.color} on:input={(event) => updateOverlay({ color: event.currentTarget.value }, 'overlay-color')} /></label>
  </div>

  <div class="masks-heading"><div><strong>Named masks</strong><small>{state.namedMasks.length}</small></div><div><button type="button" title="Import PhotoForge mask" disabled={busy} on:click={() => onimport('json')}>JSON</button><button type="button" title="Import grayscale PNG" disabled={busy} on:click={() => onimport('png')}>PNG</button></div></div>
  <div class="create-mask"><input aria-label="New mask name" placeholder={`Mask ${state.namedMasks.length + 1}`} disabled={busy} bind:value={newMaskName} on:keydown={(event) => event.key === 'Enter' && createMask()} /><button type="button" disabled={!state.activeMask || busy} on:click={createMask}>Save active</button></div>
  {#if state.namedMasks.length}
    <ol class="mask-list">
      {#each state.namedMasks as mask, index (mask.id)}
        <li class:locked={mask.locked}>
          <span class="mask-thumbnail-cell"><MaskThumbnail mask={mask.mask} label={mask.name} /></span>
          <input aria-label={`Rename ${mask.name}`} value={mask.name} disabled={mask.locked || busy} on:change={(event) => onnamedaction('rename', mask.id, event.currentTarget.value)} />
          <div class="mask-item-actions">
            <button type="button" title="Load as active selection" disabled={busy} on:click={() => onnamedaction('load', mask.id)}>↙</button>
            <button type="button" title={`Combine using ${state.mode}`} disabled={!state.activeMask || busy} on:click={() => onnamedaction('combine', mask.id)}>⊕</button>
            <button type="button" title="Replace from active selection" disabled={mask.locked || !state.activeMask || busy} on:click={() => onnamedaction('replace', mask.id)}>↥</button>
            <button type="button" title={mask.visible ? 'Hide mask' : 'Show mask'} disabled={busy} on:click={() => onnamedaction('visible', mask.id)}>{mask.visible ? '◉' : '○'}</button>
            <button type="button" title={mask.locked ? 'Unlock mask' : 'Lock mask'} disabled={busy} on:click={() => onnamedaction('locked', mask.id)}>{mask.locked ? '▣' : '□'}</button>
            <button type="button" title="Move mask up" disabled={index === 0 || busy} on:click={() => onnamedaction('up', mask.id)}>↑</button>
            <button type="button" title="Move mask down" disabled={index === state.namedMasks.length - 1 || busy} on:click={() => onnamedaction('down', mask.id)}>↓</button>
            <button type="button" title="Duplicate mask" disabled={busy} on:click={() => onnamedaction('duplicate', mask.id)}>⧉</button>
            <button type="button" title="Export PhotoForge mask" disabled={busy} on:click={() => onnamedaction('export', mask.id)}>⇩</button>
            <button type="button" title="Export grayscale PNG" disabled={busy} on:click={() => onnamedaction('export_png', mask.id)}>▧</button>
            <button type="button" title="Delete mask" disabled={mask.locked || busy} on:click={() => onnamedaction('delete', mask.id)}>×</button>
          </div>
        </li>
      {/each}
    </ol>
  {:else}
    <p class="empty-masks">Save the active selection to reuse it by stable identifier.</p>
  {/if}
</section>

<style>
  .selection-workspace { display: grid; gap: 10px; }
  .selection-heading, .masks-heading, .scope-row, .range-row span, .color-control { display: flex; align-items: center; justify-content: space-between; gap: 8px; }
  .selection-heading h2 { margin: 0; }
  .mini-actions { display: flex; gap: 4px; }
  .mini-actions button, .masks-heading button { min-width: 28px; padding: 5px 7px; }
  .mini-actions .cancel { color: #f2a6a6; }
  .mask-progress { display: grid; gap: 5px; padding: 8px; border: 1px solid var(--line); border-radius: 7px; background: rgba(192,231,126,.04); }
  .mask-progress div { display: flex; justify-content: space-between; gap: 8px; color: var(--ink-soft); font-size: .62rem; }
  .mask-progress strong { color: var(--ink); }
  .mask-progress i { position: relative; height: 4px; overflow: hidden; border-radius: 99px; background: var(--surface-raised); }
  .mask-progress i::after { content: ''; position: absolute; inset: 0; width: var(--mask-progress, 0%); background: var(--accent); transition: width 100ms linear; }
  .mask-progress i.indeterminate::after { width: 35%; animation: mask-progress-slide 1s ease-in-out infinite; }
  .selection-summary, .modifier-note, .empty-masks { margin: 0; color: var(--ink-faint); font-size: .66rem; line-height: 1.45; }
  .selection-summary strong { color: var(--accent); }
  .selection-warning { margin: 0; padding: 7px 8px; border-left: 2px solid #e2b96f; color: #e8ca94; background: rgba(226,185,111,.06); font-size: .62rem; line-height: 1.4; }
  .tool-grid { display: grid; grid-template-columns: repeat(4, 1fr); gap: 5px; }
  .tool-grid button { min-width: 0; display: grid; place-items: center; gap: 3px; padding: 8px 3px 6px; }
  .tool-grid button span { font-size: 1rem; }
  .tool-grid button small { width: 100%; overflow: hidden; color: var(--ink-faint); font-size: .52rem; text-overflow: ellipsis; white-space: nowrap; }
  button.active { border-color: var(--accent); color: var(--accent); background: rgba(192,231,126,.1); }
  .composition { display: grid; grid-template-columns: repeat(4, 1fr); }
  .composition button { min-width: 0; padding: 6px 2px; border-radius: 0; font-size: .58rem; }
  .composition button:first-child { border-radius: 6px 0 0 6px; }
  .composition button:last-child { border-radius: 0 6px 6px 0; }
  .option-row { display: flex; flex-wrap: wrap; gap: 12px; }
  .option-row label, .sample-merged, .overlay-controls > label { color: var(--ink-soft); font-size: .65rem; }
  .pressure-controls { display: grid; gap: 7px; padding: 8px; border: 1px solid var(--line); border-radius: 7px; }
  .pressure-controls > label { color: var(--ink-soft); font-size: .65rem; }
  .range-row { display: grid; gap: 4px; color: var(--ink-soft); font-size: .65rem; }
  .range-row output { color: var(--ink); font-family: var(--font-mono); }
  .range-row input { width: 100%; }
  .range-row.compact { grid-template-columns: 1fr 1fr; align-items: center; }
  .scope-row { padding: 8px; border: 1px solid var(--line); border-radius: 7px; background: rgba(255,255,255,.02); }
  .scope-row label { color: var(--ink-soft); font-size: .64rem; font-weight: 700; }
  select, input[type='number'], .create-mask input, .mask-list input { min-width: 0; border: 1px solid var(--line); border-radius: 6px; color: var(--ink); background: var(--surface-raised); font: inherit; }
  select, input[type='number'] { padding: 6px; }
  .mask-operations { display: grid; grid-template-columns: repeat(3, 1fr); gap: 5px; }
  .mask-operations button, .numeric-operations button { min-width: 0; padding: 6px 3px; font-size: .57rem; }
  .numeric-operations { display: grid; grid-template-columns: 52px repeat(5, 1fr); gap: 4px; }
  .numeric-operations input { width: 100%; box-sizing: border-box; }
  .refine-toggle { display: flex; justify-content: space-between; width: 100%; }
  .overlay-controls { display: grid; gap: 7px; padding: 9px; border: 1px solid var(--line); border-radius: 7px; }
  .color-control input { width: 38px; height: 22px; padding: 0; border: 0; background: none; }
  .masks-heading { margin-top: 4px; }
  .masks-heading > div { display: flex; align-items: center; gap: 5px; }
  .masks-heading strong { font-size: .72rem; }
  .masks-heading small { display: grid; place-items: center; min-width: 18px; height: 18px; border-radius: 99px; color: var(--ink-faint); background: var(--surface-raised); }
  .create-mask { display: grid; grid-template-columns: 1fr auto; gap: 5px; }
  .create-mask input, .mask-list input { padding: 7px; }
  .mask-list { display: grid; gap: 6px; margin: 0; padding: 0; list-style: none; }
  .mask-list li { display: grid; grid-template-columns: 40px 1fr; gap: 5px; padding: 7px; border: 1px solid var(--line); border-radius: 7px; background: rgba(255,255,255,.015); }
  .mask-list li.locked { opacity: .78; }
  .mask-thumbnail-cell { grid-row: 1 / 3; display: grid; place-items: center; }
  .mask-item-actions { grid-column: 2; display: flex; flex-wrap: wrap; gap: 3px; }
  .mask-item-actions button { min-width: 24px; padding: 4px; }
  @keyframes mask-progress-slide { from { transform: translateX(-110%); } to { transform: translateX(300%); } }
  @media (max-width: 900px) { .tool-grid { grid-template-columns: repeat(4, minmax(48px, 1fr)); } }
</style>
