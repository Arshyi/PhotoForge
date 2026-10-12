import type { Layer, LayerDocument } from './types';

/**
 * Gives a document the engine returned back the object identity of everything that did not change.
 *
 * The undo history keeps one whole document per step and relies on untouched layers
 * being the *same objects* in neighbouring steps — a layer's mask can be large, and
 * two hundred copies of it would be a real cost. The tree functions the panel used to
 * call preserved that by construction. A document deserialised from the engine is new
 * all the way down, so this restores it: any layer that is equal to the layer with the
 * same identifier before is replaced by that layer, and a group that did change keeps
 * its unchanged children.
 *
 * It decides on equality of values and nothing else, so it cannot make two different
 * layers into one; the worst it can do is fail to share, which costs memory and no
 * correctness.
 */
export function shareUnchanged(previous: LayerDocument, next: LayerDocument): LayerDocument {
  const before = new Map<string, Layer>();
  const index = (layers: Layer[]) => {
    for (const layer of layers) {
      before.set(layer.id, layer);
      if (layer.content.type === 'group') index(layer.content.children);
    }
  };
  index(previous.layers);

  const share = (layer: Layer): Layer => {
    const old = before.get(layer.id);
    if (old && equal(old, layer)) return old;
    if (layer.content.type !== 'group') return layer;
    const children = layer.content.children.map(share);
    const oldContent = old?.content.type === 'group' ? old.content : null;
    const sameChildren =
      oldContent !== null &&
      oldContent.children.length === children.length &&
      oldContent.children.every((child, position) => child === children[position]);
    return {
      ...layer,
      content: { ...layer.content, children: sameChildren && oldContent ? oldContent.children : children }
    };
  };

  const layers = next.layers.map(share);
  const unchanged = layers.length === previous.layers.length && layers.every((layer, i) => layer === previous.layers[i]);
  return { ...next, layers: unchanged ? previous.layers : layers };
}

/** Value equality for the JSON the engine speaks: absent and null are the same thing. */
export function equal(a: unknown, b: unknown): boolean {
  if (a === b) return true;
  if (a === null || a === undefined) return b === null || b === undefined;
  if (b === null || b === undefined) return false;
  if (typeof a !== 'object' || typeof b !== 'object') return false;
  if (Array.isArray(a) || Array.isArray(b)) {
    if (!Array.isArray(a) || !Array.isArray(b) || a.length !== b.length) return false;
    return a.every((item, position) => equal(item, b[position]));
  }
  const left = a as Record<string, unknown>;
  const right = b as Record<string, unknown>;
  const keys = new Set([...Object.keys(left), ...Object.keys(right)]);
  for (const key of keys) {
    if (!equal(left[key], right[key])) return false;
  }
  return true;
}
