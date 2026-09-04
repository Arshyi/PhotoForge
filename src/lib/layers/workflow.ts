import type { BaseEditOperation, EditOperation } from '../types/editor';
import {
  childrenOf,
  createAdjustmentLayer,
  eachLayer,
  findLayer,
  insertLayer,
  parentOf,
  pathTo,
  removeLayer,
  updateLayer
} from './tree';
import type { BlendMode, Layer, LayerDocument } from './types';
import { mergeSafetyProblem } from './mergeSafety';

/** Mirrors `layers::MAX_LAYER_WORKFLOW_STEPS`. */
export const MAX_LAYER_WORKFLOW_STEPS = 100;
/** Mirrors `WORKFLOW_SCHEMA_VERSION`; version 1 files carry no layer steps. */
export const WORKFLOW_SCHEMA_VERSION = 2;
export const MIN_WORKFLOW_SCHEMA_VERSION = 1;

export type LayerSelector =
  | { type: 'id'; id: string }
  | { type: 'active' }
  | { type: 'last_created' }
  | { type: 'name'; name: string }
  | { type: 'bottom' }
  | { type: 'top' };

export type LayerWorkflowStep =
  | { type: 'select_layer'; selector: LayerSelector }
  | { type: 'set_visibility'; selector: LayerSelector; visible: boolean }
  | { type: 'set_opacity'; selector: LayerSelector; opacity: number }
  | { type: 'set_blend_mode'; selector: LayerSelector; blendMode: BlendMode }
  | { type: 'create_adjustment_layer'; operation: BaseEditOperation; name?: string | null }
  | { type: 'apply_to_layer'; selector: LayerSelector; operations: EditOperation[] }
  | { type: 'create_mask_from_selection'; selector: LayerSelector }
  | { type: 'merge_down'; selector: LayerSelector }
  | { type: 'flatten' }
  | { type: 'export_composite' };

/** Steps that need work only the backend can do. */
export type DeferredLayerStep =
  | { type: 'apply_to_layer'; layerId: string; operations: EditOperation[] }
  | { type: 'create_mask_from_selection'; layerId: string }
  | { type: 'merge_down'; layerId: string }
  | { type: 'flatten' }
  | { type: 'export_composite' };

export interface LayerWorkflowResult {
  document: LayerDocument;
  /** Steps the caller must carry out through the backend, in order. */
  deferred: DeferredLayerStep[];
  /** One label per applied step, for the history entry. */
  labels: string[];
}

export class LayerWorkflowError extends Error {}

export function selectorDescription(selector: LayerSelector): string {
  switch (selector.type) {
    case 'id':
      return `layer ${selector.id}`;
    case 'active':
      return 'the selected layer';
    case 'last_created':
      return 'the layer created by an earlier step';
    case 'name':
      return `a layer named ${selector.name}`;
    case 'bottom':
      return 'the bottom layer';
    case 'top':
      return 'the top layer';
  }
}

/**
 * Resolves a selector against a document, failing closed.
 *
 * Every failure throws rather than falling back to another layer, which is what
 * stops a replay from silently editing the wrong thing.
 */
export function resolveSelector(
  document: LayerDocument,
  selector: LayerSelector,
  lastCreated: string | null
): string {
  let found: string | undefined;
  switch (selector.type) {
    case 'id':
      found = findLayer(document, selector.id)?.id;
      break;
    case 'active':
      found = document.activeLayerId
        ? findLayer(document, document.activeLayerId)?.id
        : undefined;
      break;
    case 'last_created':
      found = lastCreated ? findLayer(document, lastCreated)?.id : undefined;
      break;
    case 'bottom':
      found = document.layers[0]?.id;
      break;
    case 'top':
      found = document.layers.at(-1)?.id;
      break;
    case 'name': {
      const matches = eachLayer(document).filter((layer) => layer.name === selector.name);
      if (matches.length > 1) {
        throw new LayerWorkflowError(
          `${matches.length} layers are named ${selector.name}; a workflow selector must be unambiguous.`
        );
      }
      found = matches[0]?.id;
      break;
    }
  }
  if (!found) {
    throw new LayerWorkflowError(
      `This workflow needs ${selectorDescription(selector)}, which this document does not have.`
    );
  }
  return found;
}

function requirePixelLayer(document: LayerDocument, layerId: string, step: string): Layer {
  const layer = findLayer(document, layerId);
  if (!layer) throw new LayerWorkflowError(`Layer ${layerId} is missing.`);
  if (layer.content.type !== 'pixel') {
    throw new LayerWorkflowError(
      `${step} needs a pixel layer, but ${layer.name} is a ${layer.content.type} layer.`
    );
  }
  return layer;
}

function stepLabel(step: LayerWorkflowStep): string {
  switch (step.type) {
    case 'select_layer':
      return 'Select layer';
    case 'set_visibility':
      return 'Layer visibility';
    case 'set_opacity':
      return 'Layer opacity';
    case 'set_blend_mode':
      return 'Blend mode';
    case 'create_adjustment_layer':
      return 'New adjustment layer';
    case 'apply_to_layer':
      return 'Apply to layer';
    case 'create_mask_from_selection':
      return 'Mask from selection';
    case 'merge_down':
      return 'Merge down';
    case 'flatten':
      return 'Flatten image';
    case 'export_composite':
      return 'Export composite';
  }
}

