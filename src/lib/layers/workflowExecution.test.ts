import { describe, expect, it, vi } from 'vitest';
import { executeLayerWorkflow, type LayerWorkflowBackend } from './workflowExecution';
import { createDocument, createGroupLayer, createPixelLayer } from './tree';
import { decodedCoverageChecksum } from '../selections/checksum';
import type { MaskSnapshot } from '../selections/types';

function fixture() {
  const bottom = createPixelLayer('Bottom', 'px1', 8, 8);
  const top = createPixelLayer('Top', 'px2', 8, 8);
  return createDocument(8, 8, [bottom, top]);
}

function backend(): LayerWorkflowBackend {
  return {
    validate: vi.fn(async () => undefined),
    apply: vi.fn(async () => ({ pixelId: 'applied', width: 8, height: 8, filename: null })),
    mask: vi.fn(),
    merge: vi.fn(async () => ({ pixelId: 'merged', width: 8, height: 8, filename: null })),
    flatten: vi.fn(async () => ({ pixelId: 'flat', width: 8, height: 8, filename: null })),
    export: vi.fn(async () => undefined)
  };
}

function selection(value = 255): MaskSnapshot {
  const snapshot = {
    version: 1, width: 8, height: 8, encoding: 'base64_u8',
    data: btoa(String.fromCharCode(...Array(64).fill(value))).replace(/=+$/, ''), checksum: ''
  };
  snapshot.checksum = decodedCoverageChecksum(snapshot)!;
  return snapshot;
}

function expectNoBackendCalls(api: LayerWorkflowBackend) {
  for (const method of Object.values(api)) expect(method).not.toHaveBeenCalled();
}

