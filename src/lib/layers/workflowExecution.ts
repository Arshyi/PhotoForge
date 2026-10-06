import type { OperationCall, Origin, TransactionRequest, TransactionResult } from '../operations/types';
import type { MaskSnapshot } from '../selections/types';
import { validateLayerWorkflowSteps } from '../utils/workflows';
import type { LayerDocument } from './types';
import type { LayerWorkflowStep } from './workflow';

/**
 * What a workflow replay needs from the outside: the backend's transaction
 * engine, and the one step that is not an edit of the document.
 */
export interface LayerWorkflowBackend {
  transact(request: TransactionRequest): Promise<TransactionResult>;
  export(document: LayerDocument): Promise<unknown>;
}

/**
 * The registered operation a workflow step is.
 *
 * `export_composite` is the one step that is not an operation: it edits nothing,
 * it writes a file, so it has no place in a transaction and is run afterwards on
 * the document the transaction produced.
 */
export function stepToCall(step: LayerWorkflowStep): OperationCall | null {
  switch (step.type) {
    case 'select_layer':
      return { op: 'core.layer.select', params: { selector: step.selector } };
    case 'set_visibility':
      return { op: 'core.layer.set_visible', params: { selector: step.selector, visible: step.visible } };
    case 'set_opacity':
      return { op: 'core.layer.set_opacity', params: { selector: step.selector, opacity: step.opacity } };
    case 'set_blend_mode':
      return { op: 'core.layer.set_blend_mode', params: { selector: step.selector, blendMode: step.blendMode } };
    case 'create_adjustment_layer':
      return {
        op: 'core.layer.add_adjustment',
        params: { operation: step.operation, ...(step.name?.trim() ? { name: step.name.trim() } : {}) }
      };
    case 'apply_to_layer':
      return { op: 'core.layer.apply_edit', params: { selector: step.selector, operations: step.operations } };
    case 'create_mask_from_selection':
      return { op: 'core.layer.mask_from_selection', params: { selector: step.selector } };
    case 'merge_down':
      return { op: 'core.layer.merge_down', params: { selector: step.selector } };
    case 'flatten':
      return { op: 'core.document.flatten', params: {} };
    case 'export_composite':
      return null;
  }
}

/**
 * Replays a recorded or planned layer workflow as one transaction.
 *
 * There is no replay engine in this file. The steps become registered operations,
 * the backend runs them on a private copy of the document — every selector
 * resolved and every lock, kind and invariant checked before any pixel is made —
 * and either the whole list happened or none of it did. A plugin, a macro, a
 * planner and the Layers panel's own commands go through the same engine, which is
 * the point of it.
 *
 * `origin` says who is asking. A person's replay may act on a layer they locked
 * themselves; an unattended one may not, and a planner may only use the relative
 * selectors and the operations that are suggestions rather than decisions.
 *
 * The caller owns the operation lock and releases buffers a discarded result made.
 */
export async function executeLayerWorkflow(
  document: LayerDocument,
  steps: LayerWorkflowStep[],
  selection: MaskSnapshot | null,
  backend: LayerWorkflowBackend,
  origin: Origin = 'automation',
  label = 'Apply layer workflow'
): Promise<LayerDocument> {
  const problems = validateLayerWorkflowSteps(steps);
  if (problems.length) throw new Error(problems[0]);
  const exports = steps.flatMap((step, index) => (step.type === 'export_composite' ? [index] : []));
  if (exports.length > 1 || (exports.length === 1 && exports[0] !== steps.length - 1)) {
    throw new Error('Export composite must be the final step, and may appear only once.');
  }

  const calls = steps.flatMap((step) => {
    const call = stepToCall(step);
    return call ? [call] : [];
  });
  let result = document;
  if (calls.length) {
    const transaction = await backend.transact({ document, label, steps: calls, selection, origin });
    result = transaction.document;
  }
  if (exports.length) await backend.export(result);
  return result;
}
