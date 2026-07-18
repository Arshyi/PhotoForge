import { render } from '@testing-library/svelte';
import { describe, expect, it } from 'vitest';
import AnalysisPanel from './AnalysisPanel.svelte';

const analysis = {
  averageLuminance: 0.25,
  luminanceSpread: 0.6,
  estimatedColorCast: {
    dominant: 'warm' as const,
    redBias: 0.12,
    greenBias: 0,
    blueBias: -0.12
  },
  estimatedNoise: 0.3,
  estimatedSharpness: 0.02,
  estimatedLocalContrast: 0.02,
  edgeDensity: 0.08,
  whiteBackgroundRatio: 0.5,
  likelyDocument: true
};

describe('AnalysisPanel', () => {
  it('renders cautious heuristic observations without applying edits', () => {
    const view = render(AnalysisPanel, { props: { analysis, analyzing: false } });
    expect(view.getByText('Image appears slightly dark')).toBeTruthy();
    expect(view.getByText('Warm color cast detected')).toBeTruthy();
    expect(view.getByText('Moderate high-frequency noise estimate')).toBeTruthy();
    expect(view.getByText(/Nothing is applied automatically/)).toBeTruthy();
  });

  it('exposes a processing state', () => {
    const view = render(AnalysisPanel, { props: { analysis: null, analyzing: true } });
    expect(view.getByText('Inspecting local pixels…')).toBeTruthy();
    expect(view.getByRole('region', { name: /Image Analysis/ }).getAttribute('aria-busy')).toBe('true');
  });
});
