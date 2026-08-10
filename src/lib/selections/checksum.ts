import type { MaskSnapshot } from './types';

const FNV_OFFSET_HIGH = 0xcbf29ce4;
const FNV_OFFSET_LOW = 0x84222325;
const FNV_PRIME_HIGH = 0x100;
const FNV_PRIME_LOW = 0x1b3;

export function decodedCoverageChecksum(mask: MaskSnapshot): string | null {
  let binary: string;
  try {
    binary = atob(mask.data);
  } catch {
    return null;
  }
  const expected = mask.width * mask.height;
  let high = FNV_OFFSET_HIGH;
  let low = FNV_OFFSET_LOW;
  const update = (value: number) => {
    low = (low ^ value) >>> 0;
    const lowProduct = low * FNV_PRIME_LOW;
    const carry = Math.floor(lowProduct / 0x1_0000_0000);
    high = (Math.imul(high, FNV_PRIME_LOW) + Math.imul(low, FNV_PRIME_HIGH) + carry) >>> 0;
    low = lowProduct >>> 0;
  };

  if (mask.encoding === 'base64_u8') {
    if (binary.length !== expected) return null;
    for (let index = 0; index < binary.length; index += 1) update(binary.charCodeAt(index));
  } else if (mask.encoding === 'base64_rle_u8') {
    if (!binary.length || binary.length % 5 !== 0) return null;
    let output = 0;
    for (let index = 0; index < binary.length; index += 5) {
      const count = (
        binary.charCodeAt(index + 1) +
        binary.charCodeAt(index + 2) * 0x100 +
        binary.charCodeAt(index + 3) * 0x1_0000 +
        binary.charCodeAt(index + 4) * 0x100_0000
      );
      if (!Number.isSafeInteger(count) || count < 1 || output + count > expected) return null;
      const value = binary.charCodeAt(index);
      for (let run = 0; run < count; run += 1) update(value);
      output += count;
    }
    if (output !== expected) return null;
  } else {
    return null;
  }

  return `fnv1a64:${high.toString(16).padStart(8, '0')}${low.toString(16).padStart(8, '0')}`;
}

export function hasValidDecodedCoverageChecksum(mask: MaskSnapshot): boolean {
  return decodedCoverageChecksum(mask) === mask.checksum;
}
