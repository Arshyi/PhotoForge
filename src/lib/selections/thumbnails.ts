import { decodeCoverage } from './overlays';
import type { MaskSnapshot } from './types';

const MAX_THUMBNAIL_EDGE = 256;
const MAX_MASK_PIXELS = 100_000_000;
const DEFAULT_MAX_ENTRIES = 96;
const DEFAULT_MAX_BYTES = 2 * 1024 * 1024;

export interface MaskThumbnailBitmap {
  readonly key: string;
  readonly width: number;
  readonly height: number;
  readonly pixels: Uint8ClampedArray;
}

interface CacheEntry {
  bitmap: MaskThumbnailBitmap;
  bytes: number;
}

export function thumbnailContentKey(
  mask: MaskSnapshot,
  targetWidth: number,
  targetHeight: number
): string {
  const [width, height] = normalizedTarget(targetWidth, targetHeight);
  return `${mask.checksum}:${mask.width}x${mask.height}@${width}x${height}`;
}

export function createMaskThumbnail(
  mask: MaskSnapshot,
  targetWidth: number,
  targetHeight: number
): MaskThumbnailBitmap {
  validateMaskDimensions(mask);
  const [maximumWidth, maximumHeight] = normalizedTarget(targetWidth, targetHeight);
  const scale = Math.min(maximumWidth / mask.width, maximumHeight / mask.height);
  const width = Math.max(1, Math.min(maximumWidth, Math.round(mask.width * scale)));
  const height = Math.max(1, Math.min(maximumHeight, Math.round(mask.height * scale)));
  const coverage = decodeCoverage(mask);
  const pixels = new Uint8ClampedArray(width * height * 4);

  for (let outputY = 0; outputY < height; outputY += 1) {
    const sourceTop = (outputY * mask.height) / height;
    const sourceBottom = ((outputY + 1) * mask.height) / height;
    const firstSourceY = Math.floor(sourceTop);
    const lastSourceY = Math.min(mask.height, Math.ceil(sourceBottom));

    for (let outputX = 0; outputX < width; outputX += 1) {
      const sourceLeft = (outputX * mask.width) / width;
      const sourceRight = ((outputX + 1) * mask.width) / width;
      const firstSourceX = Math.floor(sourceLeft);
      const lastSourceX = Math.min(mask.width, Math.ceil(sourceRight));
      let weightedCoverage = 0;
      let totalWeight = 0;

      for (let sourceY = firstSourceY; sourceY < lastSourceY; sourceY += 1) {
        const yWeight = Math.min(sourceBottom, sourceY + 1) - Math.max(sourceTop, sourceY);
        for (let sourceX = firstSourceX; sourceX < lastSourceX; sourceX += 1) {
          const xWeight = Math.min(sourceRight, sourceX + 1) - Math.max(sourceLeft, sourceX);
          const weight = xWeight * yWeight;
          weightedCoverage += coverage[sourceY * mask.width + sourceX] * weight;
          totalWeight += weight;
        }
      }

      const value = Math.round(weightedCoverage / Math.max(Number.EPSILON, totalWeight));
      const outputIndex = (outputY * width + outputX) * 4;
      pixels.set([value, value, value, 255], outputIndex);
    }
  }

  return {
    key: thumbnailContentKey(mask, maximumWidth, maximumHeight),
    width,
    height,
    pixels
  };
}

export class MaskThumbnailCache {
  private readonly entries = new Map<string, CacheEntry>();
  private retainedBytes = 0;

  constructor(
    private readonly maxEntries = DEFAULT_MAX_ENTRIES,
    private readonly maxBytes = DEFAULT_MAX_BYTES
  ) {
    if (!Number.isInteger(maxEntries) || maxEntries < 1) {
      throw new Error('Thumbnail cache entry limit must be a positive integer.');
    }
    if (!Number.isInteger(maxBytes) || maxBytes < 1) {
      throw new Error('Thumbnail cache byte limit must be a positive integer.');
    }
  }

  get(mask: MaskSnapshot, targetWidth: number, targetHeight: number): MaskThumbnailBitmap {
    const key = thumbnailContentKey(mask, targetWidth, targetHeight);
    const cached = this.entries.get(key);
    if (cached) {
      this.entries.delete(key);
      this.entries.set(key, cached);
      return cached.bitmap;
    }

    const bitmap = createMaskThumbnail(mask, targetWidth, targetHeight);
    const bytes = bitmap.pixels.byteLength + key.length * 2;
    if (bytes > this.maxBytes) return bitmap;

    this.entries.set(key, { bitmap, bytes });
    this.retainedBytes += bytes;
    this.evictToLimits();
    return bitmap;
  }

  has(key: string): boolean {
    return this.entries.has(key);
  }

  clear(): void {
    this.entries.clear();
    this.retainedBytes = 0;
  }

  get size(): number {
    return this.entries.size;
  }

  get bytes(): number {
    return this.retainedBytes;
  }

  private evictToLimits(): void {
    while (this.entries.size > this.maxEntries || this.retainedBytes > this.maxBytes) {
      const oldestKey = this.entries.keys().next().value as string | undefined;
      if (!oldestKey) break;
      const oldest = this.entries.get(oldestKey);
      this.entries.delete(oldestKey);
      if (oldest) this.retainedBytes -= oldest.bytes;
    }
  }
}

export const maskThumbnailCache = new MaskThumbnailCache();

function normalizedTarget(width: number, height: number): [number, number] {
  if (!Number.isFinite(width) || !Number.isFinite(height)) {
    throw new Error('Thumbnail dimensions must be finite.');
  }
  return [
    Math.max(1, Math.min(MAX_THUMBNAIL_EDGE, Math.round(width))),
    Math.max(1, Math.min(MAX_THUMBNAIL_EDGE, Math.round(height)))
  ];
}

function validateMaskDimensions(mask: MaskSnapshot): void {
  const pixels = mask.width * mask.height;
  if (
    !Number.isInteger(mask.width) ||
    !Number.isInteger(mask.height) ||
    mask.width < 1 ||
    mask.height < 1 ||
    !Number.isSafeInteger(pixels) ||
    pixels > MAX_MASK_PIXELS
  ) {
    throw new Error('Mask dimensions are invalid for thumbnail generation.');
  }
}
