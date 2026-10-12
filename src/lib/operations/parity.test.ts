/**
 * The Layers panel edits the layer tree with TypeScript functions for the
 * rapid-fire cases (a dragged opacity slider, a rename), and the operation engine
 * edits it in Rust. Until the panel goes through the engine for everything, those
 * are two implementations of one behaviour, and two implementations drift.
 *
 * This file is how they are kept honest. It builds a set of vectors — a document,
 * an operation, and the document the TypeScript function produces — and checks
 * them against `src-tauri/tests/fixtures/operations_parity.json`. The Rust test
 * `operations_parity` applies each operation through the real engine and checks it
 * against the same file. If the TypeScript behaviour changes, this test fails until
 * the file is regenerated, and the Rust test then fails until the engine agrees.
 *
 * Regenerate after an intentional change with `UPDATE_FIXTURES=1 npx vitest run
 * src/lib/operations/parity.test.ts`.
 */
import { describe, expect, it } from 'vitest';
import { existsSync, readFileSync, writeFileSync } from 'node:fs';
import { resolve } from 'node:path';
import {
  childrenOf,
  createAdjustmentLayer,
  createDocument,
  createGroupLayer,
  createPixelLayer,
  duplicateLayer,
  eachLayer,
  groupLayers,
  insertLayer,
  moveLayer,
  pathTo,
  removeLayer,
  ungroupLayer,
  updateLayer
} from '../layers/tree';
import { resetTransform } from '../layers/transformTool';
import { mergeSafetyProblem } from '../layers/mergeSafety';
import type { Layer, LayerDocument } from '../layers/types';

const FIXTURE = resolve(__dirname, '../../../src-tauri/tests/fixtures/operations_parity.json');
const MERGE_FIXTURE = resolve(__dirname, '../../../src-tauri/tests/fixtures/operations_merge_parity.json');
const STAMP = '2026-01-01T00:00:00.000Z';
const NOW = new Date(STAMP);

/** A layer with an identifier and stamps chosen here, not generated. */
function fixed<T extends Layer>(layer: T, id: string, name = id): T {
  return {
    ...layer,
    id,
    name,
    metadata: { createdAt: STAMP, modifiedAt: STAMP, custom: {} }
  };
}

function pixel(id: string): Layer {
  return fixed(createPixelLayer(id, `px-${id}`, 8, 8, NOW), id);
}

function group(id: string, children: Layer[], isolated = true): Layer {
  return fixed(createGroupLayer(id, children, NOW, isolated), id);
}

function document(layers: Layer[], active: string | null): LayerDocument {
  return { ...createDocument(8, 8, layers, 'legacy_srgb8'), activeLayerId: active };
}

/** Renames identifiers the operation made to #1, #2, ... in the order they appear. */
function normalise(doc: LayerDocument, before: Set<string>): unknown {
  const fresh = new Map<string, string>();
  for (const layer of eachLayer(doc)) {
    if (!before.has(layer.id) && !fresh.has(layer.id)) fresh.set(layer.id, `#${fresh.size + 1}`);
  }
  const rename = (id: string | null) => (id === null ? null : (fresh.get(id) ?? id));
  const strip = (value: unknown): unknown => {
    if (Array.isArray(value)) return value.map(strip);
    if (value && typeof value === 'object') {
      const out: Record<string, unknown> = {};
      for (const [key, inner] of Object.entries(value as Record<string, unknown>).sort(([a], [b]) => a.localeCompare(b))) {
        // Absent and null mean the same thing on the wire.
        if (inner === null || inner === undefined) continue;
        out[key] = strip(inner);
      }
      return out;
    }
    return value;
  };
  const walk = (layer: Layer): Layer => ({
    ...layer,
    id: rename(layer.id) as string,
    metadata: { createdAt: 'T', modifiedAt: 'T', custom: {} },
    content:
      layer.content.type === 'group'
        ? { ...layer.content, children: layer.content.children.map(walk) }
        : layer.content
  });
  return strip({ ...doc, layers: doc.layers.map(walk), activeLayerId: rename(doc.activeLayerId) });
}

