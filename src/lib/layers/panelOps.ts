/**
 * What each Layers-panel action asks the engine to do.
 *
 * Since 0.15.0 the panel does not edit the layer tree. Each action here names the
 * registered operations it needs and nothing more; the Rust transaction engine
 * applies them, validates the result, and the document that comes back is the one
 * that becomes the next undo entry. There is no second implementation of "hide a
 * layer" or "group these" to drift from the first.
 *
 * Every function is a pure description: a label, the calls, and — for a gesture — a
 * key that folds a drag into one undo step. Nothing here touches a document, which
 * is what makes this file checkable against the registry snapshot
 * (`panelOps.test.ts`) without a backend.
 */
import type { BaseEditOperation } from '../types/editor';
import type { LayerSelector, OperationCall } from '../operations/types';
import type { BlendMode, Layer, LayerPixelsResult } from './types';

/** The request a panel action makes of the engine. */
export interface PanelPlan {
  /** The name of the one undo entry a successful run becomes. */
  label: string;
  steps: OperationCall[];
  /** Folds a continuous gesture (an opacity drag, a reorder drag) into one undo step. */
  coalesceKey?: string;
}

export type ToggleField = 'visible' | 'locked' | 'collapsed';

export const byId = (id: string): LayerSelector => ({ type: 'id', id });

function plan(label: string, op: string, params: Record<string, unknown> = {}, coalesceKey?: string): PanelPlan {
  return { label, steps: [{ op, params }], ...(coalesceKey ? { coalesceKey } : {}) };
}

/** Show/hide, lock/unlock, collapse/expand: `layer` supplies the value it has now. */
export function toggleLayerField(layer: Layer, field: ToggleField): PanelPlan {
  switch (field) {
    case 'visible':
      return plan('Layer visibility', 'core.layer.set_visible', { selector: byId(layer.id), visible: !layer.visible });
    case 'locked':
      return plan('Lock layer', 'core.layer.set_locked', { selector: byId(layer.id), locked: !layer.locked });
    case 'collapsed':
      return plan('Collapse group', 'core.layer.set_collapsed', { selector: byId(layer.id), collapsed: !layer.collapsed });
  }
}

export function renameLayer(id: string, name: string): PanelPlan {
  return plan('Rename layer', 'core.layer.rename', { selector: byId(id), name });
}

/** A drag of the opacity slider is one undo step. */
export function setOpacity(id: string, opacity: number): PanelPlan {
  return plan('Layer opacity', 'core.layer.set_opacity', { selector: byId(id), opacity }, `layer-opacity:${id}`);
}

export function setBlendMode(id: string, blendMode: BlendMode): PanelPlan {
  return plan('Blend mode', 'core.layer.set_blend_mode', { selector: byId(id), blendMode });
}

/** `index` counts among the new siblings with the layer taken out, 0 being the bottom. */
export function moveLayer(id: string, parentId: string | null, index: number): PanelPlan {
  return plan(
    'Reorder layer',
    'core.layer.move',
    { selector: byId(id), ...(parentId ? { parent: byId(parentId) } : {}), index },
    `layer-move:${id}`
  );
}

export function addGroup(): PanelPlan {
  return plan('New group', 'core.layer.add_group');
}

export function addPixelLayer(): PanelPlan {
  return plan('New layer', 'core.layer.add_pixel');
}

/** An image the backend has already decoded into a session buffer, shown as a new layer. */
export function placeImage(imported: LayerPixelsResult): PanelPlan {
  return plan('Place image as layer', 'core.layer.place_pixels', {
    pixelId: imported.pixelId,
    width: imported.width,
    height: imported.height,
    name: imported.filename ?? 'Placed image',
    ...(imported.raw ? { raw: imported.raw } : {}),
    ...(imported.origin ? { origin: imported.origin } : {})
  });
}

export function addAdjustment(operation: BaseEditOperation, name: string): PanelPlan {
  return plan('New adjustment layer', 'core.layer.add_adjustment', { operation, name });
}

/** Changing an open adjustment is a gesture: dragging its sliders is one undo step. */
export function updateAdjustment(id: string, operation: BaseEditOperation, coalesceKey?: string): PanelPlan {
  return plan('Adjustment settings', 'core.layer.update_adjustment', { selector: byId(id), operation }, coalesceKey);
}

export function duplicateLayer(id: string): PanelPlan {
  return plan('Duplicate layer', 'core.layer.duplicate', { selector: byId(id) });
}

export function deleteLayer(id: string): PanelPlan {
  return plan('Delete layer', 'core.layer.delete', { selector: byId(id) });
}

export function groupLayers(ids: string[]): PanelPlan {
  return plan(
    ids.length > 1 ? 'Group layers' : 'Group layer',
    'core.layer.group',
    { selectors: ids.map(byId), name: 'Group' }
  );
}

export function ungroupLayer(id: string): PanelPlan {
  return plan('Ungroup', 'core.layer.ungroup', { selector: byId(id) });
}

export function resetTransform(id: string): PanelPlan {
  return plan('Reset transform', 'core.layer.reset_transform', { selector: byId(id) });
}

/** `group` supplies the state it has now; the plan switches it. */
export function togglePassThrough(group: Layer): PanelPlan {
  const passThrough = group.content.type === 'group' && group.content.isolated;
  return plan(passThrough ? 'Pass-through group' : 'Isolate group', 'core.layer.set_pass_through', {
    selector: byId(group.id),
    passThrough
  });
}

/**
 * Clips a layer to the one below it, or releases it. `layer` supplies the state it
 * has now; the plan switches it.
 */
export function toggleClipped(layer: Layer): PanelPlan {
  const clipped = !(layer.clipped ?? false);
  return plan(clipped ? 'Clip to layer below' : 'Release clipping', 'core.layer.set_clipped', {
    selector: byId(layer.id),
    clipped
  });
}

/** A mask that shows everything (`filled`) or hides everything. */
export function addMask(id: string, filled: boolean): PanelPlan {
  return plan(filled ? 'Add mask' : 'Add hiding mask', 'core.layer.mask_add', { selector: byId(id), filled });
}

export function toggleMask(layer: Layer): PanelPlan {
  const enabled = !(layer.mask?.enabled ?? true);
  return plan(enabled ? 'Enable mask' : 'Disable mask', 'core.layer.mask_set_enabled', { selector: byId(layer.id), enabled });
}

export function invertMask(layer: Layer): PanelPlan {
  return plan('Invert mask', 'core.layer.mask_set_inverted', {
    selector: byId(layer.id),
    inverted: !(layer.mask?.inverted ?? false)
  });
}

export function removeMask(id: string): PanelPlan {
  return plan('Delete mask', 'core.layer.mask_remove', { selector: byId(id) });
}
