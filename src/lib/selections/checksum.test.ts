import { describe, expect, it } from 'vitest';
import { decodedCoverageChecksum, hasValidDecodedCoverageChecksum } from './checksum';
import type { MaskSnapshot } from './types';

function raw(values: number[]): MaskSnapshot {
  const data = btoa(String.fromCharCode(...values));
  const mask: MaskSnapshot = {
    version: 1,
    width: values.length,
    height: 1,
    encoding: 'base64_u8',
    data,
    checksum: ''
  };
  mask.checksum = decodedCoverageChecksum(mask) as string;
  return mask;
}

describe('decoded mask coverage checksum', () => {
  it('matches FNV-1a64 coverage for raw and RLE without expanding RLE', () => {
    const mask = raw([0, 64, 128, 255]);
    expect(mask.checksum).toBe('fnv1a64:2478c77e653e6798');
    expect(hasValidDecodedCoverageChecksum(mask)).toBe(true);

    const rle: MaskSnapshot = {
      ...mask,
      width: 4,
      encoding: 'base64_rle_u8',
      data: btoa(String.fromCharCode(255, 4, 0, 0, 0))
    };
    rle.checksum = decodedCoverageChecksum(rle) as string;
    expect(rle.checksum).toBe(decodedCoverageChecksum(raw([255, 255, 255, 255])));
  });

  it('rejects a well-shaped but false checksum and malformed runs', () => {
    expect(hasValidDecodedCoverageChecksum({
      ...raw([1, 2, 3]),
      checksum: 'fnv1a64:0123456789abcdef'
    })).toBe(false);
    expect(decodedCoverageChecksum({
      ...raw([1]),
      encoding: 'base64_rle_u8',
      data: btoa(String.fromCharCode(1, 2, 0, 0, 0))
    })).toBeNull();
  });
});
