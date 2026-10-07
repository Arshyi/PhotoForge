import { describe, expect, it } from 'vitest';
import { factValue } from './facts';
import { accepts, defaultValue, defaultValues, describeLocality, firstProblem, initialValues } from './params';
import { pluginReferenceKey } from './requirements';
import type { ParamDecl } from './types';
import { createAdjustmentLayer, createDocument, createGroupLayer, createPixelLayer } from '../layers/tree';
import type { EditOperation } from '../types/editor';
import { validateEditOperation } from '../utils/workflows';

const number: ParamDecl = { id: 'amount', title: 'Amount', type: 'number', min: 0, max: 1, default: 0.5 };
const integer: ParamDecl = { id: 'width', title: 'Width', type: 'integer', min: 0, max: 10, default: 4 };
const bool: ParamDecl = { id: 'invert', title: 'Invert', type: 'bool', default: true };
const choice: ParamDecl = { id: 'order', title: 'Order', type: 'choice', options: ['a', 'b', 'c'], default: 2 };

describe('plugin parameters', () => {
  it('start at their declared defaults, with booleans as 0 or 1', () => {
    expect(defaultValue(number)).toBe(0.5);
    expect(defaultValue(bool)).toBe(1);
    expect(defaultValues([number, integer, bool, choice])).toEqual({ amount: 0.5, width: 4, invert: 1, order: 2 });
    expect(defaultValues(undefined)).toEqual({});
  });

  it('accept exactly what the backend accepts', () => {
    expect(accepts(number, 0) && accepts(number, 1) && !accepts(number, 1.01) && !accepts(number, -0.01)).toBe(true);
    expect(accepts(integer, 3) && !accepts(integer, 3.5) && !accepts(integer, 11)).toBe(true);
    expect(accepts(bool, 0) && accepts(bool, 1) && !accepts(bool, 2) && !accepts(bool, 0.5)).toBe(true);
    expect(accepts(choice, 2) && !accepts(choice, 3) && !accepts(choice, -1) && !accepts(choice, 1.5)).toBe(true);
    for (const param of [number, integer, bool, choice]) {
      expect(accepts(param, Number.NaN)).toBe(false);
      expect(accepts(param, Number.POSITIVE_INFINITY)).toBe(false);
    }
  });

  it('start a dialog from remembered values only if they still fit', () => {
    expect(initialValues([number, integer], { amount: 0.9, width: 7 })).toEqual({ amount: 0.9, width: 7 });
    // The plugin was updated and its range moved: the old value is not offered.
    expect(initialValues([number, integer], { amount: 5, width: 7.5 })).toEqual({ amount: 0.5, width: 4 });
    expect(initialValues([number], { unknown: 1 })).toEqual({ amount: 0.5 });
  });

  it('name the first parameter that would be refused', () => {
    expect(firstProblem([number, integer], { amount: 0.2, width: 3 })).toBeNull();
    expect(firstProblem([number, integer], { amount: 2, width: 3 })).toMatch(/Amount/);
    expect(firstProblem([number, integer], { amount: 0.2 })).toMatch(/Width/);
  });

  it('describe how far a filter reaches in words', () => {
    expect(describeLocality({ kind: 'pointwise' })).toMatch(/each pixel on its own/i);
    expect(describeLocality({ kind: 'local', radius: 1 })).toBe('Looks up to 1 pixel around each pixel');
    expect(describeLocality({ kind: 'local', radius: 4 })).toBe('Looks up to 4 pixels around each pixel');
    expect(describeLocality({ kind: 'global' })).toMatch(/whole image/);
  });
});

describe('the facts a plugin panel may show', () => {
  const document = createDocument(
    320,
    200,
    [createPixelLayer('Sky', 'px1', 320, 200), createGroupLayer('Notes', [createPixelLayer('Inner', 'px2', 8, 8)])],
    'linear_srgb_f32'
  );
  const selected = { ...document, activeLayerId: document.layers[0].id };

  it('are computed from the document, and say so when there is none', () => {
    expect(factValue(null, 'layer_count')).toBe('No document open');
    expect(factValue(selected, 'layer_count')).toBe('3');
    expect(factValue(selected, 'pixel_layer_count')).toBe('2');
    expect(factValue(selected, 'canvas_width')).toBe('320 px');
    expect(factValue(selected, 'canvas_height')).toBe('200 px');
    expect(factValue(selected, 'precision')).toBe('Linear float');
    expect(factValue({ ...selected, precision: 'legacy_srgb8' }, 'precision')).toBe('Legacy 8-bit sRGB');
    expect(factValue(selected, 'active_layer_name')).toBe('Sky');
    expect(factValue(selected, 'active_layer_kind')).toBe('Pixel');
    expect(factValue(selected, 'active_layer_opacity')).toBe('100%');
    expect(factValue({ ...selected, activeLayerId: null }, 'active_layer_name')).toBe('None selected');
  });
});