const byId = (id: string) => ({ type: 'id', id });

interface Vector {
  name: string;
  document: LayerDocument;
  call: { op: string; params: Record<string, unknown> };
  expected: LayerDocument;
}

function vector(
  name: string,
  doc: LayerDocument,
  call: Vector['call'],
  apply: (doc: LayerDocument) => LayerDocument
): Vector {
  return { name, document: doc, call, expected: apply(doc) };
}

function build(): Vector[] {
  const flat = () => document([pixel('a'), pixel('b'), pixel('c')], 'c');
  const nested = () =>
    document([pixel('a'), group('g', [pixel('x'), pixel('y')]), pixel('c')], 'y');
  const tweak = (doc: LayerDocument, id: string, change: (layer: Layer) => Layer) =>
    updateLayer(doc, id, change, NOW);
  /** `c` and `d` clipped to `b`. */
  const clipped = () =>
    document(
      [pixel('a'), pixel('b'), { ...pixel('c'), clipped: true }, { ...pixel('d'), clipped: true }, pixel('e')],
      'd'
    );
  const unclip = (layer: Layer): Layer => {
    const { clipped: _clipped, ...rest } = layer;
    return rest;
  };

  return [
    vector('hide a layer', flat(), { op: 'core.layer.set_visible', params: { selector: byId('b'), visible: false } },
      (doc) => tweak(doc, 'b', (layer) => ({ ...layer, visible: false }))),
    vector('set opacity', flat(), { op: 'core.layer.set_opacity', params: { selector: byId('a'), opacity: 0.35 } },
      (doc) => tweak(doc, 'a', (layer) => ({ ...layer, opacity: 0.35 }))),
    vector('set blend mode', flat(), { op: 'core.layer.set_blend_mode', params: { selector: byId('c'), blendMode: 'screen' } },
      (doc) => tweak(doc, 'c', (layer) => ({ ...layer, blendMode: 'screen' }))),
    vector('rename', flat(), { op: 'core.layer.rename', params: { selector: byId('a'), name: 'Sky' } },
      (doc) => tweak(doc, 'a', (layer) => ({ ...layer, name: 'Sky' }))),
    vector('lock', flat(), { op: 'core.layer.set_locked', params: { selector: byId('b'), locked: true } },
      (doc) => tweak(doc, 'b', (layer) => ({ ...layer, locked: true }))),
    vector('collapse a group', nested(), { op: 'core.layer.set_collapsed', params: { selector: byId('g'), collapsed: true } },
      (doc) => tweak(doc, 'g', (layer) => ({ ...layer, collapsed: true }))),
    vector('clip a layer to the one below', flat(), { op: 'core.layer.set_clipped', params: { selector: byId('b'), clipped: true } },
      (doc) => tweak(doc, 'b', (layer) => ({ ...layer, clipped: true }))),
    vector('release a clipped layer', clipped(), { op: 'core.layer.set_clipped', params: { selector: byId('c'), clipped: false } },
      (doc) => tweak(doc, 'c', unclip)),
    vector('select', flat(), { op: 'core.layer.select', params: { selector: byId('a') } },
      (doc) => ({ ...doc, activeLayerId: 'a' })),
    vector('reset a transform, keeping the interpolation', (() => {
      const doc = flat();
      return tweak(doc, 'b', (layer) => ({
        ...layer,
        transform: { ...layer.transform, scaleX: 2, rotationDegrees: 30, translateX: 5, interpolation: 'nearest' }
      }));
    })(), { op: 'core.layer.reset_transform', params: { selector: byId('b') } },
    (doc) => tweak(doc, 'b', (layer) => ({ ...layer, transform: resetTransform(layer.transform.interpolation) }))),

    vector('move to the bottom', flat(), { op: 'core.layer.move', params: { selector: byId('c'), index: 0 } },
      (doc) => moveLayer(doc, 'c', null, 0)),
    vector('move up one', flat(), { op: 'core.layer.move', params: { selector: byId('a'), index: 1 } },
      (doc) => moveLayer(doc, 'a', null, 1)),
    vector('move into a group', nested(), { op: 'core.layer.move', params: { selector: byId('a'), parent: byId('g'), index: 1 } },
      (doc) => moveLayer(doc, 'a', 'g', 1)),
    vector('move out of a group', nested(), { op: 'core.layer.move', params: { selector: byId('x'), index: 0 } },
      (doc) => moveLayer(doc, 'x', null, 0)),

    vector('move a base with the layers clipped to it', clipped(), { op: 'core.layer.move', params: { selector: byId('b'), index: 0 } },
      (doc) => moveLayer(doc, 'b', null, 0)),
    vector('move a base up past a layer', clipped(), { op: 'core.layer.move', params: { selector: byId('b'), index: 2 } },
      (doc) => moveLayer(doc, 'b', null, 2)),
    vector('move a clipped layer alone', clipped(), { op: 'core.layer.move', params: { selector: byId('c'), index: 3 } },
      (doc) => moveLayer(doc, 'c', null, 3)),
    vector('move a clipped layer to the bottom, which releases it', clipped(), { op: 'core.layer.move', params: { selector: byId('c'), index: 0 } },
      (doc) => moveLayer(doc, 'c', null, 0)),
    vector('delete a base, which promotes the first layer clipped to it', clipped(), { op: 'core.layer.delete', params: { selector: byId('b') } },
      (doc) => removeLayer(doc, 'b')),
    vector('delete a clipped layer', clipped(), { op: 'core.layer.delete', params: { selector: byId('c') } },
      (doc) => removeLayer(doc, 'c')),
    vector('duplicate a base, above its clipping group', clipped(), { op: 'core.layer.duplicate', params: { selector: byId('b') } },
      (doc) => duplicateLayer(doc, 'b', NOW).document),
    vector('duplicate a clipped layer', clipped(), { op: 'core.layer.duplicate', params: { selector: byId('c') } },
      (doc) => duplicateLayer(doc, 'c', NOW).document),
    vector('group a whole clipping group', clipped(), { op: 'core.layer.group', params: { selectors: [byId('b'), byId('c'), byId('d')], name: 'Art' } },
      (doc) => groupLayers(doc, ['b', 'c', 'd'], 'Art', NOW).document),
    vector('group a clipped layer alone, which releases it', clipped(), { op: 'core.layer.group', params: { selectors: [byId('c')], name: 'Solo' } },
      (doc) => groupLayers(doc, ['c'], 'Solo', NOW).document),
    vector('delete a layer', flat(), { op: 'core.layer.delete', params: { selector: byId('b') } },
      (doc) => removeLayer(doc, 'b')),
    vector('delete the selected layer', flat(), { op: 'core.layer.delete', params: { selector: byId('c') } },
      (doc) => removeLayer(doc, 'c')),
    vector('delete a group that holds the selection', nested(), { op: 'core.layer.delete', params: { selector: byId('g') } },
      (doc) => removeLayer(doc, 'g')),

    vector('duplicate a layer', flat(), { op: 'core.layer.duplicate', params: { selector: byId('b') } },
      (doc) => duplicateLayer(doc, 'b', NOW).document),
    vector('duplicate a group', nested(), { op: 'core.layer.duplicate', params: { selector: byId('g') } },
      (doc) => duplicateLayer(doc, 'g', NOW).document),
    vector('duplicate a layer inside a group', nested(), { op: 'core.layer.duplicate', params: { selector: byId('x') } },
      (doc) => duplicateLayer(doc, 'x', NOW).document),

    vector('group two layers, named out of order', document([pixel('a'), pixel('b'), pixel('c'), pixel('d')], 'a'),
      { op: 'core.layer.group', params: { selectors: [byId('d'), byId('b')], name: 'Pair' } },
      (doc) => groupLayers(doc, ['d', 'b'], 'Pair', NOW).document),
    vector('group one layer', flat(), { op: 'core.layer.group', params: { selectors: [byId('b')], name: 'Solo' } },
      (doc) => groupLayers(doc, ['b'], 'Solo', NOW).document),
    vector('group inside a group', nested(), { op: 'core.layer.group', params: { selectors: [byId('x'), byId('y')], name: 'Inner' } },
      (doc) => groupLayers(doc, ['x', 'y'], 'Inner', NOW).document),

    vector('ungroup', nested(), { op: 'core.layer.ungroup', params: { selector: byId('g') } },
      (doc) => ungroupLayer(doc, 'g')),
    vector('ungroup an empty group', document([pixel('a'), group('g', []), pixel('c')], 'a'),
      { op: 'core.layer.ungroup', params: { selector: byId('g') } },
      (doc) => ungroupLayer(doc, 'g')),

    vector('add a group', flat(), { op: 'core.layer.add_group', params: { name: 'Notes' } },
      (doc) => insertLayer(doc, fixed(createGroupLayer('Notes', [], NOW), 'new', 'Notes'), null, doc.layers.length)),
    vector('add an adjustment', flat(), {
      op: 'core.layer.add_adjustment', params: { operation: { type: 'contrast', amount: 0.25 }, name: 'Punch' }
    }, (doc) =>
      insertLayer(doc, fixed(createAdjustmentLayer('Punch', { type: 'contrast', amount: 0.25 } as never, NOW), 'new', 'Punch'),
        null, doc.layers.length))
  ];
}

