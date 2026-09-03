import { describe, expect, it } from 'vitest';
import { MAX_THUMBNAIL_ENTRIES, ThumbnailCache, thumbnailKey } from './thumbnails';
import { createAdjustmentLayer, createDocument, createGroupLayer, createPixelLayer } from './tree';
import type { Layer, LayerDocument, LayerMask } from './types';

function pixel(name: string, pixelId = `px${name}`): Layer {
  return createPixelLayer(name, pixelId, 16, 16);
}

function documentFor(layers: Layer[]): LayerDocument {
  return createDocument(16, 16, layers);
}

function mask(checksum: string): LayerMask {
  return {
    snapshot: {
      version: 1,
      width: 16,
      height: 16,
      encoding: 'base64_rle_u8',
      data: '',
      checksum
    },
    enabled: true,
    inverted: false
  };
}

describe('thumbnail keys', () => {
  it('changes when the pixel buffer changes', () => {
    const before = pixel('a', 'px1');
    const after = { ...before, content: { ...before.content, pixelId: 'px2' } } as Layer;
    const document = documentFor([before]);
    expect(thumbnailKey(before, document)).not.toBe(thumbnailKey(after, document));
  });

  it('changes when visibility, opacity, or blend mode changes', () => {
    const layer = pixel('a');
    const document = documentFor([layer]);
    const base = thumbnailKey(layer, document);
    expect(thumbnailKey({ ...layer, visible: false }, document)).not.toBe(base);
    expect(thumbnailKey({ ...layer, opacity: 0.5 }, document)).not.toBe(base);
    expect(thumbnailKey({ ...layer, blendMode: 'multiply' }, document)).not.toBe(base);
  });

  it('changes when the transform or the mask changes', () => {
    const layer = pixel('a');
    const document = documentFor([layer]);
    const base = thumbnailKey(layer, document);
    expect(
      thumbnailKey({ ...layer, transform: { ...layer.transform, translateX: 3 } }, document)
    ).not.toBe(base);
    expect(thumbnailKey({ ...layer, mask: mask('fnv1a64:1111111111111111') }, document)).not.toBe(
      base
    );
    const withMask = { ...layer, mask: mask('fnv1a64:1111111111111111') };
    const disabled = {
      ...layer,
      mask: { ...mask('fnv1a64:1111111111111111'), enabled: false }
    };
    expect(thumbnailKey(withMask, document)).not.toBe(thumbnailKey(disabled, document));
  });

  it('does not change when only the name, lock state, or selection changes', () => {
    const layer = pixel('a');
    const document = documentFor([layer]);
    const base = thumbnailKey(layer, document);
    expect(thumbnailKey({ ...layer, name: 'Renamed' }, document)).toBe(base);
    expect(thumbnailKey({ ...layer, locked: true }, document)).toBe(base);
    expect(thumbnailKey(layer, { ...document, activeLayerId: layer.id })).toBe(base);
    expect(thumbnailKey({ ...layer, collapsed: true }, document)).toBe(base);
  });

  it('changes when a group child changes', () => {
    const child = pixel('child');
    const group = createGroupLayer('g', [child]);
    const document = documentFor([group]);
    const base = thumbnailKey(group, document);
    const changed = createGroupLayer('g', [{ ...child, opacity: 0.3 }]);
    expect(thumbnailKey({ ...changed, id: group.id }, document)).not.toBe(base);
  });

  it('changes when the canvas is rebound', () => {
    const layer = pixel('a');
    expect(thumbnailKey(layer, documentFor([layer]))).not.toBe(
      thumbnailKey(layer, createDocument(32, 32, [layer]))
    );
  });

  it('distinguishes adjustment layers by their operation', () => {
    const grayscale = createAdjustmentLayer('adj', { type: 'grayscale' });
    const sepia = { ...grayscale, content: { type: 'adjustment', operation: { type: 'sepia' } } } as Layer;
    const document = documentFor([grayscale]);
    expect(thumbnailKey(grayscale, document)).not.toBe(thumbnailKey(sepia, document));
  });
});

describe('thumbnail cache', () => {
  it('returns a cached thumbnail only for a matching key', () => {
    const cache = new ThumbnailCache();
    cache.set('layer', 'key-1', 'data:image/png;base64,AAA');
    expect(cache.get('layer', 'key-1')).toBe('data:image/png;base64,AAA');
    expect(cache.get('layer', 'key-2')).toBeNull();
    expect(cache.get('other', 'key-1')).toBeNull();
  });

  it('evicts the least recently used entry past its bound', () => {
    const cache = new ThumbnailCache(3);
    cache.set('a', 'k', '1');
    cache.set('b', 'k', '2');
    cache.set('c', 'k', '3');
    // Touching "a" makes "b" the least recently used.
    expect(cache.get('a', 'k')).toBe('1');
    cache.set('d', 'k', '4');
    expect(cache.size).toBe(3);
    expect(cache.get('b', 'k')).toBeNull();
    expect(cache.get('a', 'k')).toBe('1');
    expect(cache.get('d', 'k')).toBe('4');
  });

  it('replaces an entry for the same layer rather than growing', () => {
    const cache = new ThumbnailCache();
    cache.set('a', 'k1', '1');
    cache.set('a', 'k2', '2');
    expect(cache.size).toBe(1);
    expect(cache.get('a', 'k2')).toBe('2');
  });

  it('tracks pending renders so the same key is not requested twice', () => {
    const cache = new ThumbnailCache();
    expect(cache.isPending('a', 'k')).toBe(false);
    cache.beginPending('a', 'k');
    expect(cache.isPending('a', 'k')).toBe(true);
    expect(cache.isPending('a', 'other')).toBe(false);
    cache.endPending('a', 'k');
    expect(cache.isPending('a', 'k')).toBe(false);
  });

  it('drops entries for layers that no longer exist', () => {
    const cache = new ThumbnailCache();
    cache.set('kept', 'k', '1');
    cache.set('gone', 'k', '2');
    expect(cache.retain(['kept'])).toBe(1);
    expect(cache.get('gone', 'k')).toBeNull();
    expect(cache.get('kept', 'k')).toBe('1');
  });

  it('clears everything on demand', () => {
    const cache = new ThumbnailCache();
    cache.set('a', 'k', '1');
    cache.beginPending('a', 'k');
    cache.clear();
    expect(cache.size).toBe(0);
    expect(cache.isPending('a', 'k')).toBe(false);
  });

  it('defaults to the documented bound', () => {
    const cache = new ThumbnailCache();
    for (let index = 0; index < MAX_THUMBNAIL_ENTRIES + 10; index += 1) {
      cache.set(`layer-${index}`, 'k', String(index));
    }
    expect(cache.size).toBe(MAX_THUMBNAIL_ENTRIES);
  });
});
