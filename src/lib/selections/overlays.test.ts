import { describe, expect, it } from 'vitest';
import { decodeCoverage, overlayPixels } from './overlays';
import type { MaskSnapshot, OverlaySettings } from './types';

const mask: MaskSnapshot = {
  version: 1,
  width: 2,
  height: 2,
  encoding: 'base64_u8',
  data: btoa(String.fromCharCode(0, 64, 128, 255)).replace(/=+$/, ''),
  checksum: 'fnv1a64:0123456789abcdef'
};
const settings: OverlaySettings = { visible: true, mode: 'color', opacity: 0.5, color: '#ff0000' };

describe('mask overlays', () => {
  it('decodes normalized coverage exactly', () => {
    expect([...decodeCoverage(mask)]).toEqual([0, 64, 128, 255]);
  });

  it('renders color alpha from coverage without changing image data', () => {
    const pixels = overlayPixels(mask, settings);
    expect([...pixels.slice(0, 4)]).toEqual([255, 0, 0, 0]);
    expect([...pixels.slice(12, 16)]).toEqual([255, 0, 0, 128]);
  });

  it('renders mask-only grayscale and bounded resampling', () => {
    const pixels = overlayPixels(mask, { ...settings, mode: 'mask_only' }, 1, 1);
    expect([...pixels]).toEqual([255, 255, 255, 255]);
  });

  it('rejects malformed coverage length', () => {
    expect(() => decodeCoverage({ ...mask, data: 'AA' })).toThrow(/dimensions/i);
  });

  it('decodes bounded run-length snapshots', () => {
    const encoded = btoa(String.fromCharCode(255, 4, 0, 0, 0)).replace(/=+$/, '');
    expect([...decodeCoverage({ ...mask, encoding: 'base64_rle_u8', data: encoded })]).toEqual([
      255, 255, 255, 255
    ]);
  });
});
