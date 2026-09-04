<script lang="ts">
  /**
   * Real-browser interaction harness for the Phase 8 layer UI.
   *
   * This mounts the **real** `LayersPanel`, `AdjustmentLayerDialog`, and
   * `CurveEditor` components against the **real** tree and history modules, so
   * pointer, keyboard, drag, and focus behaviour can be exercised in an actual
   * browser rather than in jsdom.
   *
   * It is not shipped: `harness.html` is not a Vite build input, so nothing here
   * reaches `dist/`. It is served only by the dev server.
   *
   * Operations that genuinely require a Tauri command — merge, flatten,
   * rasterize, apply mask, mask-from-selection, project save and load, export —
   * are deliberately **not** simulated. They record an "unavailable" note so a
   * browser run can never be mistaken for evidence that they work.
   */
  import LayersPanel from '../lib/components/LayersPanel.svelte';
  import AdjustmentLayerDialog from '../lib/components/AdjustmentLayerDialog.svelte';
  import { LayerHistory } from '../lib/layers/history';
  import { resetTransform } from '../lib/layers/transformTool';
  import { adjustmentDefinitions, definitionFor } from '../lib/layers/adjustments';
  import {
    createAdjustmentLayer,
    createDocument,
    createGroupLayer,
    createPixelLayer,
    duplicateLayer,
    findLayer,
    groupLayers,
    insertLayer,
    moveLayer,
    parentOf,
    removeLayer,
    timestamp,
    ungroupLayer,
    updateLayer,
    validateDocument
  } from '../lib/layers/tree';
  import type {
    AdjustmentTarget,
    BlendMode,
    EditTarget,
    LayerDocument,
    LayerPanelAction
  } from '../lib/layers/types';
  import type { BaseEditOperation } from '../lib/types/editor';
  import { buildMask, transparentPixel } from './fixtures';

  const history = new LayerHistory();

  let document: LayerDocument = seed();
  let editTarget: EditTarget = 'layer';
  let adjustmentTarget: AdjustmentTarget = 'document';
  let selectedIds: string[] = [];
  let thumbnails: Record<string, string> = {};
  let adjustmentDraft: BaseEditOperation | null = null;
  let adjustmentMode: 'create' | 'edit' = 'create';
  let adjustmentLayerId: string | null = null;
  /** Every backend-only action a run attempted, so nothing is silently faked. */
  let unavailable: string[] = [];
  let lastNotice = '';
  let counter = 0;

  function seed(): LayerDocument {
    const background = createPixelLayer('Background', 'px-bg', 64, 64);
    const sky = createPixelLayer('Sky', 'px-sky', 64, 64);
    const inner = createGroupLayer('Inner', [createPixelLayer('Leaf', 'px-leaf', 32, 32)]);
    const outer = createGroupLayer('Outer', [inner, createPixelLayer('Cloud', 'px-cloud', 32, 32)]);
    const next = createDocument(64, 64, [background, sky, outer]);
    const bound = { ...next, activeLayerId: sky.id };
    history.replace(bound, 'Open');
    return bound;
  }

  function commit(next: LayerDocument, label: string, coalesceKey?: string) {
    if (next === document) return;
    const problems = validateDocument(next);
    if (problems.length) {
      lastNotice = problems[0];
      return;
    }
    document = history.commit(next, label, coalesceKey);
    selectedIds = selectedIds.filter((id) => findLayer(document, id));
    lastNotice = '';
  }

  function nextId(): string {
    counter += 1;
    return `px-h${counter}`;
  }

  function select(id: string, additive: boolean) {
    if (additive) {
      const current = new Set(selectedIds.length ? selectedIds : [document.activeLayerId ?? '']);
      current.delete('');
      if (current.has(id) && current.size > 1) current.delete(id);
      else current.add(id);
      const parent = parentOf(document, id);
      selectedIds = [...current].filter(
        (candidate) => findLayer(document, candidate) && parentOf(document, candidate) === parent
      );
    } else {
      selectedIds = [id];
    }
    if (document.activeLayerId !== id) {
      document = { ...document, activeLayerId: id };
      history.replaceCurrent(document);
    }
    const layer = findLayer(document, id);
    if (layer?.content.type === 'adjustment') editTarget = 'selection';
    else if (editTarget === 'mask' && !layer?.mask) editTarget = 'layer';
  }

  function create(kind: 'pixel' | 'group' | 'adjustment' | 'import') {
    if (kind === 'adjustment') {
      adjustmentLayerId = null;
      adjustmentMode = 'create';
      adjustmentDraft = adjustmentDefinitions[0].build();
      return;
    }
    if (kind === 'import') {
      // Placing an image requires the native file dialog and the backend image
      // loader; the harness records the attempt rather than inventing a layer.
      unavailable = [...unavailable, 'import (needs native dialog + backend)'];
      return;
    }
    const layer =
      kind === 'group'
        ? createGroupLayer('Group')
        : createPixelLayer('Layer', nextId(), document.canvasWidth, document.canvasHeight);
    commit(
      insertLayer(document, layer, null, document.layers.length),
      kind === 'group' ? 'New group' : 'New layer'
    );
  }

  function action(name: LayerPanelAction, id?: string) {
    const layerId = id ?? document.activeLayerId;
    if (!layerId && name !== 'flatten') return;
    const layer = layerId ? findLayer(document, layerId) : null;

    switch (name) {
      case 'duplicate':
        commit(duplicateLayer(document, layerId as string).document, 'Duplicate layer');
        return;
      case 'delete':
        if (layer?.locked) {
          lastNotice = 'Unlock this layer before deleting it.';
          return;
        }
        commit(removeLayer(document, layerId as string), 'Delete layer');
        return;
      case 'group': {
        const targets = selectedIds.length > 1 ? selectedIds : [layerId as string];
        const grouped = groupLayers(document, targets, 'Group');
        if (!grouped.group) {
          lastNotice = 'Only layers that share a parent can be grouped together.';
          return;
        }
        commit(grouped.document, 'Group layers');
        selectedIds = grouped.group ? [grouped.group.id] : [];
        return;
      }
      case 'ungroup':
        commit(ungroupLayer(document, layerId as string), 'Ungroup');
        return;
      case 'reset_transform':
        commit(
          updateLayer(document, layerId as string, (entry) => ({
            ...entry,
            transform: resetTransform(entry.transform.interpolation)
          })),
          'Reset transform'
        );
        return;
      case 'toggle_pass_through': {
        if (layer?.content.type !== 'group') return;
        const nextIsolated = !layer.content.isolated;
        commit(
          updateLayer(document, layerId as string, (entry) =>
            entry.content.type === 'group'
              ? {
                  ...entry,
                  blendMode: nextIsolated ? entry.blendMode : 'normal',
                  content: { ...entry.content, isolated: nextIsolated }
                }
              : entry
          ),
          nextIsolated ? 'Isolate group' : 'Pass-through group'
        );
        return;
      }
      case 'edit_adjustment':
        if (layer?.content.type !== 'adjustment') return;
        adjustmentLayerId = layer.id;
        adjustmentMode = 'edit';
        adjustmentDraft = layer.content.operation;
        return;
      case 'mask_white':
      case 'mask_black': {
        // A real coverage bitmap, built and checksummed in TypeScript, so the
        // mask thumbnail and mask controls render genuine data. The backend
        // command that normally produces it is not exercised.
        const snapshot = buildMask(16, 16, name === 'mask_white' ? 255 : 0);
        commit(
          updateLayer(document, layerId as string, (entry) => ({
            ...entry,
            mask: { snapshot, enabled: true, inverted: false }
          })),
          'Add mask'
        );
        editTarget = 'mask';
        return;
      }
      case 'mask_toggle':
        commit(
          updateLayer(document, layerId as string, (entry) => ({
            ...entry,
            mask: entry.mask ? { ...entry.mask, enabled: !entry.mask.enabled } : null
          })),
          'Toggle mask'
        );
        return;
      case 'mask_invert':
        commit(
          updateLayer(document, layerId as string, (entry) => ({
            ...entry,
            mask: entry.mask ? { ...entry.mask, inverted: !entry.mask.inverted } : null
          })),
          'Invert mask'
        );
        return;
      case 'mask_delete':
        commit(
          updateLayer(document, layerId as string, (entry) => ({ ...entry, mask: null })),
          'Delete mask'
        );
        if (editTarget === 'mask') editTarget = 'layer';
        return;
      default:
        unavailable = [...unavailable, `${name} (needs backend)`];
        lastNotice = `${name} requires the PhotoForge backend and was not simulated.`;
    }
  }

  function updateAdjustment(operation: BaseEditOperation, coalesceKey?: string) {
    adjustmentDraft = operation;
    if (!adjustmentLayerId) return;
    commit(
      updateLayer(document, adjustmentLayerId, (layer) =>
        layer.content.type === 'adjustment'
          ? { ...layer, content: { type: 'adjustment', operation } }
          : layer
      ),
      'Adjustment settings',
      coalesceKey
    );
  }

  function confirmAdjustment(operation: BaseEditOperation) {
    if (adjustmentLayerId) {
      history.endCoalescing();
    } else {
      const layer = createAdjustmentLayer(
        definitionFor(operation.type)?.label ?? 'Adjustment',
        operation
      );
      commit(insertLayer(document, layer, null, document.layers.length), 'New adjustment layer');
    }
    adjustmentDraft = null;
    adjustmentLayerId = null;
  }

  function undo() {
    const next = history.undo();
    if (next) document = next;
  }

  function redo() {
    const next = history.redo();
    if (next) document = next;
  }

  /** Adds many layers so the panel's scrolling and density can be exercised. */
  function addMany(count: number) {
    let next = document;
    for (let index = 0; index < count; index += 1) {
      next = insertLayer(
        next,
        createPixelLayer(`Bulk ${index + 1}`, nextId(), 64, 64),
        null,
        next.layers.length
      );
    }
    commit(next, `Add ${count} layers`);
  }

  /** Builds a deeply nested chain to exercise indentation and depth limits. */
  function nestDeep(depth: number) {
    let inner = createPixelLayer('Deep leaf', nextId(), 32, 32);
    let chain = createGroupLayer('Depth 1', [inner]);
    for (let level = 2; level <= depth; level += 1) {
      chain = createGroupLayer(`Depth ${level}`, [chain]);
    }
    commit(insertLayer(document, chain, null, document.layers.length), `Nest ${depth} deep`);
  }

  function toggleThumbnails() {
    if (Object.keys(thumbnails).length) {
      thumbnails = {};
      return;
    }
    // A real 1x1 PNG data URL: enough to prove the panel renders an <img> and
    // lays it out, without claiming the backend thumbnail renderer was used.
    const next: Record<string, string> = {};
    for (const layer of [...document.layers]) next[layer.id] = transparentPixel;
    thumbnails = next;
  }

  function reset() {
    history.clear();
    document = seed();
    selectedIds = [];
    thumbnails = {};
    unavailable = [];
    lastNotice = '';
    editTarget = 'layer';
    adjustmentTarget = 'document';
    adjustmentDraft = null;
    adjustmentLayerId = null;
  }

  // A compact machine-readable snapshot the browser run can assert against.
  $: stateDump = JSON.stringify(
    {
      activeLayerId: document.activeLayerId,
      selectedIds,
      layerNames: document.layers.map((layer) => layer.name),
      canUndo: history.canUndo,
      canRedo: history.canRedo,
      undoLabel: history.undoLabel,
      currentLabel: history.currentLabel,
      undoDepth: history.undoDepth,
      editTarget,
      adjustmentTarget,
      problems: validateDocument(document),
      unavailable,
      lastNotice,
      tree: document.layers.map(function describe(layer): unknown {
        return {
          name: layer.name,
          kind: layer.content.type,
          visible: layer.visible,
          locked: layer.locked,
          opacity: Number(layer.opacity.toFixed(3)),
          blendMode: layer.blendMode,
          collapsed: layer.collapsed,
          mask: layer.mask
            ? { enabled: layer.mask.enabled, inverted: layer.mask.inverted }
            : null,
          isolated: layer.content.type === 'group' ? layer.content.isolated : undefined,
          transform: layer.transform,
          operation: layer.content.type === 'adjustment' ? layer.content.operation : undefined,
          children:
            layer.content.type === 'group' ? layer.content.children.map(describe) : undefined
        };
      })
    },
    null,
    1
  );
