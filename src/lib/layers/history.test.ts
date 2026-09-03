import { describe, expect, it } from 'vitest';
import { LayerHistory } from './history';
import {
  createDocument,
  createGroupLayer,
  createPixelLayer,
  duplicateLayer,
  findLayer,
  groupLayers,
  insertLayer,
  moveLayer,
  removeLayer,
  updateLayer
} from './tree';
import type { Layer, LayerDocument } from './types';

function pixel(name: string, pixelId = `px${name}`): Layer {
  return createPixelLayer(name, pixelId, 16, 16);
}

function start(): { history: LayerHistory; document: LayerDocument; background: Layer } {
  const background = pixel('Background');
  const document = createDocument(16, 16, [background]);
  const history = new LayerHistory();
  history.replace(document, 'Open');
  return { history, document, background };
}

describe('layer history', () => {
  it('starts with no undo or redo available', () => {
    const { history } = start();
    expect(history.canUndo).toBe(false);
    expect(history.canRedo).toBe(false);
    expect(history.currentLabel).toBe('Open');
  });

  it('undoes and redoes a layer creation', () => {
    const { history, document } = start();
    const added = pixel('New layer');
    history.commit(insertLayer(document, added, null, 1), 'New layer');

    expect(history.canUndo).toBe(true);
    expect(history.undoLabel).toBe('Open');
    expect(findLayer(history.document as LayerDocument, added.id)).toBeTruthy();

    history.undo();
    expect(findLayer(history.document as LayerDocument, added.id)).toBeNull();
    expect(history.canRedo).toBe(true);

    history.redo();
    expect(findLayer(history.document as LayerDocument, added.id)).toBeTruthy();
  });

  it('undoes and redoes a duplicate', () => {
    const { history, document, background } = start();
    const { document: duplicated, layer } = duplicateLayer(document, background.id);
    history.commit(duplicated, 'Duplicate layer');
    expect(history.document?.layers).toHaveLength(2);

    history.undo();
    expect(history.document?.layers).toHaveLength(1);
    history.redo();
    expect(findLayer(history.document as LayerDocument, layer?.id ?? '')).toBeTruthy();
  });

  it('undoes a move into a group as one step', () => {
    const moved = pixel('moved');
    const group = createGroupLayer('g', []);
    const document = createDocument(16, 16, [moved, group]);
    const history = new LayerHistory();
    history.replace(document, 'Open');

    history.commit(moveLayer(document, moved.id, group.id, 0), 'Move layer');
    expect(history.document?.layers).toHaveLength(1);
    history.undo();
    expect(history.document?.layers).toHaveLength(2);
  });

  it('undoes grouping as one step', () => {
    const first = pixel('a');
    const second = pixel('b');
    const document = createDocument(16, 16, [first, second]);
    const history = new LayerHistory();
    history.replace(document, 'Open');

    const { document: grouped } = groupLayers(document, [first.id, second.id]);
    history.commit(grouped, 'Group layers');
    expect(history.document?.layers).toHaveLength(1);
    history.undo();
    expect(history.document?.layers).toHaveLength(2);
    expect(history.document?.layers.map((layer) => layer.id)).toEqual([first.id, second.id]);
  });

  it('undoes a deletion and restores the whole subtree', () => {
    const leaf = pixel('leaf');
    const group = createGroupLayer('g', [leaf]);
    const document = createDocument(16, 16, [group]);
    const history = new LayerHistory();
    history.replace(document, 'Open');

    history.commit(removeLayer(document, group.id), 'Delete layer');
    expect(history.document?.layers).toHaveLength(0);
    history.undo();
    expect(findLayer(history.document as LayerDocument, leaf.id)).toBeTruthy();
  });

  it('collapses a continuous slider gesture into one undo step', () => {
    const { history, document, background } = start();
    let current = document;
    for (let step = 1; step <= 10; step += 1) {
      current = updateLayer(current, background.id, (layer) => ({
        ...layer,
        opacity: step / 10
      }));
      history.commit(current, 'Layer opacity', `opacity:${background.id}`, 1_000 + step * 20);
    }
    expect(history.undoDepth).toBe(1);
    history.undo();
    expect(findLayer(history.document as LayerDocument, background.id)?.opacity).toBe(1);
  });

  it('starts a new undo step once the coalescing window lapses', () => {
    const { history, document, background } = start();
    const first = updateLayer(document, background.id, (layer) => ({ ...layer, opacity: 0.8 }));
    history.commit(first, 'Layer opacity', `opacity:${background.id}`, 1_000);
    const second = updateLayer(first, background.id, (layer) => ({ ...layer, opacity: 0.4 }));
    history.commit(second, 'Layer opacity', `opacity:${background.id}`, 5_000);
    expect(history.undoDepth).toBe(2);
  });

  it('does not coalesce two different gestures', () => {
    const { history, document, background } = start();
    const opacity = updateLayer(document, background.id, (layer) => ({ ...layer, opacity: 0.5 }));
    history.commit(opacity, 'Layer opacity', `opacity:${background.id}`, 1_000);
    const blend = updateLayer(opacity, background.id, (layer) => ({
      ...layer,
      blendMode: 'multiply'
    }));
    history.commit(blend, 'Blend mode', `blend:${background.id}`, 1_010);
    expect(history.undoDepth).toBe(2);
  });

  it('treats a drag reorder as a single undo step even with many intermediate commits', () => {
    const first = pixel('a');
    const second = pixel('b');
    const third = pixel('c');
    let current = createDocument(16, 16, [first, second, third]);
    const history = new LayerHistory();
    history.replace(current, 'Open');
    const key = `reorder:${first.id}`;
    for (let index = 1; index <= 3; index += 1) {
      current = moveLayer(current, first.id, null, index % 3);
      history.commit(current, 'Reorder layer', key, 2_000 + index * 10);
    }
    expect(history.undoDepth).toBe(1);
    history.undo();
    expect(history.document?.layers.map((layer) => layer.id)).toEqual([
      first.id,
      second.id,
      third.id
    ]);
  });

  it('ends coalescing after an undo so the next change is its own step', () => {
    const { history, document, background } = start();
    const first = updateLayer(document, background.id, (layer) => ({ ...layer, opacity: 0.5 }));
    history.commit(first, 'Layer opacity', 'opacity', 1_000);
    history.undo();
    const second = updateLayer(
      history.document as LayerDocument,
      background.id,
      (layer) => ({ ...layer, opacity: 0.25 })
    );
    history.commit(second, 'Layer opacity', 'opacity', 1_010);
    expect(history.undoDepth).toBe(1);
    expect(history.canRedo).toBe(false);
  });

  it('drops redo entries once a new change is committed', () => {
    const { history, document } = start();
    history.commit(insertLayer(document, pixel('one'), null, 1), 'New layer');
    history.undo();
    expect(history.canRedo).toBe(true);
    history.commit(
      insertLayer(history.document as LayerDocument, pixel('two'), null, 1),
      'New layer'
    );
    expect(history.canRedo).toBe(false);
  });

  it('ignores a commit that does not change the document', () => {
    const { history, document } = start();
    history.commit(document, 'No change');
    expect(history.undoDepth).toBe(0);
  });

  it('bounds the retained history depth', () => {
    const history = new LayerHistory(5);
    const background = pixel('Background');
    let current = createDocument(16, 16, [background]);
    history.replace(current, 'Open');
    for (let step = 0; step < 20; step += 1) {
      current = updateLayer(current, background.id, (layer) => ({
        ...layer,
        opacity: 1 - step / 100
      }));
      history.commit(current, `Step ${step}`);
    }
    expect(history.undoDepth).toBe(5);
  });

  it('reports labels for the undo and redo actions', () => {
    const { history, document } = start();
    history.commit(insertLayer(document, pixel('one'), null, 1), 'New layer');
    expect(history.undoLabel).toBe('Open');
    expect(history.currentLabel).toBe('New layer');
    history.undo();
    expect(history.redoLabel).toBe('New layer');
  });

  it('keeps pixel buffers reachable from history so an undone merge can be redone', () => {
    const background = pixel('Background', 'px1');
    const document = createDocument(16, 16, [background]);
    const history = new LayerHistory();
    history.replace(document, 'Open');

    const merged = createDocument(16, 16, [pixel('Merged', 'px2')]);
    history.commit(merged, 'Flatten image');
    expect(history.reachablePixelIds()).toEqual(['px1', 'px2']);

    history.undo();
    expect(history.reachablePixelIds()).toEqual(['px1', 'px2']);
  });

  it('clears everything on demand', () => {
    const { history, document } = start();
    history.commit(insertLayer(document, pixel('one'), null, 1), 'New layer');
    history.clear();
    expect(history.document).toBeNull();
    expect(history.canUndo).toBe(false);
    expect(history.reachablePixelIds()).toEqual([]);
  });
});
