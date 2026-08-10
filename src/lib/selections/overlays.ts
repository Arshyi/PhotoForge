import type { MaskSnapshot, OverlaySettings } from './types';

const MAX_OVERLAY_CACHE_ENTRIES = 12;
const MAX_OVERLAY_CACHE_BYTES = 64 * 1024 * 1024;
const MAX_VISIBLE_MASKS = 100;
const MAX_MASK_PIXELS = 100_000_000;

interface CoverageEntry {
  coverage: Uint8Array;
  bytes: number;
}

const coverageCache = new Map<string, CoverageEntry>();
let coverageCacheBytes = 0;

export function decodeCoverage(mask: MaskSnapshot): Uint8Array {
  if (!['base64_u8', 'base64_rle_u8'].includes(mask.encoding)) {
    throw new Error(`Unsupported mask encoding: ${mask.encoding}`);
  }
  const binary = atob(mask.data);
  const expected = mask.width * mask.height;
  const coverage = new Uint8Array(expected);
  if (mask.encoding === 'base64_u8') {
    if (binary.length !== expected) throw new Error('Mask data does not match its dimensions.');
    for (let index = 0; index < expected; index += 1) coverage[index] = binary.charCodeAt(index);
    return coverage;
  }
  if (!binary.length || binary.length % 5 !== 0) throw new Error('Run-length mask data is malformed.');
  let output = 0;
  for (let index = 0; index < binary.length; index += 5) {
    const count =
      binary.charCodeAt(index + 1) |
      (binary.charCodeAt(index + 2) << 8) |
      (binary.charCodeAt(index + 3) << 16) |
      (binary.charCodeAt(index + 4) << 24);
    if (count <= 0 || output + count > expected) throw new Error('Run-length mask exceeds its dimensions.');
    coverage.fill(binary.charCodeAt(index), output, output + count);
    output += count;
  }
  if (output !== expected) throw new Error('Run-length mask data does not match its dimensions.');
  return coverage;
}

export function overlayPixels(
  mask: MaskSnapshot,
  settings: OverlaySettings,
  outputWidth = mask.width,
  outputHeight = mask.height,
  animationFrame = 0
): Uint8ClampedArray {
  return overlayPixelsFromCoverage(
    cachedCoverage([mask]),
    mask.width,
    mask.height,
    settings,
    outputWidth,
    outputHeight,
    animationFrame
  );
}

export function overlayPixelsForMasks(
  masks: MaskSnapshot[],
  settings: OverlaySettings,
  outputWidth: number,
  outputHeight: number,
  animationFrame = 0
): Uint8ClampedArray {
  if (!masks.length) return new Uint8ClampedArray(outputWidth * outputHeight * 4);
  if (masks.length > MAX_VISIBLE_MASKS) {
    throw new Error(`Visible masks are limited to ${MAX_VISIBLE_MASKS}.`);
  }
  const { width, height } = masks[0];
  if (masks.some((mask) => mask.width !== width || mask.height !== height)) {
    throw new Error('Visible masks do not share the current canvas dimensions.');
  }
  return overlayPixelsFromCoverage(
    cachedCoverage(masks),
    width,
    height,
    settings,
    outputWidth,
    outputHeight,
    animationFrame
  );
}

function overlayPixelsFromCoverage(
  coverage: Uint8Array,
  sourceWidth: number,
  sourceHeight: number,
  settings: OverlaySettings,
  outputWidth: number,
  outputHeight: number,
  animationFrame: number
): Uint8ClampedArray {
  const output = new Uint8ClampedArray(outputWidth * outputHeight * 4);
  const color = parseColor(settings.color);
  for (let y = 0; y < outputHeight; y += 1) {
    const sourceY = Math.min(sourceHeight - 1, Math.floor(((y + 0.5) * sourceHeight) / outputHeight));
    for (let x = 0; x < outputWidth; x += 1) {
      const sourceX = Math.min(sourceWidth - 1, Math.floor(((x + 0.5) * sourceWidth) / outputWidth));
      const sourceIndex = sourceY * sourceWidth + sourceX;
      const value = coverage[sourceIndex];
      const outputIndex = (y * outputWidth + x) * 4;
      if (settings.mode === 'marching_ants') {
        if (!boundary(coverage, sourceWidth, sourceHeight, sourceX, sourceY)) continue;
        const white = (x + y + animationFrame) % 8 < 4;
        output.set([white ? 255 : 0, white ? 255 : 0, white ? 255 : 0, 255], outputIndex);
      } else if (settings.mode === 'color') {
        output.set(
          [color[0], color[1], color[2], Math.round(value * settings.opacity)],
          outputIndex
        );
      } else if (settings.mode === 'grayscale' || settings.mode === 'mask_only') {
        output.set(
          [value, value, value, settings.mode === 'mask_only' ? 255 : Math.round(255 * settings.opacity)],
          outputIndex
        );
      } else {
        const channel = settings.mode === 'white' ? 255 : 0;
        output.set([channel, channel, channel, Math.round((255 - value) * settings.opacity)], outputIndex);
      }
    }
  }
  return output;
}