describe('the TypeScript tree functions and the Rust engine agree', () => {
  const vectors = build();

  /** What gets written: each vector with both documents normalised the same way. */
  const snapshot = vectors.map((entry) => {
    const before = new Set(eachLayer(entry.document).map((layer) => layer.id));
    return {
      name: entry.name,
      document: entry.document,
      call: entry.call,
      // Not the documents themselves: the normalised ones, which is what both sides
      // compare. Identifiers an operation made become #1, #2, ...; timestamps become T.
      expected: normalise(entry.expected, before)
    };
  });

  it('has a vector for every structural operation the interface still does itself', () => {
    const ops = new Set(vectors.map((entry) => entry.call.op));
    for (const op of [
      'core.layer.set_visible', 'core.layer.set_opacity', 'core.layer.set_blend_mode', 'core.layer.rename',
      'core.layer.set_locked', 'core.layer.set_collapsed', 'core.layer.select', 'core.layer.reset_transform',
      'core.layer.set_clipped', 'core.layer.move', 'core.layer.delete', 'core.layer.duplicate', 'core.layer.group', 'core.layer.ungroup',
      'core.layer.add_group', 'core.layer.add_adjustment'
    ]) {
      expect(ops.has(op), `${op} has no parity vector`).toBe(true);
    }
  });

  it('produces exactly the vectors the Rust engine is tested against', () => {
    const text = JSON.stringify(snapshot, null, 2) + '\n';
    if (process.env.UPDATE_FIXTURES) writeFileSync(FIXTURE, text);
    expect(existsSync(FIXTURE), 'run with UPDATE_FIXTURES=1 to create the fixture').toBe(true);
    // Compared as values so line endings cannot matter.
    expect(JSON.parse(readFileSync(FIXTURE, 'utf8'))).toEqual(JSON.parse(text));
  });
});