</script>

<div class="harness">
  <div class="harness-controls">
    <button type="button" data-testid="undo" on:click={undo}>Undo</button>
    <button type="button" data-testid="redo" on:click={redo}>Redo</button>
    <button type="button" data-testid="add-many" on:click={() => addMany(40)}>+40 layers</button>
    <button type="button" data-testid="nest-deep" on:click={() => nestDeep(6)}>Nest 6</button>
    <button type="button" data-testid="nest-overflow" on:click={() => nestDeep(20)}>Nest 20</button>
    <button type="button" data-testid="toggle-thumbs" on:click={toggleThumbnails}>Thumbs</button>
    <button type="button" data-testid="reset" on:click={reset}>Reset</button>
    <input data-testid="text-field" placeholder="type here to test shortcut suppression" />
  </div>

  <div class="harness-body">
    <div class="harness-panel">
      <LayersPanel
        {document}
        {thumbnails}
        {editTarget}
        {adjustmentTarget}
        {selectedIds}
        disabled={false}
        busy={false}
        hasSelection={true}
        onselect={select}
        ontoggle={(id, field) =>
          commit(
            updateLayer(document, id, (layer) => ({ ...layer, [field]: !layer[field] })),
            field
          )}
        onrename={(id, name) =>
          commit(updateLayer(document, id, (layer) => ({ ...layer, name })), 'Rename layer')}
        onopacity={(id, value) =>
          commit(
            updateLayer(document, id, (layer) => ({ ...layer, opacity: value })),
            'Layer opacity',
            `opacity:${id}`
          )}
        onblend={(id, mode: BlendMode) =>
          commit(updateLayer(document, id, (layer) => ({ ...layer, blendMode: mode })), 'Blend mode')}
        onreorder={(id, parentId, index) => {
          const next = moveLayer(document, id, parentId, index);
          if (next === document) {
            lastNotice = 'That move is not allowed: a group cannot contain itself.';
            return;
          }
          commit(next, 'Reorder layer', `move:${id}`);
        }}
        oncreate={create}
        onaction={action}
        ontargetchange={(target) => (editTarget = target)}
        onadjustmenttargetchange={(target) => (adjustmentTarget = target)}
      />
    </div>

    <pre class="harness-state" data-testid="state">{stateDump}</pre>
  </div>