function cachedCoverage(masks: MaskSnapshot[]): Uint8Array {
  const orderedKeys = masks
    .map((mask) => `${mask.checksum}:${mask.width}x${mask.height}`)
    .sort();
  const key = orderedKeys.join('|');
  const existing = coverageCache.get(key);
  if (existing) {
    coverageCache.delete(key);
    coverageCache.set(key, existing);
    return existing.coverage;
  }
  const pixels = masks[0].width * masks[0].height;
  if (!Number.isSafeInteger(pixels) || pixels < 1 || pixels > MAX_MASK_PIXELS) {
    throw new Error('Visible mask dimensions exceed the bounded overlay limit.');
  }
  let coverage: Uint8Array | undefined;
  for (const mask of masks) {
    const decoded = decodeCoverage(mask);
    if (!coverage) coverage = decoded;
    else mergeCoverageMaximum(coverage, decoded);
  }
  if (!coverage) throw new Error('Visible mask coverage is unavailable.');
  const bytes = coverage.byteLength + key.length * 2;
  if (bytes <= MAX_OVERLAY_CACHE_BYTES) {
    coverageCache.set(key, { coverage, bytes });
    coverageCacheBytes += bytes;
    while (
      coverageCache.size > MAX_OVERLAY_CACHE_ENTRIES ||
      coverageCacheBytes > MAX_OVERLAY_CACHE_BYTES
    ) {
      const oldestKey = coverageCache.keys().next().value as string | undefined;
      if (!oldestKey) break;
      const removed = coverageCache.get(oldestKey);
      coverageCache.delete(oldestKey);
      if (removed) coverageCacheBytes -= removed.bytes;
    }
  }
  return coverage;
}

function mergeCoverageMaximum(output: Uint8Array, value: Uint8Array): void {
  if (value.length !== output.length) throw new Error('Visible mask coverage lengths differ.');
  for (let index = 0; index < output.length; index += 1) {
    if (value[index] > output[index]) output[index] = value[index];
  }
}

export function clearOverlayCoverageCache(): void {
  coverageCache.clear();
  coverageCacheBytes = 0;
}

export function drawMaskOverlay(
  canvas: HTMLCanvasElement,
  mask: MaskSnapshot | null,
  settings: OverlaySettings,
  width: number,
  height: number,
  animationFrame = 0
): void {
  canvas.width = Math.max(1, width);
  canvas.height = Math.max(1, height);
  const context = canvas.getContext('2d');
  if (!context) return;
  context.clearRect(0, 0, canvas.width, canvas.height);
  if (!mask || !settings.visible) return;
  context.putImageData(
    new ImageData(overlayPixels(mask, settings, canvas.width, canvas.height, animationFrame), canvas.width),
    0,
    0
  );
}

export function drawMaskOverlays(
  canvas: HTMLCanvasElement,
  masks: MaskSnapshot[],
  settings: OverlaySettings,
  width: number,
  height: number,
  animationFrame = 0
): void {
  canvas.width = Math.max(1, width);
  canvas.height = Math.max(1, height);
  const context = canvas.getContext('2d');
  if (!context) return;
  context.clearRect(0, 0, canvas.width, canvas.height);
  if (!masks.length || !settings.visible) return;
  context.putImageData(
    new ImageData(
      overlayPixelsForMasks(masks, settings, canvas.width, canvas.height, animationFrame),
      canvas.width
    ),
    0,
    0
  );
}

function boundary(data: Uint8Array, width: number, height: number, x: number, y: number): boolean {
  const selected = data[y * width + x] >= 128;
  const neighbors = [
    x > 0 ? data[y * width + x - 1] >= 128 : false,
    x + 1 < width ? data[y * width + x + 1] >= 128 : false,
    y > 0 ? data[(y - 1) * width + x] >= 128 : false,
    y + 1 < height ? data[(y + 1) * width + x] >= 128 : false
  ];
  return neighbors.some((value) => value !== selected);
}

function parseColor(value: string): [number, number, number] {
  const match = /^#([0-9a-f]{6})$/i.exec(value);
  if (!match) return [239, 91, 91];
  return [
    Number.parseInt(match[1].slice(0, 2), 16),
    Number.parseInt(match[1].slice(2, 4), 16),
    Number.parseInt(match[1].slice(4, 6), 16)
  ];
}