function pluginOperation(plugin: string, sha = 'b'.repeat(64)): EditOperation {
  return {
    type: 'plugin_filter', plugin, version: '1.0.0', sha256: sha, filter: 'solarize',
    locality: { kind: 'pointwise' }, parameters: [0.5]
  };
}

describe('knowing when a document needs plugins', () => {
  it('is empty for a document that uses none, so nothing is asked of the backend', () => {
    const document = createDocument(8, 8, [createPixelLayer('A', 'px', 8, 8)]);
    expect(pluginReferenceKey(document, [])).toBe('');
    expect(pluginReferenceKey(null, [{ type: 'grayscale' }])).toBe('');
  });

  it('names every plugin version in layers, groups, masks and the operation list', () => {
    const adjustment = (op: EditOperation) => createAdjustmentLayer('Adj', op as never);
    const masked: EditOperation = {
      type: 'masked', operation: pluginOperation('com.example.masked') as never,
      mask: { version: 1, width: 1, height: 1, encoding: 'base64_u8', data: 'AA', checksum: '' } as never,
      invert: false, mask_id: null
    };
    const document = createDocument(8, 8, [
      createPixelLayer('A', 'px', 8, 8),
      adjustment(pluginOperation('com.example.one')),
      createGroupLayer('G', [adjustment(pluginOperation('com.example.two')), adjustment(masked)]),
      adjustment({ type: 'grayscale' })
    ]);
    const key = pluginReferenceKey(document, [pluginOperation('com.example.three')]);
    for (const id of ['one', 'two', 'masked', 'three']) expect(key).toContain(`com.example.${id}@1.0.0#`);
    // The same plugin used twice is one requirement.
    const twice = createDocument(8, 8, [adjustment(pluginOperation('com.example.one')), adjustment(pluginOperation('com.example.one'))]);
    expect(pluginReferenceKey(twice, []).split('|')).toHaveLength(1);
  });

  it('changes when the version a layer names changes, so the question is asked again', () => {
    const make = (sha: string) =>
      createDocument(8, 8, [createAdjustmentLayer('A', pluginOperation('com.example.one', sha) as never)]);
    expect(pluginReferenceKey(make('a'.repeat(64)), [])).not.toBe(pluginReferenceKey(make('c'.repeat(64)), []));
  });
});

describe('the form of a plugin operation', () => {
  const good = pluginOperation('com.example.one');
  it('is accepted when well formed', () => {
    expect(validateEditOperation(good)).toBeNull();
    expect(validateEditOperation({ ...good, locality: { kind: 'local', radius: 4 } })).toBeNull();
    expect(validateEditOperation({ ...good, locality: { kind: 'global' } })).toBeNull();
    expect(validateEditOperation({ ...good, parameters: [] })).toBeNull();
  });

  it('is refused when any part is not', () => {
    const bad: Record<string, unknown>[] = [
      { plugin: 'core.layer.x' }, { plugin: 'UPPER.case' }, { plugin: 'nodots' }, { plugin: 'a..b' },
      { version: '1.0' }, { version: 'one.two.three' },
      { sha256: 'short' }, { sha256: 'G'.repeat(64) }, { sha256: 'A'.repeat(64) },
      { filter: 'Has Space' }, { filter: '' }, { filter: '1abc' },
      { locality: { kind: 'local', radius: 0 } }, { locality: { kind: 'local', radius: 257 } },
      { locality: { kind: 'bogus' } }, { locality: null },
      { parameters: [Number.NaN] }, { parameters: [Number.POSITIVE_INFINITY] }, { parameters: ['x'] },
      { parameters: Array.from({ length: 33 }, () => 0) }, { parameters: 'nope' }
    ];
    for (const change of bad) {
      expect(validateEditOperation({ ...good, ...change }), JSON.stringify(change)).not.toBeNull();
    }
  });
});