/**
 * Merge Down is decided in two places: the panel asks `mergeSafetyProblem` before it
 * sends anything, and the engine decides again when it runs. The merge itself bakes
 * pixels, which TypeScript does not do, so what is compared here is the verdict and,
 * for a merge that goes ahead, the stack it leaves: each layer's name and whether it
 * is still clipped. Clipping is where the two have disagreed.
 */
describe('the TypeScript merge check and the Rust engine agree', () => {
  type Stack = Array<[id: string, clipped: boolean]>;
  interface MergeVector {
    name: string;
    document: LayerDocument;
    /** The layer Merge down is run on. */
    id: string;
    refused: boolean;
    /** The stack that is left, bottom to top, when the merge goes ahead. */
    after: Stack | null;
  }

  const stack = (spec: Stack, active: string) =>
    document(spec.map(([id, clipped]) => (clipped ? { ...pixel(id), clipped: true } : pixel(id))), active);

  /** What the panel decides for `id`, as App.svelte asks it. */
  function verdict(doc: LayerDocument, id: string): boolean {
    const path = pathTo(doc, id)!;
    const siblings = path.length > 1 ? childrenOf(doc.layers[path[0]]) : doc.layers;
    const below = siblings[path[path.length - 1] - 1];
    return mergeSafetyProblem(doc, [below.id, id]) !== null;
  }

  const ok = (name: string, spec: Stack, id: string, after: Stack): MergeVector =>
    ({ name, document: stack(spec, id), id, refused: false, after });
  const refused = (name: string, spec: Stack, id: string): MergeVector =>
    ({ name, document: stack(spec, id), id, refused: true, after: null });

  const vectors: MergeVector[] = [
    ok('two plain layers', [['a', false], ['b', false], ['c', false]], 'c', [['a', false], ['b', false]]),
    ok('a clipped layer into its base', [['a', false], ['b', false], ['c', true]], 'c', [['a', false], ['b', false]]),
    refused('a base into a layer clipped to the one below', [['a', false], ['b', true], ['c', false]], 'c'),
    refused('a base that has a clipped layer above it', [['a', false], ['b', false], ['c', false], ['x', true]], 'c'),
    refused('a base into the ground, with a clipped layer above it', [['ground', false], ['base', false], ['ink', true]], 'base'),
    ok('a clipped layer into its base, others still clipped above', [['ground', false], ['base', false], ['ink', true], ['tint', true]], 'ink',
      [['ground', false], ['base', false], ['tint', true]]),
    ok('clipped layers into each other', [['ground', false], ['base', false], ['c1', true], ['d', true]], 'd',
      [['ground', false], ['base', false], ['c1', true]]),
    ok('clipped layers into each other, another clipped above', [['ground', false], ['base', false], ['c1', true], ['d', true], ['e', true]], 'd',
      [['ground', false], ['base', false], ['c1', true], ['e', true]]),
    ok('a base into a base after a clipping group', [['a', false], ['b', false], ['c', true], ['d', false], ['e', false]], 'e',
      [['a', false], ['b', false], ['c', true], ['d', false]]),
    {
      name: 'layers clipped to an adjustment layer',
      document: document([fixed(createAdjustmentLayer('Grey', { type: 'grayscale' } as never, NOW), 'adj'), { ...pixel('x'), clipped: true }, { ...pixel('y'), clipped: true }], 'x'),
      id: 'x',
      refused: true,
      after: null
    },
    {
      name: 'the last layer clipped to an adjustment layer',
      document: document([fixed(createAdjustmentLayer('Grey', { type: 'grayscale' } as never, NOW), 'adj'), { ...pixel('x'), clipped: true }, { ...pixel('y'), clipped: true }], 'y'),
      id: 'y',
      refused: false,
      after: [['adj', false], ['x', true]]
    }
  ];

  it('has the verdict the TypeScript check gives, for every vector', () => {
    for (const entry of vectors) {
      expect(verdict(entry.document, entry.id), entry.name).toBe(entry.refused);
    }
  });

  it('produces exactly the vectors the Rust engine is tested against', () => {
    const text = JSON.stringify(vectors, null, 2) + '\n';
    if (process.env.UPDATE_FIXTURES) writeFileSync(MERGE_FIXTURE, text);
    expect(existsSync(MERGE_FIXTURE), 'run with UPDATE_FIXTURES=1 to create the fixture').toBe(true);
    expect(JSON.parse(readFileSync(MERGE_FIXTURE, 'utf8'))).toEqual(JSON.parse(text));
  });
});
