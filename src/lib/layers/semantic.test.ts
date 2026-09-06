import { describe, expect, it } from 'vitest';
import { thumbnailKey } from './thumbnails';
import {
  createDocument,
  createGroupLayer,
  createShapeLayer,
  createTextLayer,
  isSemanticLayer,
  layerKind,
  textContentOf
} from './tree';
import { layerKindIcons, layerKindLabels } from './types';
import type { Layer, LayerDocument } from './types';

function documentFor(layers: Layer[]): LayerDocument {
  return createDocument(64, 64, layers);
}

describe('text layers', () => {
  it('is created as editable text, not as pixels', () => {
    const layer = createTextLayer('Title', 'Hello', 10, 40, 32);
    expect(layer.content.type).toBe('text');
    expect(layerKind(layer)).toBe('text');
    const content = textContentOf(layer);
    expect(content?.text).toBe('Hello');
    expect(content?.fontSize).toBe(32);
    // No named face, so nothing can be reported missing on another machine.
    expect(content?.fontFamily).toBe('');
  });

  it('reads back as text and not as anything else', () => {
    const text = createTextLayer('t', 'Hello', 0, 0);
    const shape = createShapeLayer('s', { type: 'ellipse', cx: 10, cy: 10, rx: 5, ry: 5 });
    const group = createGroupLayer('g', []);
    expect(textContentOf(shape)).toBeNull();
    expect(textContentOf(group)).toBeNull();
    expect(isSemanticLayer(text)).toBe(true);
    expect(isSemanticLayer(shape)).toBe(true);
    expect(isSemanticLayer(group)).toBe(false);
  });

  it("has its own label and icon rather than borrowing another kind's", () => {
    const kinds = ['pixel', 'group', 'adjustment', 'shape', 'text'] as const;
    const labels = kinds.map((kind) => layerKindLabels[kind]);
    const icons = kinds.map((kind) => layerKindIcons[kind]);
    expect(new Set(labels).size).toBe(kinds.length);
    expect(new Set(icons).size).toBe(kinds.length);
  });
});

describe('thumbnail keys for semantic layers', () => {
  it('changes when the words change', () => {
    const before = createTextLayer('t', 'Hello', 0, 0);
    const after = { ...before, content: { ...before.content, text: 'Goodbye' } } as Layer;
    expect(thumbnailKey(before, documentFor([before]))).not.toBe(
      thumbnailKey(after, documentFor([after]))
    );
  });

  it('changes when the setting changes but the words do not', () => {
    const before = createTextLayer('t', 'Hello', 0, 0);
    const after = { ...before, content: { ...before.content, fontSize: 96 } } as Layer;
    expect(thumbnailKey(before, documentFor([before]))).not.toBe(
      thumbnailKey(after, documentFor([after]))
    );
  });

  it('changes when a shape is recoloured', () => {
    const before = createShapeLayer('s', { type: 'ellipse', cx: 10, cy: 10, rx: 5, ry: 5 });
    const after = {
      ...before,
      content: { ...before.content, fill: { red: 1, green: 0, blue: 0, alpha: 1 } }
    } as Layer;
    expect(thumbnailKey(before, documentFor([before]))).not.toBe(
      thumbnailKey(after, documentFor([after]))
    );
  });

  /**
   * The failure this guards against is silent: before shapes and text existed,
   * anything that was not a pixel or an adjustment fell through to the group
   * branch, which contributes only the literal 'g' and its children. Two
   * different text layers would then have keyed identically and shown each
   * other's thumbnail.
   */
  it('does not key a shape, a text layer and a group the same way', () => {
    const text = createTextLayer('t', 'Hello', 0, 0);
    const shape = createShapeLayer('s', { type: 'ellipse', cx: 10, cy: 10, rx: 5, ry: 5 });
    const group = createGroupLayer('g', []);
    const keys = [text, shape, group].map((layer) => {
      // The same identifier throughout, so only the content can distinguish
      // them and an identity-based key cannot pass this by accident.
      const normalised = { ...layer, id: 'same' } as Layer;
      return thumbnailKey(normalised, documentFor([normalised]));
    });
    expect(new Set(keys).size).toBe(3);
  });

  it('distinguishes two different text layers inside one group', () => {
    const first = { ...createTextLayer('a', 'One', 0, 0), id: 'a' } as Layer;
    const second = { ...createTextLayer('b', 'Two', 0, 0), id: 'b' } as Layer;
    const group = createGroupLayer('g', [first, second]);
    const swapped = createGroupLayer('g', [second, first]);
    const normalise = (layer: Layer) => ({ ...layer, id: 'same' }) as Layer;
    expect(thumbnailKey(normalise(group), documentFor([group]))).not.toBe(
      thumbnailKey(normalise(swapped), documentFor([swapped]))
    );
  });
});
