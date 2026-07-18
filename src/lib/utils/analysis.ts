import type { ImageQualityAnalysis } from '../types/editor';

export function analysisObservations(analysis: ImageQualityAnalysis): string[] {
  const observations: string[] = [];
  if (analysis.averageLuminance < 0.34) observations.push('Image appears slightly dark');
  else if (analysis.averageLuminance > 0.78) observations.push('Image appears quite bright');

  if (analysis.estimatedColorCast.dominant === 'warm') observations.push('Warm color cast detected');
  else if (analysis.estimatedColorCast.dominant === 'cool') observations.push('Cool color cast detected');
  else if (analysis.estimatedColorCast.dominant === 'green') observations.push('Green color cast detected');

  if (analysis.estimatedNoise > 0.24) observations.push('Moderate high-frequency noise estimate');
  if (analysis.estimatedSharpness < 0.025 && analysis.luminanceSpread > 0.15) {
    observations.push('Image may be slightly soft');
  }
  if (analysis.estimatedLocalContrast < 0.035 && analysis.luminanceSpread > 0.1) {
    observations.push('Local contrast appears restrained');
  }
  if (analysis.likelyDocument) observations.push('Possible photographed document');
  if (observations.length === 0) observations.push('No strong restoration need detected');
  return observations.slice(0, 4);
}
