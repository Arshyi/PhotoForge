import { describe, expect, it } from 'vitest';
import {
  LayerWorkflowError,
  MAX_LAYER_WORKFLOW_STEPS,
  planLayerWorkflow,
  resolveSelector,
  runLayerWorkflow,
  selectorDescription,
  describeLayerSteps,
  type LayerWorkflowStep
} from './workflow';
import { createDocument, createGroupLayer, createPixelLayer, findLayer } from './tree';
import type { Layer, LayerDocument } from './types';

function pixel(name: string): Layer {
  return createPixelLayer(name, `px${name}`, 16, 16);
}

function documentWith(): { document: LayerDocument; bottom: Layer; top: Layer; group: Layer } {
  const bottom = pixel('Bottom');
  const group = createGroupLayer('Group', [pixel('Child')]);
  const top = pixel('Top');
  const document = {
    ...createDocument(16, 16, [bottom, group, top]),
    activeLayerId: top.id
  };
  return { document, bottom, top, group };
}

describe('selector resolution', () => {
  it('resolves every selector kind', () => {
    const { document, bottom, top } = documentWith();
    expect(resolveSelector(document, { type: 'id', id: bottom.id }, null)).toBe(bottom.id);
    expect(resolveSelector(document, { type: 'active' }, null)).toBe(top.id);
    expect(resolveSelector(document, { type: 'bottom' }, null)).toBe(bottom.id);
    expect(resolveSelector(document, { type: 'top' }, null)).toBe(top.id);
    expect(resolveSelector(document, { type: 'name', name: 'Bottom' }, null)).toBe(bottom.id);
    expect(resolveSelector(document, { type: 'last_created' }, top.id)).toBe(top.id);
  });

  it('fails closed for a layer the document does not have', () => {
    const { document } = documentWith();
    expect(() => resolveSelector(document, { type: 'id', id: 'ghost' }, null)).toThrow(
      LayerWorkflowError
    );
    expect(() => resolveSelector(document, { type: 'name', name: 'Nope' }, null)).toThrow(
      /does not have/
    );
  });

  it('refuses an ambiguous name rather than picking one', () => {
    const { document } = documentWith();
    const duplicated = {
      ...document,
      layers: [...document.layers, { ...pixel('Bottom') }]
    };
    expect(() => resolveSelector(duplicated, { type: 'name', name: 'Bottom' }, null)).toThrow(
      /unambiguous/
    );
  });

  it('fails closed when there is no active layer', () => {
    const { document } = documentWith();
    expect(() =>
      resolveSelector({ ...document, activeLayerId: null }, { type: 'active' }, null)
    ).toThrow(LayerWorkflowError);
  });

  it('describes each selector for the failure message', () => {
    expect(selectorDescription({ type: 'active' })).toBe('the selected layer');
    expect(selectorDescription({ type: 'id', id: 'abc' })).toBe('layer abc');
    expect(selectorDescription({ type: 'name', name: 'Sky' })).toBe('a layer named Sky');
  });
});

