<script lang="ts">
  import MaskThumbnail from './MaskThumbnail.svelte';
  import { activeLayer, childrenOf, displayRows } from '../layers/tree';
  import {
    blendModes,
    layerKindIcons,
    layerKindLabels,
    type AdjustmentTarget,
    type BlendMode,
    type EditTarget,
    type Layer,
    type LayerDocument,
    type LayerPanelAction,
    type LayerRow
  } from '../layers/types';

  export let document: LayerDocument;
  export let thumbnails: Record<string, string> = {};
  export let editTarget: EditTarget = 'layer';
  export let adjustmentTarget: AdjustmentTarget = 'document';
  export let disabled = false;
  export let busy = false;
  export let hasSelection = false;

  export let onselect: (id: string) => void;
  export let ontoggle: (id: string, field: 'visible' | 'locked' | 'collapsed') => void;
  export let onrename: (id: string, name: string) => void;
  export let onopacity: (id: string, value: number) => void;
  export let onblend: (id: string, mode: BlendMode) => void;
  export let onreorder: (id: string, parentId: string | null, index: number) => void;
  export let oncreate: (kind: 'pixel' | 'group' | 'adjustment' | 'import') => void;
  export let onaction: (action: LayerPanelAction, id?: string) => void;
  export let ontargetchange: (target: EditTarget) => void;
  export let onadjustmenttargetchange: (target: AdjustmentTarget) => void;

  let renamingId: string | null = null;
  let renameValue = '';
  let draggingId: string | null = null;
  let dropHint: { id: string; position: 'above' | 'below' | 'inside' } | null = null;

  $: rows = displayRows(document);
  $: selected = activeLayer(document);
  $: selectedIsGroup = selected?.content.type === 'group';
  $: selectedIsAdjustment = selected?.content.type === 'adjustment';
  $: selectedHasMask = Boolean(selected?.mask);
  $: totalLayers = rows.length;
  $: locked = Boolean(selected?.locked);

  function beginRename(layer: Layer) {
    if (disabled) return;
    renamingId = layer.id;
    renameValue = layer.name;
  }

  function commitRename() {
    if (renamingId && renameValue.trim()) onrename(renamingId, renameValue.trim());
    renamingId = null;
  }

  function renameKeys(event: KeyboardEvent) {
    if (event.key === 'Enter') {
      event.preventDefault();
      commitRename();
    } else if (event.key === 'Escape') {
      event.preventDefault();
      renamingId = null;
    }
  }

  function opacityPercent(layer: Layer | null): number {
    return Math.round((layer?.opacity ?? 1) * 100);
  }

  function handleDragStart(event: DragEvent, row: LayerRow) {
    if (disabled || row.layer.locked) {
      event.preventDefault();
      return;
    }
    draggingId = row.layer.id;
    event.dataTransfer?.setData('text/plain', row.layer.id);
    if (event.dataTransfer) event.dataTransfer.effectAllowed = 'move';
  }

  /**
   * Chooses between dropping above, below, or inside a row from where the
   * pointer sits in it. The middle band of a group row means "into the group".
   */
  function dropPosition(event: DragEvent, row: LayerRow): 'above' | 'below' | 'inside' {
    const target = event.currentTarget as HTMLElement | null;
    if (!target) return 'below';
    const bounds = target.getBoundingClientRect();
    const offset = bounds.height > 0 ? (event.clientY - bounds.top) / bounds.height : 0.5;
    if (row.layer.content.type === 'group' && offset > 0.3 && offset < 0.7) return 'inside';
    return offset < 0.5 ? 'above' : 'below';
  }

  function handleDragOver(event: DragEvent, row: LayerRow) {
    if (!draggingId || disabled) return;
    event.preventDefault();
    if (event.dataTransfer) event.dataTransfer.dropEffect = 'move';
    dropHint = { id: row.layer.id, position: dropPosition(event, row) };
  }

  function handleDrop(event: DragEvent, row: LayerRow) {
    if (!draggingId || disabled) return;
    event.preventDefault();
    const position = dropPosition(event, row);
    applyDrop(draggingId, row, position);
    draggingId = null;
    dropHint = null;
  }

  function handleDragEnd() {
    draggingId = null;
    dropHint = null;
  }

  /**
   * Translates a display-order drop onto a stack-order insertion. The panel
   * lists the top layer first, so dropping "above" a row means a higher index
   * in the underlying bottom-first stack.
   */
  function applyDrop(id: string, row: LayerRow, position: 'above' | 'below' | 'inside') {
    if (id === row.layer.id) return;
    if (position === 'inside' && row.layer.content.type === 'group') {
      onreorder(id, row.layer.id, childrenOf(row.layer).length);
      return;
    }
    const parentId = row.parentId;
    const index = position === 'above' ? row.index + 1 : row.index;
    onreorder(id, parentId, index);
  }

  function moveBy(id: string, delta: number) {
    const path = rows.find((row) => row.layer.id === id);
    if (!path) return;
    onreorder(id, path.parentId, Math.max(0, path.index + delta));
  }

  function canMove(row: LayerRow, delta: number): boolean {
    const siblings = row.parentId
      ? childrenOf(rows.find((entry) => entry.layer.id === row.parentId)?.layer as Layer)
      : document.layers;
    const next = row.index + delta;
    return next >= 0 && next < siblings.length;
  }
