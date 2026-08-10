import { describe, expect, it } from 'vitest';
import type { MaskSnapshot } from './types';
import {
  createMaskThumbnail,
  MaskThumbnailCache,
  thumbnailContentKey
} from './thumbnails';

function mask(
  width: number,
  height: number,
  values: number[],
  checksum = `fnv1a64:${values.reduce((value, item) => value + item, 0).toString(16).padStart(16, '0')}`
): MaskSnapshot {
  return {
    version: 1,
    width,
    height,
    encoding: 'base64_u8',
    data: btoa(String.fromCharCode(...values)).replace(/=+$/, ''),
    checksum
  };
}

describe('mask thumbnails', () => {
  it('renders real grayscale coverage while preserving aspect ratio', () => {
    const thumbnail = createMaskThumbnail(mask(4, 2, [0, 64, 128, 255, 255, 128, 64, 0]), 8, 8);

    expect([thumbnail.width, thumbnail.height]).toEqual([8, 4]);
    expect([...thumbnail.pixels.slice(0, 4)]).toEqual([0, 0, 0, 255]);
    expect([...thumbnail.pixels.slice(-4)]).toEqual([0, 0, 0, 255]);
    expect(new Set([...thumbnail.pixels].filter((_value, index) => index % 4 !== 3)).size).toBeGreaterThan(2);
  });

  it('area-averages coverage when reducing a mask', () => {
    const thumbnail = createMaskThumbnail(mask(2, 2, [0, 64, 128, 255]), 1, 1);
    expect([...thumbnail.pixels]).toEqual([112, 112, 112, 255]);
  });

  it('keys only content geometry and requested bounds', () => {
    const value = mask(2, 1, [0, 255], 'fnv1a64:1111111111111111');
    expect(thumbnailContentKey(value, 40, 32)).toBe(thumbnailContentKey(value, 40, 32));
    expect(thumbnailContentKey({ ...value, checksum: 'fnv1a64:2222222222222222' }, 40, 32))
      .not.toBe(thumbnailContentKey(value, 40, 32));
    expect(thumbnailContentKey(value, 48, 32)).not.toBe(thumbnailContentKey(value, 40, 32));
  });

  it('uses bounded least-recently-used eviction and refreshes recency on hits', () => {
    const cache = new MaskThumbnailCache(2, 4096);
    const first = mask(1, 1, [10], 'fnv1a64:0000000000000001');
    const second = mask(1, 1, [20], 'fnv1a64:0000000000000002');
    const third = mask(1, 1, [30], 'fnv1a64:0000000000000003');
    const firstKey = thumbnailContentKey(first, 4, 4);
    const secondKey = thumbnailContentKey(second, 4, 4);

    const firstBitmap = cache.get(first, 4, 4);
    cache.get(second, 4, 4);
    expect(cache.get(first, 4, 4)).toBe(firstBitmap);
    cache.get(third, 4, 4);

    expect(cache.size).toBe(2);
    expect(cache.has(firstKey)).toBe(true);
    expect(cache.has(secondKey)).toBe(false);
    expect(cache.bytes).toBeLessThanOrEqual(4096);
  });

  it('does not retain an entry larger than the byte budget', () => {
    const cache = new MaskThumbnailCache(4, 32);
    const value = mask(2, 2, [0, 64, 128, 255]);
    const thumbnail = cache.get(value, 8, 8);

    expect(thumbnail.pixels.byteLength).toBeGreaterThan(32);
    expect(cache.size).toBe(0);
    expect(cache.bytes).toBe(0);
  });
});
