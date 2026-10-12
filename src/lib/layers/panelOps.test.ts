import { describe, expect, it } from 'vitest';
import { existsSync, readFileSync, writeFileSync } from 'node:fs';
import { resolve } from 'node:path';
import registry from '../../../src-tauri/tests/fixtures/operation_registry.json';
import type { BaseEditOperation } from '../types/editor';
import type { OperationSpec } from '../operations/types';
import { createAdjustmentLayer, createDocument, createGroupLayer, createPixelLayer } from './tree';
import type { Layer, LayerDocument, LayerPixelsResult } from './types';
import * as panel from './panelOps';

const specs = registry as unknown as OperationSpec[];
const spec = (op: string) => specs.find((candidate) => candidate.id === op);

const pixel = createPixelLayer('Sky', 'px1', 8, 8);
const group = createGroupLayer('Group', [createPixelLayer('Leaf', 'px2', 4, 4)]);
const masked: Layer = {
  ...pixel,
  mask: { snapshot: {} as never, enabled: true, inverted: false }
};
const contrast = { type: 'contrast', amount: 0.25 } as BaseEditOperation;
const placed: LayerPixelsResult = { pixelId: 'px9', width: 4, height: 3, filename: 'a.png' };

/** One plan for every way the panel can ask the engine to do something. */
const everyPlan: Array<[string, panel.PanelPlan]> = [
  ['hide', panel.toggleLayerField(pixel, 'visible')],
  ['lock', panel.toggleLayerField(pixel, 'locked')],
  ['collapse', panel.toggleLayerField(group, 'collapsed')],
  ['rename', panel.renameLayer(pixel.id, 'Clouds')],
  ['opacity', panel.setOpacity(pixel.id, 0.4)],
  ['blend', panel.setBlendMode(pixel.id, 'multiply')],
  ['move to the top level', panel.moveLayer(pixel.id, null, 0)],
  ['move into a group', panel.moveLayer(pixel.id, group.id, 1)],
  ['new group', panel.addGroup()],
  ['new pixel layer', panel.addPixelLayer()],
  ['place an image', panel.placeImage(placed)],
  ['place a camera file', panel.placeImage({ ...placed, filename: null, raw: { reference: { filename: 'a.dng' } } as never })],
  ['new adjustment', panel.addAdjustment(contrast, 'Punch')],
  ['change an adjustment', panel.updateAdjustment(pixel.id, contrast, 'adjustment:contrast:amount')],
  ['duplicate', panel.duplicateLayer(pixel.id)],
  ['delete', panel.deleteLayer(pixel.id)],
  ['group one', panel.groupLayers([pixel.id])],
  ['group several', panel.groupLayers([pixel.id, group.id])],
  ['ungroup', panel.ungroupLayer(group.id)],
  ['reset transform', panel.resetTransform(pixel.id)],
  ['pass through', panel.togglePassThrough(group)],
  ['clip', panel.toggleClipped(pixel)],
  ['release clipping', panel.toggleClipped({ ...pixel, clipped: true })],
  ['reveal mask', panel.addMask(pixel.id, true)],
  ['hide mask', panel.addMask(pixel.id, false)],
  ['disable mask', panel.toggleMask(masked)],
  ['invert mask', panel.invertMask(masked)],
  ['delete mask', panel.removeMask(pixel.id)]
];

describe('every panel action asks for operations the registry has', () => {
  it.each(everyPlan)('%s', (_name, plan) => {
    expect(plan.label.trim().length).toBeGreaterThan(0);
    expect(plan.steps.length).toBeGreaterThan(0);
    for (const step of plan.steps) {
      const known = spec(step.op);
      expect(known, `${step.op} is not in the registry snapshot`).toBeTruthy();
      const given = Object.keys(step.params ?? {});
      const allowed = known!.params.map((param) => param.name);
      // The engine refuses a parameter it does not list, so a misspelling fails here
      // rather than at a person's first click.
      expect(given.filter((name) => !allowed.includes(name)), `${step.op} given an unknown parameter`).toEqual([]);
      const required = known!.params.filter((param) => param.required).map((param) => param.name);
      expect(required.filter((name) => !given.includes(name)), `${step.op} is missing a parameter`).toEqual([]);
    }
  });

  it('never asks a macro-visible operation to do what only a person may', () => {
    const userOnly = specs.filter((candidate) => candidate.userOnly).map((candidate) => candidate.id);
    expect(userOnly).toEqual(['core.layer.place_pixels']);
    for (const [, plan] of everyPlan) {
      for (const step of plan.steps) {
        if (userOnly.includes(step.op)) expect(plan.label).toBe('Place image as layer');
      }
    }
  });
});

