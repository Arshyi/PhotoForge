<script lang="ts">
  import { onDestroy, onMount } from 'svelte';
  import { invoke } from '@tauri-apps/api/core';
  import TextPanel from './TextPanel.svelte';
  import ShapePanel from './ShapePanel.svelte';
  import { LayerHistory } from '../layers/history';
  import { renderLayerComposite, validateLayerDocument } from '../layers/commands';
  import { replaceSmartSourceDocument, smartSourceDocument } from '../layers/smart';
  import { activeLayer, createGroupLayer, createShapeLayer, createTextLayer, displayRows,
    duplicateLayer, insertLayer, moveLayer, removeLayer, updateLayer, validateDocument } from '../layers/tree';
  import { blendModes, identityTransform, layerKindLabels, type Layer, type LayerDocument,
    type LayerPixelsResult, type LayerTransform } from '../layers/types';
  import { errorMessage } from '../utils/format';

  export let parentDocument: LayerDocument;
  export let sourceId: string;
  export let documentId: number;
  /** Shared with the parent render queue; this editor never opens a new session. */
  export let nextRequestId: () => number;
  export let onsave: (edited: LayerDocument) => Promise<void> | void;
  export let oncancel: () => void;

  const history = new LayerHistory();
  let draft = history.replace(smartSourceDocument(parentDocument, sourceId), 'Open smart contents');
  let dialog: HTMLDialogElement;
  let error = '';
  let previewError = '';
  let previewUrl = '';
  let previewBusy = false;
  let saving = false;
  let processing = false;
  let disposed = false;
  let previewRevision = 0;
  let timer: ReturnType<typeof setTimeout> | undefined;
  let canUndo = false;
  let canRedo = false;
  $: selected = activeLayer(draft);
  $: rows = displayRows(draft);
  $: unavailable = saving || processing;
  $: schedulePreview(draft);

  onMount(() => {
    if (typeof dialog.showModal === 'function') dialog.showModal();
    else dialog.setAttribute('open', '');
    dialog.focus();
  });
  onDestroy(() => {
    disposed = true;
    previewRevision += 1;
    clearTimeout(timer);
    if (typeof dialog?.close === 'function' && dialog.open) dialog.close();
  });

  function syncHistory() {
    canUndo = history.canUndo;
    canRedo = history.canRedo;
  }

  function commit(candidate: LayerDocument, label: string, key?: string) {
    if (unavailable) return;
    try {
      const problems = validateDocument(candidate);
      if (problems.length) throw new Error(problems[0]);
      // The parent graph also has to remain valid: nesting that is legal in a
      // standalone source can exceed the limit once placed in its ancestors.
      replaceSmartSourceDocument(parentDocument, sourceId, candidate);
      draft = history.commit(candidate, label, key);
      syncHistory();
      error = '';
    } catch (reason) { error = errorMessage(reason); }
  }

  function changeLayer(id: string, patch: Partial<Layer>, label: string, key?: string) {
    const layer = draft.layers.length ? rows.find((row) => row.layer.id === id)?.layer : null;
    if (!layer || (layer.locked && !Object.hasOwn(patch, 'locked'))) return;
    commit(updateLayer(draft, id, (current) => ({ ...current, ...patch })), label, key);
  }

  function select(id: string) {
    if (unavailable) return;
    history.endCoalescing();
    draft = history.replaceCurrent({ ...draft, activeLayerId: id });
  }

  function create(kind: 'text' | 'shape' | 'group') {
    const layer = kind === 'text' ? createTextLayer('Text', 'New text', 16, 16, Math.min(48, draft.canvasHeight / 3))
      : kind === 'shape' ? createShapeLayer('Rectangle', { type: 'rectangle', x: 8, y: 8,
        width: Math.max(1, draft.canvasWidth / 2), height: Math.max(1, draft.canvasHeight / 2), cornerRadius: 0 })
      : createGroupLayer('Group');
    commit(insertLayer(draft, layer, null, draft.layers.length), `Add ${kind}`);
  }

  function undo(redo = false) {
    if (unavailable) return;
    draft = (redo ? history.redo() : history.undo()) ?? draft;
    syncHistory();
    error = '';
  }

  function removeSelected() {
    if (selected && !selected.locked) commit(removeLayer(draft, selected.id), 'Delete source layer');
  }

  function transform(key: keyof LayerTransform, raw: string) {
    if (!selected || raw.trim() === '' || !Number.isFinite(Number(raw))) return;
    changeLayer(selected.id, { transform: { ...selected.transform, [key]: Number(raw) } }, 'Transform source layer');
  }

  function schedulePreview(document: LayerDocument) {
    const revision = ++previewRevision;
    clearTimeout(timer);
    previewBusy = true;
    previewError = '';
    timer = setTimeout(() => void preview(document, revision), 100);
  }

  async function preview(document: LayerDocument, revision: number) {
    const request = nextRequestId();
    try {
      const result = await renderLayerComposite(document, [], documentId, request);
      if (disposed || revision !== previewRevision) return;
      if (result.isCurrent && result.requestId === request) previewUrl = result.previewDataUrl;
      else previewError = 'Preview was superseded. Edit a property to refresh it.';
    } catch (reason) {
      if (!disposed && revision === previewRevision) previewError = errorMessage(reason);
    } finally {
      if (!disposed && revision === previewRevision) previewBusy = false;
    }
  }

  async function rasterize(id: string) {
    const current = rows.find((row) => row.layer.id === id)?.layer;
    if (!current || current.locked || unavailable) return;
    const original = draft;
    processing = true;
    try {
      const pixels = await invoke<LayerPixelsResult>('rasterize_semantic_layer', { document: original, layerId: id });
      if (disposed || draft !== original) return;
      processing = false;
      commit(updateLayer(draft, id, (layer) => ({ ...layer, raw: null, mask: null,
        transform: { ...identityTransform }, content: { type: 'pixel', pixelId: pixels.pixelId,
          width: pixels.width, height: pixels.height } })), 'Rasterize source layer');
    } catch (reason) { if (!disposed) error = errorMessage(reason); }
    finally { if (!disposed) processing = false; }
  }

  async function save() {
    if (unavailable) return;
    saving = true;
    error = '';
    try {
      const candidate = replaceSmartSourceDocument(parentDocument, sourceId, draft);
      await validateLayerDocument(candidate);
      if (!disposed) await onsave(draft);
    } catch (reason) { if (!disposed) error = errorMessage(reason); }
    finally { if (!disposed) saving = false; }
  }

  function cancel(event?: Event) {
    event?.preventDefault();
    if (!unavailable) oncancel();
  }

  function keys(event: KeyboardEvent) {
    // Native text controls keep text Undo, selection, clipboard and arrows.
    // No keystroke from this modal may become a parent-document shortcut.
    event.stopPropagation();
    const target = event.target as HTMLElement;
    if (event.key === 'Tab') {
      const controls = [...dialog.querySelectorAll<HTMLElement>('button:not(:disabled), input:not(:disabled), textarea:not(:disabled), select:not(:disabled), [tabindex="0"]')];
      const first = controls[0], last = controls.at(-1);
      if (event.shiftKey && (target === first || target === dialog)) { event.preventDefault(); last?.focus(); }
      else if (!event.shiftKey && target === last) { event.preventDefault(); first?.focus(); }
      return;
    }
    if (event.key === 'Escape') { cancel(event); return; }
    if (target.matches('input, textarea, select, [contenteditable="true"]')) return;
    if ((event.ctrlKey || event.metaKey) && event.key.toLowerCase() === 'z') {
      event.preventDefault(); undo(event.shiftKey);
    } else if ((event.ctrlKey || event.metaKey) && event.key.toLowerCase() === 'y') {
      event.preventDefault(); undo(true);
    } else if (event.key === 'Delete' || event.key === 'Backspace') {
      event.preventDefault(); removeSelected();
    }
  }
