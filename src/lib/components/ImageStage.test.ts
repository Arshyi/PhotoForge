import { fireEvent, render } from '@testing-library/svelte';
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

  it('maps scaled pointer input into canonical image-space coordinates', async () => {
    const onselectiongesture = vi.fn();
    const view = render(ImageStage, {
      props: {
        ...baseProps,
        comparison: false,
        imageWidth: 4000,
        imageHeight: 2000,
        selectionTool: 'rectangle',
        onselectiongesture
      }
    });
    const layer = view.getByRole('button', { name: 'rectangle selection canvas' });
    Object.defineProperty(layer, 'getBoundingClientRect', {
      value: () => ({ left: 0, top: 0, width: 200, height: 100, right: 200, bottom: 100, x: 0, y: 0, toJSON: () => ({}) })
    });
    Object.defineProperty(layer, 'setPointerCapture', { value: vi.fn() });
    Object.defineProperty(layer, 'releasePointerCapture', { value: vi.fn() });
    const down = new MouseEvent('pointerdown', { bubbles: true, clientX: 50, clientY: 25 });
    const up = new MouseEvent('pointerup', { bubbles: true, clientX: 150, clientY: 75 });
    Object.defineProperty(down, 'pointerId', { value: 1 });
    Object.defineProperty(up, 'pointerId', { value: 1 });
    await fireEvent(layer, down);
    await fireEvent(layer, up);
    expect(onselectiongesture).toHaveBeenCalledWith({
      tool: 'rectangle',
      points: [{ x: 1000, y: 500 }, { x: 3000, y: 1500 }],
      shiftKey: false,
      altKey: false
    });
  });
});