describe('what each action sends', () => {
  it('switches a flag to the opposite of what the layer has now', () => {
    expect(panel.toggleLayerField(pixel, 'visible').steps[0].params).toEqual({ selector: { type: 'id', id: pixel.id }, visible: false });
    expect(panel.toggleLayerField({ ...pixel, visible: false }, 'visible').steps[0].params).toMatchObject({ visible: true });
    expect(panel.toggleLayerField({ ...pixel, locked: true }, 'locked').steps[0].params).toMatchObject({ locked: false });
    expect(panel.toggleLayerField({ ...group, collapsed: true }, 'collapsed').steps[0].params).toMatchObject({ collapsed: false });
  });

  it('keeps the labels the Undo menu has always shown', () => {
    expect(panel.toggleLayerField(pixel, 'visible').label).toBe('Layer visibility');
    expect(panel.toggleLayerField(pixel, 'locked').label).toBe('Lock layer');
    expect(panel.toggleLayerField(group, 'collapsed').label).toBe('Collapse group');
    expect(panel.setOpacity(pixel.id, 0.5).label).toBe('Layer opacity');
    expect(panel.moveLayer(pixel.id, null, 0).label).toBe('Reorder layer');
    expect(panel.groupLayers([pixel.id]).label).toBe('Group layer');
    expect(panel.groupLayers([pixel.id, group.id]).label).toBe('Group layers');
  });

  it('folds a drag into one undo step and a click into its own', () => {
    expect(panel.setOpacity('a', 0.2).coalesceKey).toBe('layer-opacity:a');
    expect(panel.setOpacity('b', 0.2).coalesceKey).toBe('layer-opacity:b');
    expect(panel.moveLayer('a', null, 0).coalesceKey).toBe('layer-move:a');
    expect(panel.updateAdjustment('a', contrast, 'adjustment:contrast:amount').coalesceKey).toBe('adjustment:contrast:amount');
    for (const plan of [panel.toggleLayerField(pixel, 'visible'), panel.duplicateLayer('a'), panel.deleteLayer('a')]) {
      expect(plan.coalesceKey).toBeUndefined();
    }
  });

  it('names no parent for a move to the top level, and one for a move into a group', () => {
    expect(panel.moveLayer('a', null, 2).steps[0].params).toEqual({ selector: { type: 'id', id: 'a' }, index: 2 });
    expect(panel.moveLayer('a', 'g', 1).steps[0].params).toEqual({
      selector: { type: 'id', id: 'a' },
      parent: { type: 'id', id: 'g' },
      index: 1
    });
  });

  it('switches pass-through to the opposite of what the group has now', () => {
    expect(panel.togglePassThrough(group)).toMatchObject({
      label: 'Pass-through group',
      steps: [{ params: { passThrough: true } }]
    });
    const passing: Layer = { ...group, content: { type: 'group', children: [], isolated: false } };
    expect(panel.togglePassThrough(passing)).toMatchObject({
      label: 'Isolate group',
      steps: [{ params: { passThrough: false } }]
    });
  });

  it('switches clipping to the opposite of what the layer has now', () => {
    expect(panel.toggleClipped(pixel)).toMatchObject({
      label: 'Clip to layer below',
      steps: [{ op: 'core.layer.set_clipped', params: { selector: { type: 'id', id: pixel.id }, clipped: true } }]
    });
    expect(panel.toggleClipped({ ...pixel, clipped: true })).toMatchObject({
      label: 'Release clipping',
      steps: [{ params: { clipped: false } }]
    });
    expect(panel.toggleClipped(pixel).coalesceKey).toBeUndefined();
  });

  it('switches a mask to the opposite of what it has now', () => {
    expect(panel.toggleMask(masked)).toMatchObject({ label: 'Disable mask', steps: [{ params: { enabled: false } }] });
    const off: Layer = { ...masked, mask: { ...masked.mask!, enabled: false, inverted: true } };
    expect(panel.toggleMask(off)).toMatchObject({ label: 'Enable mask', steps: [{ params: { enabled: true } }] });
    expect(panel.invertMask(masked).steps[0].params).toMatchObject({ inverted: true });
    expect(panel.invertMask(off).steps[0].params).toMatchObject({ inverted: false });
  });

  it('places an image by the buffer the backend already holds, with its provenance', () => {
    expect(panel.placeImage(placed).steps[0].params).toEqual({
      pixelId: 'px9', width: 4, height: 3, name: 'a.png'
    });
    expect(panel.placeImage({ ...placed, filename: null }).steps[0].params).toMatchObject({ name: 'Placed image' });
    const origin = { path: 'C:/x.png' } as never;
    expect(panel.placeImage({ ...placed, origin }).steps[0].params).toMatchObject({ origin });
  });
});

// ---- the plans, run on the real engine ------------------------------------------------------------
//
// The tests above hold the panel to the registry's *shape*. These hold it to the engine's *behaviour*:
// every plan the panel can send, on a document with every kind of layer it can be pointed at, is
// written to `src-tauri/tests/fixtures/panel_plans.json`, and `tests/operations_panel.rs` runs each of
// them through the real transaction engine as the person at the keyboard. A parameter that is the
// wrong type, a blend mode spelt the wrong way, a selector that resolves to nothing: each fails there,
// not at somebody's first click.
//
// Regenerate after an intentional change with `UPDATE_FIXTURES=1 npx vitest run src/lib/layers/panelOps.test.ts`.

const FIXTURE = resolve(__dirname, '../../../src-tauri/tests/fixtures/panel_plans.json');
const STAMP = '2026-01-01T00:00:00.000Z';
const NOW = new Date(STAMP);

