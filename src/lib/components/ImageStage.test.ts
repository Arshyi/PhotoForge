import { render } from '@testing-library/svelte';
import { describe, expect, it, vi } from 'vitest';
import ImageStage from './ImageStage.svelte';

const baseProps = {
  originalUrl: 'data:image/png;base64,AA==',
  previewUrl: 'data:image/png;base64,AA==',
  filename: 'fixture.png',
  zoom: 100,
  comparison: true,
  comparisonPosition: 50,
  processing: false,
  stale: false,
  onopen: vi.fn(),
  oncomparisonchange: vi.fn()
};

describe('ImageStage comparison layout', () => {
  it('uses independent side-by-side geometry for rotated comparisons', () => {
    const view = render(ImageStage, {
      props: { ...baseProps, splitComparison: true }
    });

    expect(view.getByText(/^Before/)).toBeTruthy();
    expect(view.getByText(/^After/)).toBeTruthy();
    expect(view.queryByLabelText('Before and after divider')).toBeNull();
  });

  it('keeps the swipe divider when both images share geometry', () => {
    const view = render(ImageStage, {
      props: { ...baseProps, splitComparison: false }
    });

    expect(view.getByLabelText('Before and after divider')).toBeTruthy();
    expect(view.container.querySelector('.split-canvas')).toBeNull();
  });

  it('announces the processing indicator while retaining the current image', () => {
    const view = render(ImageStage, {
      props: { ...baseProps, splitComparison: false, comparison: false, processing: true }
    });
    expect(view.getByText('Forging preview')).toBeTruthy();
    expect(view.getByAltText('Edited preview of fixture.png')).toBeTruthy();
  });
});
