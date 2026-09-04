import type { EditOperation } from '../types/editor';
import type { MaskSnapshot } from '../selections/types';
import { validateEditOperation, validateLayerWorkflowSteps } from '../utils/workflows';
import {
  childrenOf, createAdjustmentLayer, createDocument, createPixelLayer, eachLayer,
  findLayer, insertLayer, parentOf, pathTo, removeLayer, updateLayer, validateDocument
} from './tree';
import type { LayerDocument, LayerMaskResult, LayerPixelsResult } from './types';
import { resolveSelector, type LayerWorkflowStep } from './workflow';
import { mergeSafetyProblem } from './mergeSafety';

export interface LayerWorkflowBackend {
  validate(document: LayerDocument, steps: LayerWorkflowStep[]): Promise<unknown>;
  apply(document: LayerDocument, layerId: string, operations: EditOperation[]): Promise<LayerPixelsResult>;
  mask(document: LayerDocument, layerId: string, selection: MaskSnapshot): Promise<LayerMaskResult>;
  merge(document: LayerDocument, ids: string[]): Promise<LayerPixelsResult>;
  flatten(document: LayerDocument): Promise<LayerPixelsResult>;
  export(document: LayerDocument): Promise<unknown>;
}

/**
 * Preflight the entire structural sequence, then replay it in order against a
 * private tree. Pixel writes only create immutable buffers; no visible state or
 * history changes until the caller commits the returned tree. The caller owns
 * the operation lock and releases orphaned buffers on failure.
 *
 * Unlike the old pure helper's deferred list, a merge/flatten/mask takes effect
 * before the next selector resolves. Export is allowed only as the last step.
 */
export async function executeLayerWorkflow(
  document: LayerDocument,
  steps: LayerWorkflowStep[],
  selection: MaskSnapshot | null,
  backend: LayerWorkflowBackend
): Promise<LayerDocument> {
  const problems = validateLayerWorkflowSteps(steps);
  if (problems.length) throw new Error(problems[0]);
  const exports = steps.flatMap((step, index) => step.type === 'export_composite' ? [index] : []);
  if (exports.length > 1 || (exports.length === 1 && exports[0] !== steps.length - 1)) {
    throw new Error('Export composite must be the final step, and may appear only once.');
  }
  if (selection && steps.some((step) => step.type === 'create_mask_from_selection')) assertMask(selection);
  await walk(document, steps, selection, null);
  return walk(document, steps, selection, backend);
}

function assertDocument(document: LayerDocument) {
  const problems = validateDocument(document);
  if (problems.length) throw new Error(problems[0]);
}

function assertUnlocked(document: LayerDocument, id: string) {
  let current: string | null = id;
  while (current) {
    const layer = findLayer(document, current);
    if (!layer) throw new Error(`Layer ${current} no longer exists.`);
    if (layer.locked) throw new Error(`Unlock ${layer.name} before replaying this layer edit.`);
    current = parentOf(document, current);
  }
}

function assertSubtreeUnlocked(document: LayerDocument, id: string) {
  assertUnlocked(document, id);
  const descendants = [...childrenOf(findLayer(document, id)!)];
  while (descendants.length) {
    const layer = descendants.pop()!;
    if (layer.locked) throw new Error(`Unlock ${layer.name} before replaying this layer edit.`);
    descendants.push(...childrenOf(layer));
  }
}

function assertPixels(result: Pick<LayerPixelsResult, 'pixelId' | 'width' | 'height'>, width: number, height: number) {
  if (result.width !== width || result.height !== height ||
    typeof result.pixelId !== 'string' || !/^[A-Za-z0-9_-]{1,64}$/.test(result.pixelId)) {
    throw new Error('The layer worker returned an invalid pixel buffer or unexpected dimensions.');
  }
}

function assertMask(snapshot: MaskSnapshot) {
  // Use the same bounded coverage/checksum validation as persisted selections.
  const problem = validateEditOperation({
    type: 'masked', operation: { type: 'grayscale' }, mask: snapshot, invert: false, mask_id: null
  });
  if (problem) throw new Error(`Invalid workflow selection mask: ${problem}`);
}