</div>

<AdjustmentLayerDialog
  operation={adjustmentDraft}
  mode={adjustmentMode}
  layerName={adjustmentLayerId
    ? findLayer(document, adjustmentLayerId)?.name ?? 'adjustment layer'
    : ''}
  onchange={updateAdjustment}
  onconfirm={confirmAdjustment}
  oncancel={() => {
    adjustmentDraft = null;
    adjustmentLayerId = null;
  }}
/>

<style>
  .harness { display: grid; gap: 10px; padding: 10px; }
  .harness-controls { display: flex; flex-wrap: wrap; gap: 6px; }
  .harness-controls button { padding: 6px 10px; }
  .harness-controls input { flex: 1; min-width: 160px; padding: 6px; border: 1px solid var(--line); border-radius: 6px; color: var(--ink); background: var(--surface-raised); }
  .harness-body { display: grid; grid-template-columns: minmax(280px, 380px) 1fr; gap: 12px; align-items: start; }
  .harness-panel { min-width: 0; }
  .harness-state { max-height: 78vh; margin: 0; padding: 8px; overflow: auto; border: 1px solid var(--line); border-radius: 8px; color: var(--ink-faint); background: var(--surface-raised); font-family: var(--font-mono, monospace); font-size: .6rem; line-height: 1.35; }
  @media (max-width: 900px) {
    .harness-body { grid-template-columns: 1fr; }
    .harness-state { max-height: 40vh; }
  }
</style>