describe('transactional layer workflow execution', () => {
  it('merges before resolving the following active-layer operation', async () => {
    const source = fixture();
    const api = backend();
    const result = await executeLayerWorkflow(source, [
      { type: 'merge_down', selector: { type: 'active' } },
      { type: 'set_opacity', selector: { type: 'active' }, opacity: 0.4 },
      { type: 'apply_to_layer', selector: { type: 'active' }, operations: [{ type: 'grayscale' }] },
      { type: 'export_composite' }
    ], null, api);
    expect(source.layers).toHaveLength(2);
    expect(result.layers).toHaveLength(1);
    expect(result.layers[0].opacity).toBe(0.4);
    expect(result.layers[0].content).toMatchObject({ pixelId: 'applied' });
    expect(api.apply).toHaveBeenCalledWith(expect.objectContaining({ layers: [expect.objectContaining({ opacity: 0.4 })] }),
      result.layers[0].id, [{ type: 'grayscale' }]);
    expect(api.export).toHaveBeenCalledWith(result);
  });

  it('preflights references after flatten before any backend side effect', async () => {
    const source = fixture();
    const api = backend();
    await expect(executeLayerWorkflow(source, [
      { type: 'flatten' },
      { type: 'set_opacity', selector: { type: 'id', id: source.layers[1].id }, opacity: 0.2 }
    ], null, api)).rejects.toThrow(/does not have/);
    expect(api.flatten).not.toHaveBeenCalled();
    expect(api.validate).not.toHaveBeenCalled();
  });

  it('retains the original tree when a later worker fails', async () => {
    const source = fixture();
    const before = JSON.stringify(source);
    const api = backend();
    api.apply = vi.fn(async () => { throw new Error('worker failed'); });
    await expect(executeLayerWorkflow(source, [
      { type: 'set_opacity', selector: { type: 'top' }, opacity: 0.3 },
      { type: 'apply_to_layer', selector: { type: 'active' }, operations: [{ type: 'grayscale' }] }
    ], null, api)).rejects.toThrow('worker failed');
    expect(JSON.stringify(source)).toBe(before);
  });

  it('resolves last-created against the actual adjustment and validates its ID', async () => {
    const api = backend();
    const result = await executeLayerWorkflow(fixture(), [
      { type: 'create_adjustment_layer', name: 'Tone', operation: { type: 'grayscale' } },
      { type: 'set_opacity', selector: { type: 'last_created' }, opacity: 0.6 }
    ], null, api);
    expect(result.layers.at(-1)).toMatchObject({ name: 'Tone', opacity: 0.6 });
    expect(api.validate).toHaveBeenLastCalledWith(expect.anything(), [
      { type: 'set_opacity', selector: { type: 'id', id: result.layers.at(-1)!.id }, opacity: 0.6 }
    ]);
  });

  it('rejects missing selections, locked edits, and nonterminal exports up front', async () => {
    const api = backend();
    const source = fixture();
    await expect(executeLayerWorkflow(source, [{ type: 'create_mask_from_selection', selector: { type: 'top' } }], null, api))
      .rejects.toThrow(/selection/);
    source.layers[1].locked = true;
    await expect(executeLayerWorkflow(source, [{ type: 'flatten' }], null, api)).rejects.toThrow(/Unlock/);
    await expect(executeLayerWorkflow(source, [{ type: 'export_composite' }, { type: 'flatten' }], null, api))
      .rejects.toThrow(/final step/);
    expect(api.validate).not.toHaveBeenCalled();
  });

  it('attaches a new selection mask before merging the layer', async () => {
    const source = fixture(); const api = backend(); const snapshot = selection(128);
    api.mask = vi.fn(async () => ({ snapshot, width: 8, height: 8 }));
    const result = await executeLayerWorkflow(source, [
      { type: 'create_mask_from_selection', selector: { type: 'active' } },
      { type: 'merge_down', selector: { type: 'active' } }
    ], snapshot, api);
    expect(api.merge).toHaveBeenCalledWith(expect.objectContaining({ layers: [
      source.layers[0], expect.objectContaining({ mask: { snapshot, enabled: true, inverted: false } })
    ] }), [source.layers[0].id, source.layers[1].id]);
    expect(result.layers[0].content).toMatchObject({ pixelId: 'merged' });
    expect(source.layers[1].mask).toBeNull();
  });

  it('flattens updated pixel references from the preceding apply step', async () => {
    const source = fixture(); const api = backend();
    const result = await executeLayerWorkflow(source, [
      { type: 'apply_to_layer', selector: { type: 'top' }, operations: [{ type: 'grayscale' }] },
      { type: 'flatten' }
    ], null, api);
    expect(api.flatten).toHaveBeenCalledWith(expect.objectContaining({ layers: [
      source.layers[0], expect.objectContaining({ content: { type: 'pixel', pixelId: 'applied', width: 8, height: 8 } })
    ] }));
    expect(result.layers[0].content).toMatchObject({ pixelId: 'flat' });
    expect(source.layers[1].content).toMatchObject({ pixelId: 'px2' });
  });

  it('rejects later invalid references before any earlier apply, mask, or export backend call', async () => {
    const source = fixture(); const api = backend();
    await expect(executeLayerWorkflow(source, [
      { type: 'apply_to_layer', selector: { type: 'top' }, operations: [{ type: 'grayscale' }] },
      { type: 'create_mask_from_selection', selector: { type: 'top' } },
      { type: 'set_opacity', selector: { type: 'name', name: 'Missing' }, opacity: 0.3 },
      { type: 'export_composite' }
    ], selection(), api)).rejects.toThrow(/does not have/);
    expectNoBackendCalls(api);
  });

  it('does not discard locked descendants when merging into a group', async () => {
    const locked = createPixelLayer('Locked child', 'locked', 8, 8); locked.locked = true;
    const source = createDocument(8, 8, [
      createGroupLayer('Below', [locked]), createPixelLayer('Above', 'top', 8, 8)
    ]);
    const api = backend();
    await expect(executeLayerWorkflow(source, [
      { type: 'merge_down', selector: { type: 'active' } }
    ], null, api)).rejects.toThrow(/Unlock Locked child/);
    expectNoBackendCalls(api);
  });

  it('preflights a corrupt selection checksum before any backend calls', async () => {
    const api = backend();
    await expect(executeLayerWorkflow(fixture(), [
      { type: 'apply_to_layer', selector: { type: 'top' }, operations: [{ type: 'grayscale' }] },
      { type: 'create_mask_from_selection', selector: { type: 'top' } }
    ], { ...selection(), checksum: 'fnv1a64:0000000000000000' }, api)).rejects.toThrow(/checksum/);
    expectNoBackendCalls(api);
  });

  it('does not pass a malformed worker buffer to a later flatten', async () => {
    const source = fixture(); const api = backend();
    api.apply = vi.fn(async () => ({ pixelId: 'bad', width: 9, height: 8, filename: null }));
    await expect(executeLayerWorkflow(source, [
      { type: 'apply_to_layer', selector: { type: 'top' }, operations: [{ type: 'grayscale' }] },
      { type: 'flatten' }
    ], null, api)).rejects.toThrow(/unexpected dimensions/);
    expect(api.flatten).not.toHaveBeenCalled();
    expect(source.layers[1].content).toMatchObject({ pixelId: 'px2' });
  });

  it('fails closed when merge down would omit a blend-dependent backdrop', async () => {
    const bottom = createPixelLayer('Bottom', 'px1', 8, 8);
    const middle = createPixelLayer('Middle', 'px2', 8, 8);
    middle.blendMode = 'screen';
    const top = createPixelLayer('Top', 'px3', 8, 8);
    const source = createDocument(8, 8, [bottom, middle, top]);
    source.activeLayerId = top.id;
    const api = backend();
    await expect(executeLayerWorkflow(source, [
      { type: 'merge_down', selector: { type: 'active' } }
    ], null, api)).rejects.toThrow(/depends on layers beneath/);
    expectNoBackendCalls(api);
  });
});
