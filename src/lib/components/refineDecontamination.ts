const CONFIDENT_FOREGROUND_COVERAGE = 224;
const MAX_PREVIEW_PIXELS = 65_536;
const MAX_NEIGHBORHOOD_PIXEL_VISITS = 128_000_000;
const STRENGTH_SCALE = 65_535;

export interface DecontaminatePreviewOptions {
  enabled: boolean;
  strength: number;
  radius: number;
}

/**
 * Produces a bounded, deterministic dialog-only RGB preview using the same
 * circular, distance-weighted foreground sampling and integer blending as the
 * native full-resolution operation. Inputs are immutable and alpha is copied.
 */
export function decontaminatePreviewPixels(
  source: Uint8ClampedArray,
  maskPixels: Uint8ClampedArray,
  width: number,
  height: number,
  options: DecontaminatePreviewOptions
): Uint8ClampedArray {
  const pixelCount = width * height;
  if (
    !Number.isInteger(width) ||
    !Number.isInteger(height) ||
    width < 1 ||
    height < 1 ||
    !Number.isSafeInteger(pixelCount) ||
    pixelCount > MAX_PREVIEW_PIXELS ||
    source.length !== pixelCount * 4 ||
    maskPixels.length !== pixelCount * 4
  ) {
    throw new Error('Decontamination preview dimensions are invalid.');
  }
  const output = new Uint8ClampedArray(source);
  if (!options.enabled) return output;

  const strength = Number.isFinite(options.strength)
    ? Math.max(0, Math.min(1, Math.fround(options.strength)))
    : 0;
  if (strength === 0) return output;
  const radius = Number.isFinite(options.radius)
    ? Math.max(1, Math.min(32, Math.round(options.radius)))
    : 4;
  const diameter = radius * 2 + 1;
  let edgePixels = 0;
  for (let index = 0; index < maskPixels.length; index += 4) {
    const coverage = maskPixels[index];
    if (coverage > 0 && coverage < CONFIDENT_FOREGROUND_COVERAGE) edgePixels += 1;
  }
  if (edgePixels * diameter * diameter > MAX_NEIGHBORHOOD_PIXEL_VISITS) {
    throw new Error('Decontamination preview exceeds its bounded work limit.');
  }

  const strengthWeight = Math.round(Math.fround(strength * STRENGTH_SCALE));
  const radiusSquared = radius * radius;
  for (let y = 0; y < height; y += 1) {
    for (let x = 0; x < width; x += 1) {
      const pixelIndex = (y * width + x) * 4;
      const coverage = maskPixels[pixelIndex];
      if (coverage === 0 || coverage >= CONFIDENT_FOREGROUND_COVERAGE) continue;

      const xMin = Math.max(0, x - radius);
      const yMin = Math.max(0, y - radius);
      const xMax = Math.min(width - 1, x + radius);
      const yMax = Math.min(height - 1, y + radius);
      const channelSums = [0, 0, 0];
      let totalWeight = 0;
      for (let sampleY = yMin; sampleY <= yMax; sampleY += 1) {
        for (let sampleX = xMin; sampleX <= xMax; sampleX += 1) {
          const dx = Math.abs(sampleX - x);
          const dy = Math.abs(sampleY - y);
          const distanceSquared = dx * dx + dy * dy;
          if (distanceSquared > radiusSquared) continue;
          const sampleIndex = (sampleY * width + sampleX) * 4;
          const sampleCoverage = maskPixels[sampleIndex];
          if (sampleCoverage < CONFIDENT_FOREGROUND_COVERAGE || source[sampleIndex + 3] === 0) {
            continue;
          }
          const weight = sampleCoverage * (radiusSquared + 1 - distanceSquared);
          channelSums[0] += source[sampleIndex] * weight;
          channelSums[1] += source[sampleIndex + 1] * weight;
          channelSums[2] += source[sampleIndex + 2] * weight;
          totalWeight += weight;
        }
      }
      if (totalWeight === 0) continue;

      for (let channel = 0; channel < 3; channel += 1) {
        const foreground = Math.floor((channelSums[channel] + totalWeight / 2) / totalWeight);
        output[pixelIndex + channel] = Math.floor(
          (source[pixelIndex + channel] * (STRENGTH_SCALE - strengthWeight) +
            foreground * strengthWeight + STRENGTH_SCALE / 2) /
            STRENGTH_SCALE
        );
      }
    }
  }
  return output;
}
