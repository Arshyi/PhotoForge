import { describe, expect, it, vi } from 'vitest';
import { executeLayerWorkflow, stepToCall, type LayerWorkflowBackend } from './workflowExecution';
import { createDocument, createPixelLayer } from './tree';
import type { LayerWorkflowStep } from './workflow';
import type { TransactionRequest, TransactionResult } from '../operations/types';
import registry from '../../../src-tauri/tests/fixtures/operation_registry.json';

/**
 * The replay engine now lives in the backend (`src-tauri/src/operations`), where
 * its behaviour — atomicity, rollback of pixel buffers, lock and planner rules,
 * the dry run, the ordering of merges and selections — is tested against the real
 * thing in `src-tauri/tests/operations_engine.rs`. What is left here is the part
 * that is TypeScript: turning steps into registered operations, sending one
 * transaction, and running the export afterwards.
 */

function fixture() {
  const bottom = createPixelLayer('Bottom', 'px1', 8, 8);
  const top = createPixelLayer('Top', 'px2', 8, 8);
  return createDocument(8, 8, [bottom, top]);
}

function backend(result?: Partial<TransactionResult>) {
  const transact = vi.fn(async (request: TransactionRequest): Promise<TransactionResult> => ({
    document: { ...request.document, activeLayerId: 'after' },
    revision: 'r'.repeat(64),
    label: request.label,
    steps: [],
    createdPixelIds: [],
    lastCreated: null,
    ...result
  }));
  const exported = vi.fn(async () => undefined);
  const api: LayerWorkflowBackend = { transact, export: exported };
  return { api, transact, exported };
}

const everyStep: LayerWorkflowStep[] = [
  { type: 'select_layer', selector: { type: 'top' } },
  { type: 'set_visibility', selector: { type: 'active' }, visible: false },
  { type: 'set_opacity', selector: { type: 'active' }, opacity: 0.5 },
  { type: 'set_blend_mode', selector: { type: 'active' }, blendMode: 'multiply' },
  { type: 'create_adjustment_layer', name: ' Tone ', operation: { type: 'grayscale' } },
  { type: 'apply_to_layer', selector: { type: 'last_created' }, operations: [{ type: 'grayscale' }] },
  { type: 'create_mask_from_selection', selector: { type: 'active' } },
  { type: 'merge_down', selector: { type: 'active' } },
  { type: 'flatten' }
];

describe('layer workflow replay through the operation engine', () => {
  it('turns every step that edits the document into an operation the registry has', () => {
    const known = new Set(registry.map((spec) => spec.id));
    const calls = everyStep.map((step) => stepToCall(step));
    for (const call of calls) {
      expect(call, 'every editing step is an operation').not.toBeNull();
      expect(known.has(call!.op), `${call!.op} is registered`).toBe(true);
    }
    // And no two steps share an operation by accident.
    expect(new Set(calls.map((call) => call!.op)).size).toBe(everyStep.length);
    // Export is the one step that is not an edit.
    expect(stepToCall({ type: 'export_composite' })).toBeNull();
  });

  it('names the right operation and parameters for each step', () => {
    expect(stepToCall(everyStep[1])).toEqual({
      op: 'core.layer.set_visible',
      params: { selector: { type: 'active' }, visible: false }
    });
    expect(stepToCall(everyStep[3])).toEqual({
      op: 'core.layer.set_blend_mode',
      params: { selector: { type: 'active' }, blendMode: 'multiply' }
    });
    // A name is trimmed; a blank one is left out so the backend's default applies.
    expect(stepToCall(everyStep[4])!.params).toEqual({ operation: { type: 'grayscale' }, name: 'Tone' });
    expect(
      stepToCall({ type: 'create_adjustment_layer', name: '  ', operation: { type: 'grayscale' } })!.params
    ).toEqual({ operation: { type: 'grayscale' } });
    expect(stepToCall({ type: 'create_adjustment_layer', operation: { type: 'grayscale' } })!.params).toEqual({
      operation: { type: 'grayscale' }
    });
  });

  it('sends the whole list as one transaction, who is asking, and the selection', async () => {
    const { api, transact } = backend();
    const source = fixture();
    const selection = { version: 1, width: 8, height: 8, encoding: 'base64_u8', data: '', checksum: '' } as never;
    const result = await executeLayerWorkflow(source, everyStep, selection, api, 'planner', 'Guided edit');
    expect(transact).toHaveBeenCalledTimes(1);
    const request = transact.mock.calls[0][0];
    expect(request.steps).toHaveLength(everyStep.length);
    expect(request.origin).toBe('planner');
    expect(request.label).toBe('Guided edit');
    expect(request.selection).toBe(selection);
    expect(request.document).toBe(source);
    // What comes back is the backend's document, untouched by this layer.
    expect(result.activeLayerId).toBe('after');
    expect(source.activeLayerId).not.toBe('after');
  });

  it('is an automation by default, and says so', async () => {
    const { api, transact } = backend();
    await executeLayerWorkflow(fixture(), [{ type: 'flatten' }], null, api);
    expect(transact.mock.calls[0][0].origin).toBe('automation');
    expect(transact.mock.calls[0][0].label).toBe('Apply layer workflow');
  });

  it('exports after the transaction, from the document it produced', async () => {
    const { api, transact, exported } = backend();
    const result = await executeLayerWorkflow(
      fixture(),
      [{ type: 'set_opacity', selector: { type: 'top' }, opacity: 0.4 }, { type: 'export_composite' }],
      null,
      api
    );
    expect(transact.mock.calls[0][0].steps).toHaveLength(1);
    expect(exported).toHaveBeenCalledExactlyOnceWith(result);
    expect(transact.mock.invocationCallOrder[0]).toBeLessThan(exported.mock.invocationCallOrder[0]);
  });

  it('does not open a transaction for a workflow that only exports', async () => {
    const { api, transact, exported } = backend();
    const source = fixture();
    const result = await executeLayerWorkflow(source, [{ type: 'export_composite' }], null, api);
    expect(transact).not.toHaveBeenCalled();
    expect(exported).toHaveBeenCalledExactlyOnceWith(source);
    expect(result).toBe(source);
  });

  it('refuses a misplaced or repeated export before anything is sent', async () => {
    const { api, transact, exported } = backend();
    await expect(
      executeLayerWorkflow(fixture(), [{ type: 'export_composite' }, { type: 'flatten' }], null, api)
    ).rejects.toThrow(/final step/);
    await expect(
      executeLayerWorkflow(fixture(), [{ type: 'export_composite' }, { type: 'export_composite' }], null, api)
    ).rejects.toThrow(/final step/);
    expect(transact).not.toHaveBeenCalled();
    expect(exported).not.toHaveBeenCalled();
  });

  it('refuses malformed steps before anything is sent', async () => {
    const { api, transact } = backend();
    await expect(
      executeLayerWorkflow(fixture(), [{ type: 'set_opacity', selector: { type: 'top' }, opacity: 4 }], null, api)
    ).rejects.toThrow();
    expect(transact).not.toHaveBeenCalled();
  });

  it('exports nothing and returns nothing when the transaction fails', async () => {
    const { api, exported } = backend();
    api.transact = vi.fn(async () => {
      throw new Error('Step 2 (core.layer.merge_down) failed. Nothing was changed.');
    });
    const source = fixture();
    const before = JSON.stringify(source);
    await expect(
      executeLayerWorkflow(source, [{ type: 'flatten' }, { type: 'export_composite' }], null, api)
    ).rejects.toThrow(/Nothing was changed/);
    expect(exported).not.toHaveBeenCalled();
    expect(JSON.stringify(source)).toBe(before);
  });
});
