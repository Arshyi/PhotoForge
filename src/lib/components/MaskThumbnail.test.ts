import { render, waitFor } from '@testing-library/svelte';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { MaskThumbnailCache } from '../selections/thumbnails';
import type { MaskSnapshot } from '../selections/types';
import MaskThumbnail from './MaskThumbnail.svelte';

const mask: MaskSnapshot = {
  version: 1,
  width: 2,
  height: 1,
  encoding: 'base64_u8',
  data: 'AP8',
  checksum: 'fnv1a64:0123456789abcdef'
};

let putImageData: ReturnType<typeof vi.fn>;

beforeEach(() => {
  putImageData = vi.fn();
  vi.spyOn(HTMLCanvasElement.prototype, 'getContext').mockImplementation(() => ({
    createImageData: (width: number, height: number) => ({
      data: new Uint8ClampedArray(width * height * 4),
      width,
      height
    }),
    putImageData
  }) as unknown as CanvasRenderingContext2D);
});

afterEach(() => {
  vi.restoreAllMocks();
  vi.unstubAllGlobals();
});

describe('MaskThumbnail', () => {
  it('reliably renders without IntersectionObserver support', async () => {
    vi.stubGlobal('IntersectionObserver', undefined);
    const cache = new MaskThumbnailCache(4, 4096);
    const view = render(MaskThumbnail, {
      props: { mask, label: 'Subject', targetWidth: 8, targetHeight: 8, cache }
    });

    expect(view.getByRole('img', { name: 'Mask thumbnail for Subject' })).toBeTruthy();
    await waitFor(() => expect(putImageData).toHaveBeenCalledTimes(1));
    const canvas = view.container.querySelector('canvas') as HTMLCanvasElement;
    expect([canvas.width, canvas.height]).toEqual([8, 4]);
    expect(cache.size).toBe(1);
  });

  it('defers generation until the thumbnail intersects the viewport', async () => {
    let intersectionCallback: IntersectionObserverCallback | undefined;
    const disconnect = vi.fn();
    class TestIntersectionObserver {
      readonly root = null;
      readonly rootMargin = '0px';
      readonly thresholds = [0];
      constructor(callback: IntersectionObserverCallback) {
        intersectionCallback = callback;
      }
      observe() {}
      unobserve() {}
      disconnect() { disconnect(); }
      takeRecords(): IntersectionObserverEntry[] { return []; }
    }
    vi.stubGlobal('IntersectionObserver', TestIntersectionObserver);
    const cache = new MaskThumbnailCache(4, 4096);
    render(MaskThumbnail, { props: { mask, label: 'Sky', cache } });

    await Promise.resolve();
    expect(cache.size).toBe(0);
    expect(putImageData).not.toHaveBeenCalled();

    intersectionCallback?.(
      [{ isIntersecting: true, intersectionRatio: 1 } as IntersectionObserverEntry],
      {} as IntersectionObserver
    );
    await waitFor(() => expect(putImageData).toHaveBeenCalledTimes(1));
    expect(cache.size).toBe(1);
    expect(disconnect).toHaveBeenCalled();
  });
});