</script>

<dialog bind:this={dialog} class="smart-editor" aria-labelledby="smart-editor-title" tabindex="-1" on:cancel={cancel} on:keydown={keys}>
  <header><h2 id="smart-editor-title">Edit smart contents</h2><p>{draft.canvasWidth} × {draft.canvasHeight} · Saving updates every shared instance.</p></header>
  <p class="hint">This editor has its own Undo history. Cancel leaves the parent unchanged. Editing a linked source embeds the result without modifying its file.</p>
  <nav aria-label="Source document actions">
    <button disabled={!canUndo || unavailable} on:click={() => undo()}>Undo source edit</button>
    <button disabled={!canRedo || unavailable} on:click={() => undo(true)}>Redo source edit</button>
    <button disabled={unavailable} on:click={() => create('text')}>Add source text</button>
    <button disabled={unavailable} on:click={() => create('shape')}>Add source rectangle</button>
    <button disabled={unavailable} on:click={() => create('group')}>Add source group</button>
  </nav>
  {#if error}<p role="alert">{error}</p>{/if}
  <div class="workspace">
    <section class="preview" aria-label="Smart source preview">
      {#if previewUrl}<img src={previewUrl} alt="Rendered smart source contents" />{/if}
      {#if previewBusy}<p role="status">Rendering source preview…</p>{/if}
      {#if previewError}<p role="alert">{previewError}</p>{/if}
    </section>
    <section class="properties" aria-label="Source layer editor">
      <ul aria-label="Source layers">
        {#each rows as row (row.layer.id)}
          <li style:padding-left={`${row.depth * 12}px`}>
            <button class:selected={selected?.id === row.layer.id} aria-pressed={selected?.id === row.layer.id} disabled={unavailable}
              on:click={() => select(row.layer.id)}>{row.layer.name} · {layerKindLabels[row.layer.content.type]}{row.layer.raw ? ' · RAW' : ''}</button>
            <button aria-label={`Move ${row.layer.name} up`} disabled={unavailable || row.layer.locked}
              on:click={() => commit(moveLayer(draft, row.layer.id, row.parentId, row.index + 1), 'Reorder source layers')}>↑</button>
            <button aria-label={`Move ${row.layer.name} down`} disabled={unavailable || row.layer.locked || row.index === 0}
              on:click={() => commit(moveLayer(draft, row.layer.id, row.parentId, row.index - 1), 'Reorder source layers')}>↓</button>
          </li>
        {/each}
      </ul>
      {#if selected}
        <fieldset disabled={unavailable}>
          <legend>{selected.name}</legend>
          <label><input type="checkbox" checked={selected.locked} on:change={(event) => changeLayer(selected!.id, { locked: event.currentTarget.checked }, 'Lock source layer')} />Lock source layer</label>
          <fieldset disabled={selected.locked}>
            <label>Name<input aria-label="Source layer name" value={selected.name} maxlength="120"
              on:change={(event) => changeLayer(selected!.id, { name: event.currentTarget.value.trim() }, 'Rename source layer')} /></label>
            <label><input type="checkbox" checked={selected.visible} on:change={(event) => changeLayer(selected!.id, { visible: event.currentTarget.checked }, 'Source layer visibility')} />Visible</label>
            <label>Opacity<input aria-label="Source layer opacity" type="number" min="0" max="1" step="0.01" value={selected.opacity}
              on:change={(event) => changeLayer(selected!.id, { opacity: Number(event.currentTarget.value) }, 'Source layer opacity')} /></label>
            <label>Blend<select aria-label="Source layer blend" value={selected.blendMode}
              on:change={(event) => changeLayer(selected!.id, { blendMode: event.currentTarget.value as Layer['blendMode'] }, 'Source layer blend')}>
              {#each blendModes as mode}<option value={mode.id}>{mode.label}</option>{/each}
            </select></label>
            {#if selected.content.type !== 'adjustment'}
              {#each ['translateX', 'translateY', 'scaleX', 'scaleY', 'rotationDegrees'] as key}
                <label>{key}<input aria-label={`Source ${key}`} type="number" step="any" value={Number(selected.transform[key as keyof LayerTransform])}
                  on:change={(event) => transform(key as keyof LayerTransform, event.currentTarget.value)} /></label>
              {/each}
              <button on:click={() => changeLayer(selected!.id, { transform: { ...identityTransform } }, 'Reset source placement')}>Reset source placement</button>
            {/if}
            <button on:click={() => commit(duplicateLayer(draft, selected!.id).document, 'Duplicate source layer')}>Duplicate source layer</button>
            <button on:click={removeSelected}>Delete source layer</button>
            {#if selected.mask}<p class="hint">The existing layer mask is preserved.</p>{/if}
            {#if selected.raw}<p class="hint">The original RAW source and development settings are preserved.</p>{/if}
            {#if selected.content.type === 'text'}
              <TextPanel content={selected.content} layerId={selected.id}
                onchange={(id, content) => changeLayer(id, { content: { ...content, type: 'text' } }, 'Edit source text', `text:${id}`)}
                onrasterize={(id) => void rasterize(id)} />
            {:else if selected.content.type === 'shape'}
              <ShapePanel content={selected.content}
                onchange={(content) => changeLayer(selected!.id, { content: { ...content, type: 'shape' } }, 'Edit source shape')}
                onrasterize={() => void rasterize(selected!.id)} />
            {:else if selected.content.type === 'smart_object'}
              <p class="hint">Nested smart source {selected.content.sourceId} is preserved. This panel edits its instance placement and properties.</p>
              <button on:click={() => void rasterize(selected!.id)}>Rasterize nested instance</button>
            {/if}
          </fieldset>
        </fieldset>
      {:else}<p>Select a source layer, or add editable text or geometry.</p>{/if}
    </section>
  </div>
  <footer><button disabled={unavailable} on:click={() => cancel()}>Cancel contents edit</button>
    <button disabled={unavailable} on:click={() => void save()}>{saving ? 'Saving contents…' : 'Save contents'}</button></footer>
</dialog>

<style>
  .smart-editor { width: min(1120px, 94vw); max-height: 94vh; overflow: auto; color: #e7edf8; background: #151c29; border: 1px solid #657087; border-radius: 8px; }
  .smart-editor::backdrop { background: rgb(0 0 0 / .65); }
  header h2, header p { margin: 0 0 8px; }
  .hint { font-size: .82rem; opacity: .8; }
  nav, footer { display: flex; flex-wrap: wrap; gap: 8px; padding: 8px 0; }
  footer { justify-content: flex-end; }
  .workspace { display: grid; grid-template-columns: minmax(220px, 1fr) minmax(300px, .85fr); gap: 16px; }
  .preview { align-self: start; position: sticky; top: 0; min-height: 240px; background: #363b46; display: grid; place-items: center; }
  .preview img { display: block; max-width: 100%; max-height: 65vh; object-fit: contain; }
  .properties { min-width: 0; }
  ul { list-style: none; padding: 0; margin: 0; max-height: 210px; overflow: auto; }
  li { display: flex; gap: 4px; margin: 4px 0; }
  li > button:first-child { flex: 1; overflow-wrap: anywhere; text-align: left; }
  button.selected { outline: 2px solid #83b7ff; }
  fieldset { min-width: 0; border: 1px solid #4b586c; display: grid; gap: 8px; }
  label { display: flex; justify-content: space-between; gap: 12px; align-items: center; }
  input:not([type=checkbox]), select { max-width: 160px; }
  [role=alert] { color: #ffb2a6; }
  @media (max-width: 720px) { .workspace { grid-template-columns: 1fr; } .preview { position: static; } }
</style>