/**
 * Applies layer steps to a document.
 *
 * Every selector is resolved as the run proceeds; the first failure throws and
 * the caller keeps its original document, so a replay is all-or-nothing rather
 * than half-applied. Steps that need pixel work are returned as `deferred` for
 * the caller to run through the backend in order.
 */
export function runLayerWorkflow(
  document: LayerDocument,
  steps: LayerWorkflowStep[],
  now?: Date
): LayerWorkflowResult {
  if (steps.length > MAX_LAYER_WORKFLOW_STEPS) {
    throw new LayerWorkflowError(
      `A workflow may contain at most ${MAX_LAYER_WORKFLOW_STEPS} layer steps.`
    );
  }

  let current = document;
  let lastCreated: string | null = null;
  const deferred: DeferredLayerStep[] = [];
  const labels: string[] = [];

  for (const step of steps) {
    labels.push(stepLabel(step));
    switch (step.type) {
      case 'create_adjustment_layer': {
        const layer = createAdjustmentLayer(
          step.name?.trim() || 'Adjustment',
          step.operation,
          now
        );
        current = insertLayer(current, layer, null, current.layers.length);
        lastCreated = layer.id;
        break;
      }
      case 'select_layer': {
        const id = resolveSelector(current, step.selector, lastCreated);
        current = { ...current, activeLayerId: id };
        break;
      }
      case 'set_visibility': {
        const id = resolveSelector(current, step.selector, lastCreated);
        current = updateLayer(current, id, (layer) => ({ ...layer, visible: step.visible }), now);
        break;
      }
      case 'set_opacity': {
        if (!Number.isFinite(step.opacity) || step.opacity < 0 || step.opacity > 1) {
          throw new LayerWorkflowError('Layer opacity must be between 0 and 1.');
        }
        const id = resolveSelector(current, step.selector, lastCreated);
        current = updateLayer(current, id, (layer) => ({ ...layer, opacity: step.opacity }), now);
        break;
      }
      case 'set_blend_mode': {
        const id = resolveSelector(current, step.selector, lastCreated);
        current = updateLayer(
          current,
          id,
          (layer) => ({ ...layer, blendMode: step.blendMode }),
          now
        );
        break;
      }
      case 'apply_to_layer': {
        const id = resolveSelector(current, step.selector, lastCreated);
        requirePixelLayer(current, id, 'Apply to layer');
        if (step.operations.length === 0) {
          throw new LayerWorkflowError('Apply to layer needs at least one operation.');
        }
        deferred.push({ type: 'apply_to_layer', layerId: id, operations: step.operations });
        break;
      }
      case 'create_mask_from_selection': {
        const id = resolveSelector(current, step.selector, lastCreated);
        deferred.push({ type: 'create_mask_from_selection', layerId: id });
        break;
      }
      case 'merge_down': {
        const id = resolveSelector(current, step.selector, lastCreated);
        requirePixelLayer(current, id, 'Merge down');
        const path = pathTo(current, id);
        if (!path || path[path.length - 1] === 0) {
          throw new LayerWorkflowError('There is no layer beneath this one to merge into.');
        }
        const parent = parentOf(current, id);
        const siblings = parent ? childrenOf(findLayer(current, parent)!) : current.layers;
        const below = siblings[path[path.length - 1] - 1];
        const mergeProblem = mergeSafetyProblem(current, [below.id, id]);
        if (mergeProblem) throw new LayerWorkflowError(mergeProblem);
        deferred.push({ type: 'merge_down', layerId: id });
        break;
      }
      case 'flatten':
        deferred.push({ type: 'flatten' });
        break;
      case 'export_composite':
        deferred.push({ type: 'export_composite' });
        break;
    }
  }

  return { document: current, deferred, labels };
}

/**
 * Checks a workflow against a document without changing anything, so the UI can
 * warn before a replay starts.
 */
export function planLayerWorkflow(
  document: LayerDocument,
  steps: LayerWorkflowStep[]
): { ok: true } | { ok: false; problem: string } {
  try {
    runLayerWorkflow(document, steps);
    return { ok: true };
  } catch (error) {
    return {
      ok: false,
      problem: error instanceof Error ? error.message : String(error)
    };
  }
}

/** Records the layer steps a user's current document changes represent. */
export function describeLayerSteps(steps: LayerWorkflowStep[]): string[] {
  return steps.map((step) => {
    const label = stepLabel(step);
    if ('selector' in step) return `${label} — ${selectorDescription(step.selector)}`;
    if (step.type === 'create_adjustment_layer') {
      return `${label} — ${step.name?.trim() || step.operation.type}`;
    }
    return label;
  });
}

/** True when a group's children would be affected by a step targeting it. */
export function affectsChildren(document: LayerDocument, layerId: string): boolean {
  const layer = findLayer(document, layerId);
  return Boolean(layer && childrenOf(layer).length > 0);
}

/** Removes a layer and everything that referenced it, used by merge replay. */
export function dropLayer(document: LayerDocument, layerId: string): LayerDocument {
  const parent = parentOf(document, layerId);
  const next = removeLayer(document, layerId);
  return parent ? next : next;
}
