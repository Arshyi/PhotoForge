import { describe, expect, it } from 'vitest';
import {
  activeLayer,
  countLayers,
  createAdjustmentLayer,
  createDocument,
  createGroupLayer,
  createPixelLayer,
  displayRows,
  duplicateLayer,
  eachLayer,
  findLayer,
  groupLayers,
  insertLayer,
  isSelfOrDescendant,
  isSimpleDocument,
  layerAt,
  moveLayer,
  newLayerId,
  parentOf,
  pathTo,
  referencedPixelIds,
  removeLayer,
  ungroupLayer,
  updateLayer,
  validateDocument
} from './tree';
import { MAX_GROUP_DEPTH, type Layer, type LayerDocument } from './types';

function pixel(name: string, pixelId = `px${name}`, width = 16, height = 16): Layer {
  return createPixelLayer(name, pixelId, width, height);
}

function documentWith(layers: Layer[], width = 16, height = 16): LayerDocument {
  return createDocument(width, height, layers);
}

describe('layer identifiers', () => {
  it('generates unique identifiers that satisfy the backend character set', () => {
    const ids = new Set<string>();
    for (let index = 0; index < 500; index += 1) ids.add(newLayerId());
    expect(ids.size).toBe(500);
    for (const id of ids) {
      expect(id).toMatch(/^[A-Za-z0-9_-]{1,64}$/);
    }
  });
});

describe('tree traversal', () => {
  it('enumerates depth first from the bottom of the stack', () => {
    const document = documentWith([
      pixel('base'),
      createGroupLayer('outer', [pixel('inner'), createGroupLayer('nested', [pixel('deep')])])
    ]);
    expect(eachLayer(document).map((layer) => layer.name)).toEqual([
      'base',
      'outer',
      'inner',
      'nested',
      'deep'
    ]);
    expect(countLayers(document)).toBe(5);
  });

  it('resolves paths and parents for nested layers', () => {
    const deep = pixel('deep');
    const nested = createGroupLayer('nested', [deep]);
    const document = documentWith([pixel('base'), createGroupLayer('outer', [nested])]);
    expect(pathTo(document, deep.id)).toEqual([1, 0, 0]);
    expect(layerAt(document, [1, 0, 0])?.id).toBe(deep.id);
    expect(parentOf(document, deep.id)).toBe(nested.id);
    expect(parentOf(document, document.layers[0].id)).toBeNull();
    expect(pathTo(document, 'missing')).toBeNull();
  });

  it('collects the pixel buffers a document references', () => {
    const shared = pixel('a', 'shared');
    const alsoShared = pixel('b', 'shared');
    const document = documentWith([
      shared,
      alsoShared,
      createGroupLayer('g', [pixel('c', 'other')]),
      createAdjustmentLayer('adj', { type: 'grayscale' })
    ]);
    expect(referencedPixelIds(document)).toEqual(['other', 'shared']);
  });
});

describe('insertion and removal', () => {
  it('inserts at the requested position and selects the new layer', () => {
    const document = documentWith([pixel('bottom')]);
    const added = pixel('top');
    const next = insertLayer(document, added, null, 1);
    expect(next.layers.map((layer) => layer.name)).toEqual(['bottom', 'top']);
    expect(next.activeLayerId).toBe(added.id);
  });

  it('refuses to insert a duplicate identifier', () => {
    const existing = pixel('a');
    const document = documentWith([existing]);
    expect(insertLayer(document, existing, null, 0)).toBe(document);
  });

  it('refuses to insert into a layer that is not a group', () => {
    const target = pixel('a');
    const document = documentWith([target]);
    expect(insertLayer(document, pixel('b'), target.id, 0)).toBe(document);
  });

  it('removes a layer and its whole subtree', () => {
    const leaf = pixel('leaf');
    const group = createGroupLayer('g', [leaf]);
    const document = documentWith([pixel('base'), group]);
    const next = removeLayer(document, group.id);
    expect(countLayers(next)).toBe(1);
    expect(findLayer(next, leaf.id)).toBeNull();
  });

  it('clears the selection when the selected layer is removed', () => {
    const target = pixel('a');
    const document = { ...documentWith([target]), activeLayerId: target.id };
    expect(removeLayer(document, target.id).activeLayerId).toBeNull();
  });

  it('keeps the selection when an unrelated layer is removed', () => {
    const kept = pixel('a');
    const other = pixel('b');
    const document = { ...documentWith([kept, other]), activeLayerId: kept.id };
    expect(removeLayer(document, other.id).activeLayerId).toBe(kept.id);
  });

  it('leaves the original document untouched when editing', () => {
    const target = pixel('a');
    const document = documentWith([target, pixel('b')]);
    const snapshot = JSON.stringify(document);
    updateLayer(document, target.id, (layer) => ({ ...layer, opacity: 0.25 }));
    expect(JSON.stringify(document)).toBe(snapshot);
  });

  it('shares untouched subtrees by reference instead of deep copying', () => {
    const untouched = createGroupLayer('untouched', [pixel('deep')]);
    const target = pixel('target');
    const document = documentWith([untouched, target]);
    const next = updateLayer(document, target.id, (layer) => ({ ...layer, opacity: 0.5 }));
    expect(next.layers[0]).toBe(document.layers[0]);
    expect(next.layers[1]).not.toBe(document.layers[1]);
  });

  it('stamps a modification time when a layer changes', () => {
    const target = pixel('a');
    const document = documentWith([target]);
    const next = updateLayer(
      document,
      target.id,
      (layer) => ({ ...layer, opacity: 0.5 }),
      new Date('2030-01-02T03:04:05.000Z')
    );
    expect(findLayer(next, target.id)?.metadata.modifiedAt).toBe('2030-01-02T03:04:05.000Z');
  });
});

