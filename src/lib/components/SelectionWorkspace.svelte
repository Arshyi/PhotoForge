<script lang="ts">
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
  export let onstatechange: (state: SelectionState, coalesceKey?: string) => void;
  export let onoperation: (operation: MaskOperation) => void;
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
  let refineOpen = false;
  let refineSmooth = 3;
  let refineFeather = 2;
  let refineContrast = 0;
  let refineShift = 0;

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

  function applyRefine() {
    onoperation({
      type: 'refine',
      smooth: refineSmooth,
      feather: refineFeather,
      contrast: refineContrast,
      shift_edge: refineShift
    });
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
      <button type="button" class:active={state.mode === mode.id} aria-pressed={state.mode === mode.id} on:click={() => update({ mode: mode.id })}>{mode.label}</button>
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
    <select id="selection-scope" value={state.applyScope} on:change={(event) => update({ applyScope: event.currentTarget.value as ApplyScope })}>
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
  <button class="refine-toggle" type="button" disabled={!state.activeMask} aria-expanded={refineOpen} on:click={() => (refineOpen = !refineOpen)}>Refine selection <span>{refineOpen ? '−' : '+'}</span></button>
  {#if refineOpen}
    <div class="refine-panel">
      <label>Smooth <input type="number" min="0" max="128" bind:value={refineSmooth} /></label>
      <label>Feather <input type="number" min="0" max="256" bind:value={refineFeather} /></label>
      <label>Contrast <input type="number" min="-1" max="1" step="0.05" bind:value={refineContrast} /></label>
      <label>Shift edge <input type="number" min="-256" max="256" bind:value={refineShift} /></label>
      <button type="button" disabled={busy} on:click={applyRefine}>Apply refinement</button>
      <p>Classical smoothing, morphology, and local image gradients alter only mask coverage.</p>
    </div>
  {/if}

  <div class="overlay-controls">
    <label><input type="checkbox" checked={state.overlay.visible} on:change={(event) => updateOverlay({ visible: event.currentTarget.checked })} /> Show overlay (Q)</label>
    <select aria-label="Selection overlay mode" value={state.overlay.mode} on:change={(event) => updateOverlay({ mode: event.currentTarget.value as OverlayMode })}>
      <option value="marching_ants">Marching ants</option><option value="color">Color overlay</option><option value="grayscale">Grayscale</option><option value="black">Black background</option><option value="white">White background</option><option value="mask_only">Mask only</option>
    </select>
    <label class="range-row compact"><span>Opacity</span><input aria-label="Overlay opacity" type="range" min="0.05" max="1" step="0.05" value={state.overlay.opacity} on:input={(event) => updateOverlay({ opacity: Number(event.currentTarget.value) }, 'overlay-opacity')} /></label>
    <label class="color-control">Color <input aria-label="Overlay color" type="color" value={state.overlay.color} on:input={(event) => updateOverlay({ color: event.currentTarget.value }, 'overlay-color')} /></label>
  </div>

  <div class="masks-heading"><div><strong>Named masks</strong><small>{state.namedMasks.length}</small></div><div><button type="button" title="Import PhotoForge mask" on:click={() => onimport('json')}>JSON</button><button type="button" title="Import grayscale PNG" on:click={() => onimport('png')}>PNG</button></div></div>
  <div class="create-mask"><input aria-label="New mask name" placeholder={`Mask ${state.namedMasks.length + 1}`} bind:value={newMaskName} on:keydown={(event) => event.key === 'Enter' && createMask()} /><button type="button" disabled={!state.activeMask} on:click={createMask}>Save active</button></div>
  {#if state.namedMasks.length}
    <ol class="mask-list">
      {#each state.namedMasks as mask, index (mask.id)}
        <li class:locked={mask.locked}>
          <span class="mask-thumbnail" style={`--coverage:${Math.round((mask.mask.data.length / Math.max(1, mask.mask.width * mask.mask.height)) * 40)}%`}>◐</span>
          <input aria-label={`Rename ${mask.name}`} value={mask.name} disabled={mask.locked} on:change={(event) => onnamedaction('rename', mask.id, event.currentTarget.value)} />
          <div class="mask-item-actions">
            <button type="button" title="Load as active selection" on:click={() => onnamedaction('load', mask.id)}>↙</button>
            <button type="button" title={`Combine using ${state.mode}`} disabled={!state.activeMask} on:click={() => onnamedaction('combine', mask.id)}>⊕</button>
            <button type="button" title="Replace from active selection" disabled={mask.locked || !state.activeMask} on:click={() => onnamedaction('replace', mask.id)}>↥</button>
            <button type="button" title={mask.visible ? 'Hide mask' : 'Show mask'} on:click={() => onnamedaction('visible', mask.id)}>{mask.visible ? '◉' : '○'}</button>
            <button type="button" title={mask.locked ? 'Unlock mask' : 'Lock mask'} on:click={() => onnamedaction('locked', mask.id)}>{mask.locked ? '▣' : '□'}</button>
            <button type="button" title="Move mask up" disabled={index === 0} on:click={() => onnamedaction('up', mask.id)}>↑</button>
            <button type="button" title="Move mask down" disabled={index === state.namedMasks.length - 1} on:click={() => onnamedaction('down', mask.id)}>↓</button>
            <button type="button" title="Duplicate mask" on:click={() => onnamedaction('duplicate', mask.id)}>⧉</button>
            <button type="button" title="Export PhotoForge mask" on:click={() => onnamedaction('export', mask.id)}>⇩</button>
            <button type="button" title="Export grayscale PNG" on:click={() => onnamedaction('export_png', mask.id)}>▧</button>
            <button type="button" title="Delete mask" disabled={mask.locked} on:click={() => onnamedaction('delete', mask.id)}>×</button>
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
  .selection-summary, .modifier-note, .empty-masks, .refine-panel p { margin: 0; color: var(--ink-faint); font-size: .66rem; line-height: 1.45; }
  .selection-summary strong { color: var(--accent); }
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
  .refine-panel { display: grid; grid-template-columns: 1fr 1fr; gap: 7px; padding: 9px; border: 1px solid var(--line); border-radius: 7px; }
  .refine-panel label { display: grid; gap: 3px; color: var(--ink-faint); font-size: .58rem; }
  .refine-panel button, .refine-panel p { grid-column: 1 / -1; }
  .overlay-controls { display: grid; gap: 7px; padding: 9px; border: 1px solid var(--line); border-radius: 7px; }
  .color-control input { width: 38px; height: 22px; padding: 0; border: 0; background: none; }
  .masks-heading { margin-top: 4px; }
  .masks-heading > div { display: flex; align-items: center; gap: 5px; }
  .masks-heading strong { font-size: .72rem; }
  .masks-heading small { display: grid; place-items: center; min-width: 18px; height: 18px; border-radius: 99px; color: var(--ink-faint); background: var(--surface-raised); }
  .create-mask { display: grid; grid-template-columns: 1fr auto; gap: 5px; }
  .create-mask input, .mask-list input { padding: 7px; }
  .mask-list { display: grid; gap: 6px; margin: 0; padding: 0; list-style: none; }
  .mask-list li { display: grid; grid-template-columns: 28px 1fr; gap: 5px; padding: 7px; border: 1px solid var(--line); border-radius: 7px; background: rgba(255,255,255,.015); }
  .mask-list li.locked { opacity: .78; }
  .mask-thumbnail { grid-row: 1 / 3; display: grid; place-items: center; border-radius: 5px; color: var(--accent); background: linear-gradient(135deg, rgba(192,231,126,.25), rgba(255,255,255,.03)); }
  .mask-item-actions { grid-column: 2; display: flex; flex-wrap: wrap; gap: 3px; }
  .mask-item-actions button { min-width: 24px; padding: 4px; }
  @media (max-width: 900px) { .tool-grid { grid-template-columns: repeat(4, minmax(48px, 1fr)); } }
</style>
