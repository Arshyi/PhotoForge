import { decodedCoverageChecksum } from '../lib/selections/checksum';
import type { MaskSnapshot } from '../lib/selections/types';

/** A real 1x1 transparent PNG, used to prove the panel renders an <img>. */
export const transparentPixel =
  'data:image/png;base64,iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAYAAAAfFcSJAAAADUlEQVR42mP8z8BQDwAEhQGAhKmMIQAAAABJRU5ErkJggg==';

/**
 * Builds a genuine `MaskSnapshot` in TypeScript.
 *
 * The coverage bytes are real, the base64 is real, and the checksum comes from
 * the same `decodedCoverageChecksum` the application uses, so the mask decodes
 * and renders through the ordinary path. This substitutes for the backend
 * command that would normally produce it; it does not simulate that command.
 */
export function buildMask(width: number, height: number, value: number): MaskSnapshot {
  const coverage = new Uint8Array(width * height).fill(value);
  let binary = '';
  for (const byte of coverage) binary += String.fromCharCode(byte);
  const snapshot: MaskSnapshot = {
    version: 1,
    width,
    height,
    encoding: 'base64_u8',
    data: btoa(binary),
    checksum: 'fnv1a64:0000000000000000'
  };
  const checksum = decodedCoverageChecksum(snapshot);
  if (!checksum) throw new Error('harness mask fixture failed its own checksum');
  return { ...snapshot, checksum };
}