describe('moving layers', () => {
  it('moves a layer into and back out of a group', () => {
    const free = pixel('free');
    const group = createGroupLayer('g', [pixel('member')]);
    let document = documentWith([free, group]);

    document = moveLayer(document, free.id, group.id, 0);
    expect(document.layers).toHaveLength(1);
    expect(findLayer(document, group.id)).toBeTruthy();
    expect(countLayers(document)).toBe(3);

    document = moveLayer(document, free.id, null, 0);
    expect(document.layers).toHaveLength(2);
    expect(document.layers[0].id).toBe(free.id);
  });

  it('reorders siblings within the root stack', () => {
    const first = pixel('a');
    const second = pixel('b');
    const document = documentWith([first, second]);
    const next = moveLayer(document, first.id, null, 1);
    expect(next.layers.map((layer) => layer.id)).toEqual([second.id, first.id]);
  });

  it('never moves a group into itself or one of its descendants', () => {
    const inner = createGroupLayer('inner', [pixel('leaf')]);
    const outer = createGroupLayer('outer', [inner]);
    const document = documentWith([outer]);
    expect(moveLayer(document, outer.id, outer.id, 0)).toBe(document);
    expect(moveLayer(document, outer.id, inner.id, 0)).toBe(document);
    expect(countLayers(document)).toBe(3);
  });

  it('rejects a move onto a parent that is not a group', () => {
    const first = pixel('a');
    const second = pixel('b');
    const document = documentWith([first, second]);
    expect(moveLayer(document, first.id, second.id, 0)).toBe(document);
  });

  it('rejects a move that would nest past the group limit but allows one that fits', () => {
    // A chain of exactly MAX_GROUP_DEPTH groups: the innermost already sits at
    // the limit, so nothing may be placed inside it.
    let chain: Layer = createGroupLayer('g0', []);
    for (let index = 1; index < MAX_GROUP_DEPTH; index += 1) {
      chain = createGroupLayer(`g${index}`, [chain]);
    }
    const spare = pixel('spare');
    const document = documentWith([chain, spare]);
    expect(validateDocument(document)).toEqual([]);

    const innermost = eachLayer(document).find(
      (layer) => layer.content.type === 'group' && layer.content.children.length === 0
    ) as Layer;
    const oneLevelOut = eachLayer(document).find(
      (layer) => layer.content.type === 'group' && layer.content.children[0]?.id === innermost.id
    ) as Layer;

    expect(pathTo(document, innermost.id)).toHaveLength(MAX_GROUP_DEPTH);
    expect(moveLayer(document, spare.id, innermost.id, 0)).toBe(document);

    const allowed = moveLayer(document, spare.id, oneLevelOut.id, 0);
    expect(allowed).not.toBe(document);
    expect(validateDocument(allowed)).toEqual([]);
  });

  it('keeps the selected layer selected across a move', () => {
    const moved = pixel('moved');
    const group = createGroupLayer('g', []);
    const document = { ...documentWith([moved, group]), activeLayerId: moved.id };
    expect(moveLayer(document, moved.id, group.id, 0).activeLayerId).toBe(moved.id);
  });

  it('reports descendants correctly', () => {
    const leaf = pixel('leaf');
    const inner = createGroupLayer('inner', [leaf]);
    const outer = createGroupLayer('outer', [inner]);
    const other = pixel('other');
    const document = documentWith([outer, other]);
    expect(isSelfOrDescendant(document, outer.id, outer.id)).toBe(true);
    expect(isSelfOrDescendant(document, outer.id, leaf.id)).toBe(true);
    expect(isSelfOrDescendant(document, outer.id, other.id)).toBe(false);
  });
});

