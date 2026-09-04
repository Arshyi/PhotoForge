import type { Layer, LayerDocument } from './types';
import { childrenOf } from './tree';

/** Most thumbnails held at once. Older entries are evicted least-recently-used. */
export const MAX_THUMBNAIL_ENTRIES = 96;

export interface ThumbnailEntry {
  dataUrl: string;
  key: string;
}

/**
 * Identifies everything about a layer that can change its thumbnail.
 *
 * The key deliberately excludes name, lock state, and selection, so renaming a
 * layer or selecting it never triggers a re-render. It includes the mask
 * checksum rather than the mask payload, so comparing keys stays cheap.
 */
export function thumbnailKey(layer: Layer, document: LayerDocument): string {
  const parts: string[] = [layer.id, document.precision ?? 'legacy_srgb8'];
  const content = layer.content;
  if (content.type === 'pixel') {
    parts.push('p', content.pixelId, String(content.width), String(content.height));
  } else if (content.type === 'adjustment') {
    parts.push('a', content.operation.type);
  } else {
    // A group thumbnail is the composite of its children, so every visible
    // property of every descendant contributes to the key.
    parts.push('g');
    const stack = [...childrenOf(layer)];
    while (stack.length) {
      const child = stack.pop() as Layer;
      parts.push(childSignature(child));
      stack.push(...childrenOf(child));
    }
  }
  parts.push(
    layer.visible ? 'v' : '-',
    layer.opacity.toFixed(3),
    layer.blendMode,
    transformSignature(layer),
    maskSignature(layer),
    `${document.canvasWidth}x${document.canvasHeight}`
  );
  return parts.join('|');
}

function childSignature(layer: Layer): string {
  const content = layer.content;
  const identity =
    content.type === 'pixel'
      ? `${content.pixelId}:${content.width}x${content.height}`
      : content.type === 'adjustment'
        ? JSON.stringify(content.operation)
        : 'group';
  return [
    layer.id,
    identity,
    layer.visible ? 'v' : '-',
    layer.opacity.toFixed(3),
    layer.blendMode,
    transformSignature(layer),
    maskSignature(layer)
  ].join(',');
}

function transformSignature(layer: Layer): string {
  const t = layer.transform;
  return [
    t.translateX,
    t.translateY,
    t.scaleX,
    t.scaleY,
    t.rotationDegrees,
    t.flipHorizontal ? 1 : 0,
    t.flipVertical ? 1 : 0,
    t.interpolation ?? 'bilinear'
  ].join(':');
}

function maskSignature(layer: Layer): string {
  if (!layer.mask) return 'nomask';
  return `${layer.mask.snapshot.checksum}:${layer.mask.enabled ? 1 : 0}:${layer.mask.inverted ? 1 : 0}`;
}

/**
 * A bounded least-recently-used thumbnail cache.
 *
 * Panel redraws read from here; only a key change schedules a real render, so
 * scrolling or reselecting never re-renders pixels.
 */
export class ThumbnailCache {
  private entries = new Map<string, ThumbnailEntry>();
  private pending = new Set<string>();

  constructor(private readonly maxEntries = MAX_THUMBNAIL_ENTRIES) {}

  get size(): number {
    return this.entries.size;
  }

  /** Returns the cached thumbnail only when it matches the layer's current key. */
  get(layerId: string, key: string): string | null {
    const entry = this.entries.get(layerId);
    if (!entry || entry.key !== key) return null;
    // Refresh recency.
    this.entries.delete(layerId);
    this.entries.set(layerId, entry);
    return entry.dataUrl;
  }

  set(layerId: string, key: string, dataUrl: string): void {
    if (this.entries.has(layerId)) this.entries.delete(layerId);
    this.entries.set(layerId, { key, dataUrl });
    while (this.entries.size > this.maxEntries) {
      const oldest = this.entries.keys().next();
      if (oldest.done) break;
      this.entries.delete(oldest.value);
    }
  }

  /** True when a render for this exact key is already in flight. */
  isPending(layerId: string, key: string): boolean {
    return this.pending.has(`${layerId}#${key}`);
  }

  beginPending(layerId: string, key: string): void {
    this.pending.add(`${layerId}#${key}`);
  }

  endPending(layerId: string, key: string): void {
    this.pending.delete(`${layerId}#${key}`);
  }

  /** Drops cached thumbnails for layers the document no longer contains. */
  retain(layerIds: Iterable<string>): number {
    const keep = new Set(layerIds);
    let removed = 0;
    for (const id of [...this.entries.keys()]) {
      if (!keep.has(id)) {
        this.entries.delete(id);
        removed += 1;
      }
    }
    return removed;
  }

  clear(): void {
    this.entries.clear();
    this.pending.clear();
  }
}