</script>

<section class="tool-section layers-panel" aria-labelledby="layers-heading">
  <div class="layers-heading">
    <div>
      <h2 id="layers-heading"><span aria-hidden="true">▤</span> Layers</h2>
      <small>{totalLayers} {totalLayers === 1 ? 'layer' : 'layers'}</small>
    </div>
    <div class="create-actions">
      <button
        type="button"
        title="New pixel layer (Ctrl+Shift+N)"
        aria-label="New pixel layer"
        {disabled}
        on:click={() => oncreate('pixel')}>＋</button
      >
      <button
        type="button"
        title="New group (Ctrl+G groups the selection)"
        aria-label="New group"
        {disabled}
        on:click={() => oncreate('group')}>▤</button
      >
      <button
        type="button"
        title="New adjustment layer"
        aria-label="New adjustment layer"
        {disabled}
        on:click={() => oncreate('adjustment')}>◐</button
      >
      <button
        type="button"
        title="Place an image as a new layer"
        aria-label="Place image as layer"
        {disabled}
        on:click={() => oncreate('import')}>⇪</button
      >
    </div>
  </div>

  <div class="target-row" role="group" aria-label="Active editing target">
    <span>Editing</span>
    <div class="segmented">
      <button
        type="button"
        class:active={editTarget === 'layer'}
        aria-pressed={editTarget === 'layer'}
        disabled={disabled || selectedIsAdjustment}
        title="Tools paint into the selected layer's pixels"
        on:click={() => ontargetchange('layer')}>Layer</button
      >
      <button
        type="button"
        class:active={editTarget === 'mask'}
        aria-pressed={editTarget === 'mask'}
        disabled={disabled || !selectedHasMask}
        title="Tools paint into the selected layer's mask"
        on:click={() => ontargetchange('mask')}>Mask</button
      >
      <button
        type="button"
        class:active={editTarget === 'selection'}
        aria-pressed={editTarget === 'selection'}
        {disabled}
        title="Tools change the document selection"
        on:click={() => ontargetchange('selection')}>Selection</button
      >
    </div>
  </div>
  <p class="target-note" data-target={editTarget}>
    {#if editTarget === 'mask'}
      Painting changes the mask on <strong>{selected?.name ?? 'this layer'}</strong>, not its pixels.
    {:else if editTarget === 'layer'}
      Painting changes the pixels of <strong>{selected?.name ?? 'the selected layer'}</strong>.
    {:else}
      Painting changes the document selection, not any layer.
    {/if}
  </p>

  <div class="adjustment-target" role="group" aria-label="Where adjustments are applied">
    <label for="adjustment-target-select">Adjustments go to</label>
    <select
      id="adjustment-target-select"
      value={adjustmentTarget}
      {disabled}
      on:change={(event) =>
        onadjustmenttargetchange(
          (event.currentTarget as HTMLSelectElement).value as AdjustmentTarget
        )}
    >
      <option value="document">The whole document (as before)</option>
      <option value="layer">The selected layer, applied directly</option>
      <option value="adjustmentLayer">A new adjustment layer</option>
    </select>
  </div>

  {#if selected}
    <div class="layer-properties" class:disabled={disabled || locked}>
      <div class="property-row">
        <label for="layer-blend-select">Blend</label>
        <select
          id="layer-blend-select"
          value={selected.blendMode}
          disabled={disabled || locked}
          on:change={(event) =>
            onblend(
              selected?.id ?? '',
              (event.currentTarget as HTMLSelectElement).value as BlendMode
            )}
        >
          {#each blendModes as mode}
            <option value={mode.id}>{mode.label}</option>
          {/each}
        </select>
      </div>
      <div class="property-row range">
        <label for="layer-opacity-range">Opacity</label>
        <input
          id="layer-opacity-range"
          type="range"
          min="0"
          max="100"
          step="1"
          value={opacityPercent(selected)}
          disabled={disabled || locked}
          on:input={(event) =>
            onopacity(selected?.id ?? '', Number((event.currentTarget as HTMLInputElement).value) / 100)}
        />
        <output>{opacityPercent(selected)}%</output>
      </div>
    </div>
  {/if}

  <ul class="layer-list" role="tree" aria-label="Layer stack">
    {#each rows as row (row.layer.id)}
      <li
        role="treeitem"
        aria-selected={document.activeLayerId === row.layer.id}
        aria-expanded={row.layer.content.type === 'group' ? !row.layer.collapsed : undefined}
        aria-level={row.depth + 1}
        class:selected={document.activeLayerId === row.layer.id}
        class:dimmed={!row.layer.visible || row.hiddenByAncestor}
        class:locked={row.layer.locked}
        class:dragging={draggingId === row.layer.id}
        class:drop-above={dropHint?.id === row.layer.id && dropHint.position === 'above'}
        class:drop-below={dropHint?.id === row.layer.id && dropHint.position === 'below'}
        class:drop-inside={dropHint?.id === row.layer.id && dropHint.position === 'inside'}
        style={`--layer-depth: ${row.depth}`}
        draggable={!disabled && !row.layer.locked}
        data-layer-id={row.layer.id}
        data-testid={`layer-row-${row.layer.id}`}
        on:dragstart={(event) => handleDragStart(event, row)}
        on:dragover={(event) => handleDragOver(event, row)}
        on:drop={(event) => handleDrop(event, row)}
        on:dragend={handleDragEnd}
      >
        <div class="row-main">
          {#if row.layer.content.type === 'group'}
            <button
              type="button"
              class="twisty"
              aria-label={row.layer.collapsed ? `Expand ${row.layer.name}` : `Collapse ${row.layer.name}`}
              {disabled}
              on:click|stopPropagation={() => ontoggle(row.layer.id, 'collapsed')}
              >{row.layer.collapsed ? '▸' : '▾'}</button
            >
          {:else}
            <span class="twisty-spacer" aria-hidden="true"></span>
          {/if}

          <button
            type="button"
            class="visibility"
            aria-label={`${row.layer.visible ? 'Hide' : 'Show'} ${row.layer.name}`}
            aria-pressed={row.layer.visible}
            {disabled}
            on:click|stopPropagation={() => ontoggle(row.layer.id, 'visible')}
            >{row.layer.visible ? '◉' : '◌'}</button
          >

          <button
            type="button"
            class="select-layer"
            aria-label={`Select ${row.layer.name}`}
            {disabled}
            on:click={() => onselect(row.layer.id)}
            on:dblclick={() =>
              row.layer.content.type === 'adjustment'
                ? onaction('edit_adjustment', row.layer.id)
                : beginRename(row.layer)}
          >
            <span class="thumbnail" data-kind={row.layer.content.type}>
              {#if thumbnails[row.layer.id]}
                <img src={thumbnails[row.layer.id]} alt="" />
              {:else}
                <em aria-hidden="true">{layerKindIcons[row.layer.content.type]}</em>
              {/if}
            </span>
            <span class="row-text">
              {#if renamingId === row.layer.id}
                <input
                  class="rename-input"
                  aria-label={`Rename ${row.layer.name}`}
                  bind:value={renameValue}
                  on:click|stopPropagation
                  on:blur={commitRename}
                  on:keydown={renameKeys}
                />
              {:else}
                <strong title={row.layer.name}>{row.layer.name}</strong>
              {/if}
              <small>
                <i aria-hidden="true">{layerKindIcons[row.layer.content.type]}</i>
                {layerKindLabels[row.layer.content.type]}
                {#if row.layer.blendMode !== 'normal'}
                  · {blendModes.find((mode) => mode.id === row.layer.blendMode)?.label}
                {/if}
                {#if row.layer.opacity < 1}
                  · {Math.round(row.layer.opacity * 100)}%
                {/if}
              </small>
            </span>
          </button>

          {#if row.layer.mask}
            <span
              class="mask-cell"
              class:disabled-mask={!row.layer.mask.enabled}
              title={`Layer mask${row.layer.mask.enabled ? '' : ' (disabled)'}${row.layer.mask.inverted ? ', inverted' : ''}`}
            >
              <MaskThumbnail
                mask={row.layer.mask.snapshot}
                label={`Mask on ${row.layer.name}`}
                targetWidth={26}
                targetHeight={26}
              />
            </span>
          {/if}

          <button
            type="button"
            class="lock"
            aria-label={`${row.layer.locked ? 'Unlock' : 'Lock'} ${row.layer.name}`}
            aria-pressed={row.layer.locked}
            {disabled}
            on:click|stopPropagation={() => ontoggle(row.layer.id, 'locked')}
            >{row.layer.locked ? '🔒' : '🔓'}</button
          >
        </div>

        {#if document.activeLayerId === row.layer.id}
          <div class="row-actions">
            <button
              type="button"
              aria-label="Move layer up"
              title="Move up"
              disabled={disabled || !canMove(row, 1)}
              on:click={() => moveBy(row.layer.id, 1)}>▲</button
            >
            <button
              type="button"
              aria-label="Move layer down"
              title="Move down"
              disabled={disabled || !canMove(row, -1)}
              on:click={() => moveBy(row.layer.id, -1)}>▼</button
            >
            <button
              type="button"
              aria-label="Rename layer"
              title="Rename"
              {disabled}
              on:click={() => beginRename(row.layer)}>✎</button
            >
            <button
              type="button"
              aria-label="Duplicate layer"
              title="Duplicate (Ctrl+J)"
              {disabled}
              on:click={() => onaction('duplicate', row.layer.id)}>⧉</button
            >
            <button
              type="button"
              aria-label="Delete layer"
              title="Delete"
              {disabled}
              on:click={() => onaction('delete', row.layer.id)}>🗑</button
            >
          </div>
        {/if}
      </li>
    {:else}
      <li class="empty-state">
        <p>No layers yet. Open an image or add a layer to begin.</p>
      </li>
    {/each}
  </ul>

  {#if selected}
    <div class="mask-actions" role="group" aria-label="Layer mask">
      <strong>Mask</strong>
      <div class="button-grid">
        {#if selectedHasMask}
          <button type="button" {disabled} on:click={() => onaction('mask_toggle', selected?.id)}>
            {selected.mask?.enabled ? 'Disable' : 'Enable'}
          </button>
          <button type="button" {disabled} on:click={() => onaction('mask_invert', selected?.id)}
            >Invert</button
          >
          <button
            type="button"
            title="Replace the mask with the current selection"
            disabled={disabled || !hasSelection}
            on:click={() => onaction('mask_from_selection', selected?.id)}>From selection</button
          >
          <button
            type="button"
            {disabled}
            title="Load the mask as the document selection"
            on:click={() => onaction('mask_load_selection', selected?.id)}>To selection</button
          >
          <button
            type="button"
            title="Bake the mask into the layer's pixels"
            disabled={disabled || selectedIsAdjustment}
            on:click={() => onaction('mask_apply', selected?.id)}>Apply</button
          >
          <button type="button" {disabled} on:click={() => onaction('mask_delete', selected?.id)}
            >Delete</button
          >
        {:else}
          <button type="button" {disabled} on:click={() => onaction('mask_white', selected?.id)}
            >Reveal all</button
          >
          <button type="button" {disabled} on:click={() => onaction('mask_black', selected?.id)}
            >Hide all</button
          >
          <button
            type="button"
            disabled={disabled || !hasSelection}
            title={hasSelection ? 'Create a mask from the current selection' : 'Make a selection first'}
            on:click={() => onaction('mask_from_selection', selected?.id)}>From selection</button
          >
        {/if}
      </div>
    </div>

    <div class="layer-actions" role="group" aria-label="Layer actions">
      <button
        type="button"
        disabled={disabled || selectedIsGroup}
        title="Group the selected layer (Ctrl+G)"
        on:click={() => onaction('group', selected?.id)}>Group</button
      >
      <button
        type="button"
        disabled={disabled || !selectedIsGroup}
        title="Ungroup (Ctrl+Shift+G)"
        on:click={() => onaction('ungroup', selected?.id)}>Ungroup</button
      >
      <button
        type="button"
        disabled={disabled || busy}
        title="Merge this layer into the one below it"
        on:click={() => onaction('merge_down', selected?.id)}>Merge down</button
      >
      <button
        type="button"
        disabled={disabled || busy}
        title="Flatten every visible layer into one"
        on:click={() => onaction('flatten')}>Flatten</button
      >
      {#if selectedIsAdjustment}
        <button type="button" {disabled} on:click={() => onaction('edit_adjustment', selected?.id)}
          >Edit adjustment</button
        >
      {/if}
      <button
        type="button"
        disabled={disabled || selectedIsAdjustment}
        title="Return this layer to an untransformed position"
        on:click={() => onaction('reset_transform', selected?.id)}>Reset transform</button
      >
      <button
        type="button"
        disabled={disabled || busy || selectedIsAdjustment}
        title="Bake the transform into the layer's pixels"
        on:click={() => onaction('rasterize_transform', selected?.id)}>Rasterize</button
      >
    </div>
  {/if}
</section>

<style>
  .layers-panel { display: grid; gap: 9px; }
  .layers-heading { display: flex; align-items: center; justify-content: space-between; gap: 8px; }
  .layers-heading > div:first-child { display: flex; align-items: baseline; gap: 7px; }
  .layers-heading h2 { margin: 0; }
  .layers-heading small { color: var(--ink-faint); font-size: .62rem; }
  .create-actions { display: flex; gap: 4px; }
  .create-actions button { min-width: 26px; padding: 5px 6px; }
  .target-row { display: flex; align-items: center; justify-content: space-between; gap: 8px; }
  .target-row > span { color: var(--ink-soft); font-size: .64rem; font-weight: 700; }
  .segmented { display: flex; }
  .segmented button { min-width: 0; padding: 5px 9px; border-radius: 0; font-size: .58rem; }
  .segmented button:first-child { border-radius: 6px 0 0 6px; }
  .segmented button:last-child { border-radius: 0 6px 6px 0; }
  button.active { border-color: var(--accent); color: var(--accent); background: rgba(192,231,126,.1); }
  .target-note { margin: 0; padding: 6px 8px; border-left: 2px solid var(--line); color: var(--ink-faint); font-size: .62rem; line-height: 1.4; }
  .target-note[data-target='mask'] { border-left-color: #e2b96f; color: #e8ca94; background: rgba(226,185,111,.06); }
  .target-note strong { color: var(--ink); }
  .adjustment-target { display: grid; gap: 4px; }
  .adjustment-target label { color: var(--ink-soft); font-size: .64rem; }
  select, .rename-input { min-width: 0; padding: 6px; border: 1px solid var(--line); border-radius: 6px; color: var(--ink); background: var(--surface-raised); font: inherit; }
  .layer-properties { display: grid; gap: 6px; padding: 8px; border: 1px solid var(--line); border-radius: 7px; background: rgba(255,255,255,.02); }
  .layer-properties.disabled { opacity: .6; }
  .property-row { display: grid; grid-template-columns: 54px 1fr; align-items: center; gap: 7px; color: var(--ink-soft); font-size: .64rem; }
  .property-row.range { grid-template-columns: 54px 1fr 42px; }
  .property-row output { color: var(--ink); font-family: var(--font-mono); font-size: .62rem; text-align: right; }
  .property-row input[type='range'] { width: 100%; }
  .layer-list { display: grid; gap: 3px; max-height: 340px; margin: 0; padding: 0; overflow-y: auto; list-style: none; }
  .layer-list li { position: relative; padding: 4px; border: 1px solid transparent; border-radius: 7px; background: rgba(255,255,255,.015); }
  .layer-list li.selected { border-color: var(--accent); background: rgba(192,231,126,.08); }
  .layer-list li.dimmed .row-main { opacity: .45; }
  .layer-list li.locked { background: rgba(255,255,255,.03); }
  .layer-list li.dragging { opacity: .5; }
  .layer-list li.drop-above::before, .layer-list li.drop-below::after { content: ''; position: absolute; left: 4px; right: 4px; height: 2px; background: var(--accent); }
  .layer-list li.drop-above::before { top: -2px; }
  .layer-list li.drop-below::after { bottom: -2px; }
  .layer-list li.drop-inside { border-color: var(--accent); border-style: dashed; }
  .row-main { display: flex; align-items: center; gap: 4px; padding-left: calc(var(--layer-depth, 0) * 14px); }
  .twisty, .visibility, .lock { min-width: 22px; padding: 3px 4px; border: 0; background: none; color: var(--ink-soft); font-size: .72rem; cursor: pointer; }
  .twisty-spacer { display: inline-block; min-width: 22px; }
  .visibility[aria-pressed='true'] { color: var(--accent); }
  .select-layer { display: flex; flex: 1; align-items: center; gap: 7px; min-width: 0; padding: 3px; border: 0; background: none; text-align: left; cursor: pointer; }
  .thumbnail { display: grid; place-items: center; width: 34px; height: 34px; overflow: hidden; border: 1px solid var(--line); border-radius: 5px; background-color: var(--surface-raised); background-image: linear-gradient(45deg, rgba(255,255,255,.06) 25%, transparent 25%, transparent 75%, rgba(255,255,255,.06) 75%), linear-gradient(45deg, rgba(255,255,255,.06) 25%, transparent 25%, transparent 75%, rgba(255,255,255,.06) 75%); background-position: 0 0, 5px 5px; background-size: 10px 10px; }
  .thumbnail img { width: 100%; height: 100%; object-fit: contain; }
  .thumbnail em { color: var(--ink-faint); font-size: .9rem; font-style: normal; }
  .row-text { display: grid; gap: 1px; min-width: 0; }
  .row-text strong { overflow: hidden; color: var(--ink); font-size: .68rem; text-overflow: ellipsis; white-space: nowrap; }
  .row-text small { display: flex; align-items: center; gap: 4px; overflow: hidden; color: var(--ink-faint); font-size: .55rem; text-overflow: ellipsis; white-space: nowrap; }
  .row-text small i { font-style: normal; }
  .rename-input { width: 100%; padding: 3px 5px; font-size: .68rem; }
  .mask-cell { display: grid; place-items: center; padding: 1px; border: 1px solid var(--line); border-radius: 4px; }
  .mask-cell.disabled-mask { opacity: .4; }
  .row-actions { display: flex; gap: 3px; margin-top: 4px; padding-left: calc(var(--layer-depth, 0) * 14px + 22px); }
  .row-actions button { min-width: 24px; padding: 3px 5px; font-size: .58rem; }
  .empty-state { padding: 14px 8px; border: 1px dashed var(--line); border-radius: 7px; text-align: center; }
  .empty-state p { margin: 0; color: var(--ink-faint); font-size: .64rem; }
  .mask-actions { display: grid; gap: 5px; padding: 8px; border: 1px solid var(--line); border-radius: 7px; }
  .mask-actions strong { color: var(--ink-soft); font-size: .62rem; }
  .button-grid, .layer-actions { display: grid; grid-template-columns: repeat(3, 1fr); gap: 4px; }
  .button-grid button, .layer-actions button { min-width: 0; padding: 6px 3px; font-size: .56rem; }
</style>