describe('duplication', () => {
  it('duplicates a layer above the original with a new identifier', () => {
    const original = pixel('Portrait');
    const document = documentWith([original]);
    const { document: next, layer } = duplicateLayer(document, original.id);
    expect(layer?.id).not.toBe(original.id);
    expect(layer?.name).toBe('Portrait copy');
    expect(next.layers.map((entry) => entry.id)).toEqual([original.id, layer?.id]);
    expect(next.activeLayerId).toBe(layer?.id);
  });

  it('reuses the pixel buffer so a duplicate costs no extra pixel memory', () => {
    const original = pixel('a', 'shared');
    const document = documentWith([original]);
    const { document: next } = duplicateLayer(document, original.id);
    expect(referencedPixelIds(next)).toEqual(['shared']);
  });

  it('gives every layer in a duplicated group a fresh identifier', () => {
    const group = createGroupLayer('g', [pixel('one'), createGroupLayer('inner', [pixel('two')])]);
    const document = documentWith([group]);
    const { document: next } = duplicateLayer(document, group.id);
    const ids = eachLayer(next).map((layer) => layer.id);
    expect(new Set(ids).size).toBe(ids.length);
    expect(validateDocument(next)).toEqual([]);
  });
});

describe('grouping', () => {
  it('wraps sibling layers in a new group in stack order', () => {
    const first = pixel('a');
    const second = pixel('b');
    const third = pixel('c');
    const document = documentWith([first, second, third]);
    const { document: next, group } = groupLayers(document, [second.id, third.id], 'Sky');
    expect(group?.name).toBe('Sky');
    expect(next.layers.map((layer) => layer.id)).toEqual([first.id, group?.id]);
    const children = group && next.layers[1].content.type === 'group'
      ? next.layers[1].content.children.map((layer) => layer.id)
      : [];
    expect(children).toEqual([second.id, third.id]);
  });

  it('refuses to group layers with different parents', () => {
    const loose = pixel('loose');
    const inner = pixel('inner');
    const document = documentWith([loose, createGroupLayer('g', [inner])]);
    expect(groupLayers(document, [loose.id, inner.id]).group).toBeNull();
  });

  it('ungroups a group back into its parent in order', () => {
    const first = pixel('a');
    const second = pixel('b');
    const group = createGroupLayer('g', [first, second]);
    const bottom = pixel('bottom');
    const document = documentWith([bottom, group]);
    const next = ungroupLayer(document, group.id);
    expect(next.layers.map((layer) => layer.id)).toEqual([bottom.id, first.id, second.id]);
  });

  it('leaves non-group layers untouched when ungrouping', () => {
    const target = pixel('a');
    const document = documentWith([target]);
    expect(ungroupLayer(document, target.id)).toBe(document);
  });

  it('round trips group then ungroup back to the original order', () => {
    const ids = ['a', 'b', 'c'].map((name) => pixel(name));
    const document = documentWith(ids);
    const { document: grouped, group } = groupLayers(document, [ids[0].id, ids[1].id]);
    const restored = ungroupLayer(grouped, group?.id ?? '');
    expect(restored.layers.map((layer) => layer.id)).toEqual(ids.map((layer) => layer.id));
  });
});

describe('display rows', () => {
  it('lists the top layer first with indentation', () => {
    const document = documentWith([
      pixel('bottom'),
      createGroupLayer('g', [pixel('child')]),
      pixel('top')
    ]);
    const rows = displayRows(document);
    expect(rows.map((row) => row.layer.name)).toEqual(['top', 'g', 'child', 'bottom']);
    expect(rows.map((row) => row.depth)).toEqual([0, 0, 1, 0]);
  });

  it('hides the children of a collapsed group', () => {
    const group = { ...createGroupLayer('g', [pixel('child')]), collapsed: true };
    const rows = displayRows(documentWith([group]));
    expect(rows.map((row) => row.layer.name)).toEqual(['g']);
  });

  it('marks children of a hidden group so the panel can dim them', () => {
    const group = { ...createGroupLayer('g', [pixel('child')]), visible: false };
    const rows = displayRows(documentWith([group]));
    expect(rows[0].hiddenByAncestor).toBe(false);
    expect(rows[1].hiddenByAncestor).toBe(true);
  });
});

