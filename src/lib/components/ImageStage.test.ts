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

  it('emits deterministic resolved brush values for finite pen pressure', async () => {
    const onselectiongesture = vi.fn();
    const view = render(ImageStage, {
      props: {
        ...baseProps,
        comparison: false,
        imageWidth: 100,
        imageHeight: 100,
        selectionTool: 'brush',
        brushDiameter: 100,
        brushOpacity: 0.8,
        pressureEnabled: true,
        pressureAffectsSize: true,
        pressureAffectsOpacity: true,
        pressureMinSizeFactor: 0.25,
        pressureMinOpacityFactor: 0.5,
        onselectiongesture
      }
    });
    const layer = view.getByRole('button', { name: 'brush selection canvas' });
    Object.defineProperty(layer, 'getBoundingClientRect', {
      value: () => ({ left: 0, top: 0, width: 100, height: 100, right: 100, bottom: 100, x: 0, y: 0, toJSON: () => ({}) })
    });
    Object.defineProperty(layer, 'setPointerCapture', { value: vi.fn() });
    Object.defineProperty(layer, 'releasePointerCapture', { value: vi.fn() });
    const pointer = (type: string, x: number, pressure: number) => {
      const event = new MouseEvent(type, { bubbles: true, clientX: x, clientY: 20 });
      Object.defineProperties(event, {
        pointerId: { value: 7 },
        pointerType: { value: 'pen' },
        pressure: { value: pressure }
      });
      return event;
    };
    await fireEvent(layer, pointer('pointerdown', 10, 0.5));
    await fireEvent(layer, pointer('pointerup', 30, 1));
    expect(onselectiongesture).toHaveBeenCalledWith({
      tool: 'brush',
      points: [{ x: 10, y: 20 }, { x: 30, y: 20 }],
      resolvedBrushSamples: [
        { x: 10, y: 20, diameter: 62.5, opacity: 0.6 },
        { x: 30, y: 20, diameter: 100, opacity: 0.8 }
      ],
      shiftKey: false,
      altKey: false
    });
  });

  it('keeps mouse brush values uniform even when pressure controls are enabled', async () => {
    const onselectiongesture = vi.fn();
    const view = render(ImageStage, {
      props: {
        ...baseProps,
        comparison: false,
        imageWidth: 100,
        imageHeight: 100,
        selectionTool: 'brush',
        brushDiameter: 40,
        brushOpacity: 0.7,
        pressureEnabled: true,
        pressureAffectsSize: true,
        pressureAffectsOpacity: true,
        onselectiongesture
      }
    });
    const layer = view.getByRole('button', { name: 'brush selection canvas' });
    Object.defineProperty(layer, 'getBoundingClientRect', {
      value: () => ({ left: 0, top: 0, width: 100, height: 100, right: 100, bottom: 100, x: 0, y: 0, toJSON: () => ({}) })
    });
    Object.defineProperty(layer, 'setPointerCapture', { value: vi.fn() });
    Object.defineProperty(layer, 'releasePointerCapture', { value: vi.fn() });
    const pointer = (type: string, x: number) => {
      const event = new MouseEvent(type, { bubbles: true, clientX: x, clientY: 10 });
      Object.defineProperties(event, {
        pointerId: { value: 8 },
        pointerType: { value: 'mouse' },
        pressure: { value: 0.5 }
      });
      return event;
    };
    await fireEvent(layer, pointer('pointerdown', 10));
    await fireEvent(layer, pointer('pointerup', 30));
    const samples = onselectiongesture.mock.calls[0][0].resolvedBrushSamples;
    expect(samples).toEqual([
      { x: 10, y: 10, diameter: 40, opacity: 0.7 },
      { x: 30, y: 10, diameter: 40, opacity: 0.7 }
    ]);
  });
});