describe('running a layer workflow', () => {
  it('creates an adjustment layer and then targets it', () => {
    const { document } = documentWith();
    const steps: LayerWorkflowStep[] = [
      { type: 'create_adjustment_layer', operation: { type: 'grayscale' }, name: 'Mono' },
      { type: 'set_opacity', selector: { type: 'last_created' }, opacity: 0.6 }
    ];
    const result = runLayerWorkflow(document, steps);
    const created = result.document.layers.at(-1);
    expect(created?.name).toBe('Mono');
    expect(created?.content.type).toBe('adjustment');
    expect(created?.opacity).toBeCloseTo(0.6);
    expect(result.labels).toEqual(['New adjustment layer', 'Layer opacity']);
  });

  it('sets visibility and blend mode on a named layer', () => {
    const { document, bottom } = documentWith();
    const result = runLayerWorkflow(document, [
      { type: 'set_visibility', selector: { type: 'name', name: 'Bottom' }, visible: false },
      { type: 'set_blend_mode', selector: { type: 'bottom' }, blendMode: 'multiply' }
    ]);
    const updated = findLayer(result.document, bottom.id);
    expect(updated?.visible).toBe(false);
    expect(updated?.blendMode).toBe('multiply');
  });

  it('selects a layer without changing anything else', () => {
    const { document, bottom } = documentWith();
    const result = runLayerWorkflow(document, [
      { type: 'select_layer', selector: { type: 'id', id: bottom.id } }
    ]);
    expect(result.document.activeLayerId).toBe(bottom.id);
    expect(result.document.layers).toEqual(document.layers);
  });

  it('defers the steps that need backend pixel work, in order', () => {
    const { document, top } = documentWith();
    const result = runLayerWorkflow(document, [
      { type: 'apply_to_layer', selector: { type: 'top' }, operations: [{ type: 'grayscale' }] },
      { type: 'create_mask_from_selection', selector: { type: 'top' } },
      { type: 'flatten' },
      { type: 'export_composite' }
    ]);
    expect(result.deferred).toEqual([
      { type: 'apply_to_layer', layerId: top.id, operations: [{ type: 'grayscale' }] },
      { type: 'create_mask_from_selection', layerId: top.id },
      { type: 'flatten' },
      { type: 'export_composite' }
    ]);
  });

  it('leaves the original document untouched', () => {
    const { document } = documentWith();
    const snapshot = JSON.stringify(document);
    runLayerWorkflow(document, [
      { type: 'set_opacity', selector: { type: 'top' }, opacity: 0.2 }
    ]);
    expect(JSON.stringify(document)).toBe(snapshot);
  });

  it('throws before applying anything when a later step cannot resolve', () => {
    const { document } = documentWith();
    const steps: LayerWorkflowStep[] = [
      { type: 'set_opacity', selector: { type: 'top' }, opacity: 0.5 },
      { type: 'set_opacity', selector: { type: 'id', id: 'ghost' }, opacity: 0.5 }
    ];
    expect(() => runLayerWorkflow(document, steps)).toThrow(LayerWorkflowError);
    // The caller keeps its own document, so nothing was half-applied.
    expect(findLayer(document, document.activeLayerId as string)?.opacity).toBe(1);
  });

  it('rejects a step that needs a pixel layer but got a group', () => {
    const { document, group } = documentWith();
    expect(() =>
      runLayerWorkflow(document, [
        {
          type: 'apply_to_layer',
          selector: { type: 'id', id: group.id },
          operations: [{ type: 'grayscale' }]
        }
      ])
    ).toThrow(/needs a pixel layer/);
  });

  it('rejects merging down the bottom layer', () => {
    const { document, bottom } = documentWith();
    expect(() =>
      runLayerWorkflow(document, [
        { type: 'merge_down', selector: { type: 'id', id: bottom.id } }
      ])
    ).toThrow(/no layer beneath/);
  });

  it('rejects a blend-dependent merge that would omit its backdrop', () => {
    const bottom = pixel('Bottom');
    const middle = pixel('Middle');
    middle.blendMode = 'screen';
    const top = pixel('Top');
    const document = { ...createDocument(16, 16, [bottom, middle, top]), activeLayerId: top.id };
    expect(() => runLayerWorkflow(document, [
      { type: 'merge_down', selector: { type: 'active' } }
    ])).toThrow(/depends on layers beneath/);
    expect(planLayerWorkflow(document, [
      { type: 'merge_down', selector: { type: 'active' } }
    ])).toEqual({ ok: false, problem: expect.stringMatching(/depends on layers beneath/) });
  });

  it('rejects an out-of-range opacity and an empty operation list', () => {
    const { document } = documentWith();
    expect(() =>
      runLayerWorkflow(document, [{ type: 'set_opacity', selector: { type: 'top' }, opacity: 5 }])
    ).toThrow(/between 0 and 1/);
    expect(() =>
      runLayerWorkflow(document, [
        { type: 'apply_to_layer', selector: { type: 'top' }, operations: [] }
      ])
    ).toThrow(/at least one operation/);
  });

  it('rejects more steps than the documented ceiling', () => {
    const { document } = documentWith();
    const steps: LayerWorkflowStep[] = Array.from(
      { length: MAX_LAYER_WORKFLOW_STEPS + 1 },
      () => ({ type: 'flatten' })
    );
    expect(() => runLayerWorkflow(document, steps)).toThrow(/at most/);
  });

  it('runs an empty workflow as a no-op', () => {
    const { document } = documentWith();
    const result = runLayerWorkflow(document, []);
    expect(result.document).toBe(document);
    expect(result.deferred).toEqual([]);
  });
});

describe('planning without applying', () => {
  it('reports success for a workflow that fits the document', () => {
    const { document } = documentWith();
    expect(
      planLayerWorkflow(document, [
        { type: 'set_opacity', selector: { type: 'top' }, opacity: 0.5 }
      ])
    ).toEqual({ ok: true });
  });

  it('reports the problem for a workflow that does not', () => {
    const { document } = documentWith();
    const result = planLayerWorkflow(document, [
      { type: 'set_opacity', selector: { type: 'id', id: 'ghost' }, opacity: 0.5 }
    ]);
    expect(result.ok).toBe(false);
    if (!result.ok) expect(result.problem).toMatch(/does not have/);
  });
});

describe('describing steps', () => {
  it('produces a readable line per step', () => {
    expect(
      describeLayerSteps([
        { type: 'create_adjustment_layer', operation: { type: 'sepia' }, name: 'Warm' },
        { type: 'set_opacity', selector: { type: 'last_created' }, opacity: 0.4 },
        { type: 'flatten' }
      ])
    ).toEqual([
      'New adjustment layer — Warm',
      'Layer opacity — the layer created by an earlier step',
      'Flatten image'
    ]);
  });
});