describe('the simple-document fast path', () => {
  it('recognizes a freshly opened single-layer document', () => {
    const document = documentWith([pixel('Background', 'px1', 16, 16)]);
    expect(isSimpleDocument(document)).toBe(true);
  });

  it('stops being simple as soon as layers are added or changed', () => {
    const background = pixel('Background');
    expect(isSimpleDocument(documentWith([background, pixel('second')]))).toBe(false);
    expect(
      isSimpleDocument(documentWith([{ ...background, opacity: 0.5 }]))
    ).toBe(false);
    expect(
      isSimpleDocument(documentWith([{ ...background, blendMode: 'multiply' }]))
    ).toBe(false);
    expect(isSimpleDocument(documentWith([{ ...background, visible: false }]))).toBe(false);
    expect(
      isSimpleDocument(
        documentWith([
          { ...background, transform: { ...background.transform, translateX: 4 } }
        ])
      )
    ).toBe(false);
    expect(isSimpleDocument(documentWith([createGroupLayer('g', [])]))).toBe(false);
    expect(
      isSimpleDocument(documentWith([pixel('Background', 'px1', 8, 8)], 16, 16))
    ).toBe(false);
  });

  /**
   * An ordinary photo opened in 0.8.2 must take exactly the render path it took
   * before layers existed. A transform gained an optional sampling mode in this
   * release, and picking one on a layer that is not being resampled changes no
   * pixels, so it must not push the document onto the layered path either.
   */
  it('stays simple for a document written before the sampling mode existed', () => {
    const background = pixel('Background', 'px1', 16, 16);
    const legacy = {
      ...background,
      transform: {
        translateX: 0,
        translateY: 0,
        scaleX: 1,
        scaleY: 1,
        rotationDegrees: 0,
        flipHorizontal: false,
        flipVertical: false
      } as unknown as typeof background.transform
    };
    expect(isSimpleDocument(documentWith([legacy]))).toBe(true);
  });

  it('stays simple when only the sampling mode differs', () => {
    const background = pixel('Background', 'px1', 16, 16);
    const nearest = {
      ...background,
      transform: { ...background.transform, interpolation: 'nearest' as const }
    };
    expect(isSimpleDocument(documentWith([nearest]))).toBe(true);
  });

  it('leaves the fast path as soon as the placement actually moves', () => {
    const background = pixel('Background', 'px1', 16, 16);
    for (const patch of [
      { flipHorizontal: true },
      { flipVertical: true },
      { scaleY: 1.01 },
      { rotationDegrees: 0.5 },
      { translateY: -1 }
    ]) {
      expect(
        isSimpleDocument(
          documentWith([{ ...background, transform: { ...background.transform, ...patch } }])
        )
      ).toBe(false);
    }
  });
});

describe('validation', () => {
  it('accepts a well formed document', () => {
    const document = documentWith([
      pixel('base'),
      createGroupLayer('g', [createAdjustmentLayer('adj', { type: 'grayscale' })])
    ]);
    expect(validateDocument(document)).toEqual([]);
  });

  it('reports duplicate identifiers', () => {
    const duplicate = pixel('a');
    const document = documentWith([duplicate, { ...duplicate }]);
    expect(validateDocument(document).join(' ')).toContain('share the identifier');
  });

  it('reports an out of range opacity and a broken transform', () => {
    const layer = pixel('a');
    expect(validateDocument(documentWith([{ ...layer, opacity: 2 }])).join(' ')).toContain(
      'opacity'
    );
    expect(
      validateDocument(
        documentWith([{ ...layer, transform: { ...layer.transform, scaleX: 0 } }])
      ).join(' ')
    ).toContain('transform');
    expect(
      validateDocument(
        documentWith([{ ...layer, transform: { ...layer.transform, translateX: Number.NaN } }])
      ).join(' ')
    ).toContain('transform');
  });

  it('reports a blank name and a stale selection', () => {
    const layer = pixel('a');
    expect(validateDocument(documentWith([{ ...layer, name: '  ' }])).join(' ')).toContain(
      'invalid name'
    );
    const document = { ...documentWith([layer]), activeLayerId: 'ghost' };
    expect(validateDocument(document).join(' ')).toContain('no longer exists');
  });

  it('reports nesting past the group limit', () => {
    let deepest: Layer = pixel('leaf');
    for (let index = 0; index < MAX_GROUP_DEPTH; index += 1) {
      deepest = createGroupLayer(`g${index}`, [deepest]);
    }
    expect(validateDocument(documentWith([deepest])).join(' ')).toContain('nested deeper');
  });
});

describe('active layer', () => {
  it('resolves the selected layer or null', () => {
    const target = pixel('a');
    const document = { ...documentWith([target]), activeLayerId: target.id };
    expect(activeLayer(document)?.id).toBe(target.id);
    expect(activeLayer({ ...document, activeLayerId: null })).toBeNull();
  });
});