async function walk(
  document: LayerDocument,
  steps: LayerWorkflowStep[],
  selection: MaskSnapshot | null,
  backend: LayerWorkflowBackend | null
): Promise<LayerDocument> {
  let current = document;
  let lastCreated: string | null = null;
  assertDocument(current);
  for (const [index, step] of steps.entries()) {
    const id = 'selector' in step ? resolveSelector(current, step.selector, lastCreated) : null;
    if (id && step.type !== 'select_layer') assertUnlocked(current, id);
    // Resolve last_created locally; the backend sees the actual newly created
    // ID and validates the exact step against its actual input tree.
    if (backend) await backend.validate(current, [id && 'selector' in step
      ? { ...step, selector: { type: 'id', id } } : step]);
    switch (step.type) {
      case 'select_layer':
        current = { ...current, activeLayerId: id };
        break;
      case 'set_visibility':
        current = updateLayer(current, id!, (layer) => ({ ...layer, visible: step.visible }));
        break;
      case 'set_opacity':
        current = updateLayer(current, id!, (layer) => ({ ...layer, opacity: step.opacity }));
        break;
      case 'set_blend_mode':
        current = updateLayer(current, id!, (layer) => ({ ...layer, blendMode: step.blendMode }));
        break;
      case 'create_adjustment_layer': {
        const layer = createAdjustmentLayer(step.name?.trim() || 'Adjustment', structuredClone(step.operation));
        current = insertLayer(current, layer, null, current.layers.length);
        lastCreated = layer.id;
        break;
      }
      case 'apply_to_layer': {
        const layer = findLayer(current, id!)!;
        if (layer.content.type !== 'pixel') throw new Error('Apply to layer needs a pixel layer.');
        if (backend) {
          const result = await backend.apply(current, id!, step.operations);
          assertPixels(result, layer.content.width, layer.content.height);
          current = updateLayer(current, id!, (entry) => ({ ...entry, content: {
            type: 'pixel', pixelId: result.pixelId, width: result.width, height: result.height
          } }));
        }
        break;
      }
      case 'create_mask_from_selection': {
        if (!selection) throw new Error('Make a selection before replaying Mask from selection.');
        if (selection.width !== current.canvasWidth || selection.height !== current.canvasHeight) {
          throw new Error('The selection must be mapped to the layer document canvas before replay.');
        }
        if (backend) {
          const result = await backend.mask(current, id!, selection);
          const layer = findLayer(current, id!)!;
          const width = layer.content.type === 'pixel' ? layer.content.width : current.canvasWidth;
          const height = layer.content.type === 'pixel' ? layer.content.height : current.canvasHeight;
          if (result.width !== width || result.height !== height ||
            result.snapshot.width !== width || result.snapshot.height !== height) {
            throw new Error('The layer mask worker returned unexpected dimensions.');
          }
          assertMask(result.snapshot);
          current = updateLayer(current, id!, (layer) => ({ ...layer, mask: {
            snapshot: result.snapshot, enabled: true, inverted: false
          } }));
        }
        break;
      }
      case 'merge_down': {
        const layer = findLayer(current, id!)!;
        if (layer.content.type !== 'pixel') throw new Error('Merge down needs a pixel layer.');
        const path = pathTo(current, id!)!;
        const position = path.at(-1)!;
        if (!position) throw new Error('There is no layer beneath this one to merge into.');
        const parent = parentOf(current, id!);
        const siblings = parent ? childrenOf(findLayer(current, parent)!) : current.layers;
        const below = siblings[position - 1];
        assertSubtreeUnlocked(current, below.id);
        const mergeProblem = mergeSafetyProblem(current, [below.id, id!]);
        if (mergeProblem) throw new Error(mergeProblem);
        const result = backend ? await backend.merge(current, [below.id, id!]) : {
          pixelId: `preflight${index}`, width: current.canvasWidth, height: current.canvasHeight
        };
        if (backend) assertPixels(result, current.canvasWidth, current.canvasHeight);
        const replacement = createPixelLayer(below.name, result.pixelId, result.width, result.height);
        current = insertLayer(removeLayer(removeLayer(current, id!), below.id), replacement, parent, position - 1);
        break;
      }
      case 'flatten': {
        if (eachLayer(current).some((layer) => layer.locked)) {
          throw new Error('Unlock the document layers before replaying Flatten.');
        }
        const result = backend ? await backend.flatten(current) : {
          pixelId: `preflight${index}`, width: current.canvasWidth, height: current.canvasHeight
        };
        if (backend) assertPixels(result, current.canvasWidth, current.canvasHeight);
        current = createDocument(current.canvasWidth, current.canvasHeight, [
          createPixelLayer('Background', result.pixelId, result.width, result.height)
        ]);
        break;
      }
      case 'export_composite':
        if (backend) await backend.export(current);
        break;
    }
    assertDocument(current);
  }
  return current;
}