function fixed<T extends Layer>(layer: T, id: string): T {
  return { ...layer, id, name: id, metadata: { createdAt: STAMP, modifiedAt: STAMP, custom: {} } };
}

function engineDocument(): LayerDocument {
  const group = fixed(createGroupLayer('g', [fixed(createPixelLayer('leaf', 'px-leaf', 4, 4, NOW), 'leaf')], NOW), 'g');
  const masked: Layer = {
    ...fixed(createPixelLayer('m', 'px-m', 8, 8, NOW), 'm'),
    mask: { snapshot: { version: 1, width: 8, height: 8, encoding: 'placeholder', data: '', checksum: '' }, enabled: true, inverted: false }
  };
  const adjustment = fixed(createAdjustmentLayer('adj', { type: 'contrast', amount: 0.25 } as BaseEditOperation, NOW), 'adj');
  const base = createDocument(8, 8, [fixed(createPixelLayer('a', 'px-a', 8, 8, NOW), 'a'), group, masked, adjustment], 'legacy_srgb8');
  return { ...base, activeLayerId: 'a' };
}

describe('the plans are accepted by the real engine', () => {
  const doc = engineDocument();
  const layer = (id: string) => doc.layers.find((candidate) => candidate.id === id)!;
  const brightness = { type: 'brightness', amount: 0.1 } as BaseEditOperation;

  /** Every distinct plan the panel can send, with a document it is valid on. */
  const entries = [
    ['hide', panel.toggleLayerField(layer('a'), 'visible')],
    ['lock', panel.toggleLayerField(layer('a'), 'locked')],
    ['collapse', panel.toggleLayerField(layer('g'), 'collapsed')],
    ['rename', panel.renameLayer('a', 'Clouds')],
    ['opacity', panel.setOpacity('a', 0.35)],
    ['opacity to zero', panel.setOpacity('a', 0)],
    ['blend', panel.setBlendMode('a', 'multiply')],
    ['blend soft light', panel.setBlendMode('a', 'soft_light')],
    ['move to the bottom', panel.moveLayer('m', null, 0)],
    ['move into a group', panel.moveLayer('a', 'g', 1)],
    ['move out of a group', panel.moveLayer('leaf', null, 0)],
    ['new group', panel.addGroup()],
    ['new pixel layer', panel.addPixelLayer()],
    ['new adjustment', panel.addAdjustment(brightness, 'Lift')],
    ['change an adjustment', panel.updateAdjustment('adj', brightness, 'adjustment:brightness:amount')],
    ['duplicate', panel.duplicateLayer('a')],
    ['duplicate a group', panel.duplicateLayer('g')],
    ['delete', panel.deleteLayer('a')],
    ['group one', panel.groupLayers(['a'])],
    ['group several', panel.groupLayers(['a', 'g'])],
    ['ungroup', panel.ungroupLayer('g')],
    ['reset transform', panel.resetTransform('a')],
    ['pass through', panel.togglePassThrough(layer('g'))],
    ['clip a layer', panel.toggleClipped(layer('m'))],
    ['reveal mask', panel.addMask('a', true)],
    ['hide mask', panel.addMask('a', false)],
    ['disable mask', panel.toggleMask(layer('m'))],
    ['invert mask', panel.invertMask(layer('m'))],
    ['delete mask', panel.removeMask('m')],
    ['place an image', panel.placeImage({ pixelId: 'px-placed', width: 4, height: 3, filename: 'photo.png' })]
  ] as const;

  const snapshot = entries.map(([name, plan]) => ({ name, document: doc, label: plan.label, steps: plan.steps }));

  it('covers every operation the panel issues', () => {
    const used = new Set(entries.flatMap(([, plan]) => plan.steps.map((step) => step.op)));
    expect([...used].sort()).toEqual([
      'core.layer.add_adjustment', 'core.layer.add_group', 'core.layer.add_pixel', 'core.layer.delete',
      'core.layer.duplicate', 'core.layer.group', 'core.layer.mask_add', 'core.layer.mask_remove',
      'core.layer.mask_set_enabled', 'core.layer.mask_set_inverted', 'core.layer.move', 'core.layer.place_pixels',
      'core.layer.rename', 'core.layer.reset_transform', 'core.layer.set_blend_mode', 'core.layer.set_clipped',
      'core.layer.set_collapsed', 'core.layer.set_locked', 'core.layer.set_opacity', 'core.layer.set_pass_through', 'core.layer.set_visible',
      'core.layer.ungroup', 'core.layer.update_adjustment'
    ]);
  });

  it('is exactly what the Rust test runs through the engine', () => {
    const text = JSON.stringify(snapshot, null, 2) + '\n';
    if (process.env.UPDATE_FIXTURES) writeFileSync(FIXTURE, text);
    expect(existsSync(FIXTURE), 'run with UPDATE_FIXTURES=1 to create the fixture').toBe(true);
    expect(JSON.parse(readFileSync(FIXTURE, 'utf8'))).toEqual(JSON.parse(text));
  });
});
