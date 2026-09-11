import { afterEach, describe, expect, it, vi } from 'vitest';
import { cleanup, fireEvent, render } from '@testing-library/svelte';
import SemanticCanvas from './SemanticCanvas.svelte';
import { createTextLayer } from '../layers/tree';

afterEach(cleanup);
function setup(tool: 'text' | 'rectangle' | 'ellipse' | 'line' | 'polygon' | 'star' = 'rectangle') {
  const props = { tool, canvasWidth: 1000, canvasHeight: 500, ontext: vi.fn(), onshape: vi.fn(), oncommittext: vi.fn(), oncanceltext: vi.fn() };
  const view = render(SemanticCanvas, props);
  const surface = view.getByRole('button', { name: `${tool} layer canvas` });
  surface.getBoundingClientRect = () => ({ left: 20, top: 40, width: 500, height: 250 }) as DOMRect;
  surface.setPointerCapture = vi.fn(); surface.hasPointerCapture = () => true; surface.releasePointerCapture = vi.fn();
  return { ...view, ...props, surface };
}
function pointer(element: HTMLElement, type: string, x: number, y: number, button = 0) {
  const event = new MouseEvent(type, { bubbles: true, clientX: x, clientY: y, button });
  Object.defineProperty(event, 'pointerId', { value: 1 });
  element.dispatchEvent(event);
}
describe('semantic canvas', () => {
  it('records one parametric rectangle in document coordinates at any preview scale', async () => {
    const v = setup();
    pointer(v.surface, 'pointerdown', 120, 90);
    pointer(v.surface, 'pointermove', 200, 150);
    expect(v.onshape).not.toHaveBeenCalled();
    pointer(v.surface, 'pointerup', 220, 190);
    expect(v.onshape).toHaveBeenCalledExactlyOnceWith({ type: 'rectangle', x: 200, y: 100, width: 200, height: 200, cornerRadius: 0 });
  });
  it('preserves line direction and allows a horizontal line', () => {
    const v = setup('line');
    pointer(v.surface, 'pointerdown', 220, 90); pointer(v.surface, 'pointerup', 120, 90);
    expect(v.onshape).toHaveBeenCalledWith({ type: 'line', x1: 400, y1: 100, x2: 200, y2: 100 });
  });
  it('does not create a shape for right click, zero area or cancellation', () => {
    const v = setup();
    pointer(v.surface, 'pointerdown', 120, 90, 2); pointer(v.surface, 'pointerup', 220, 190);
    pointer(v.surface, 'pointerdown', 120, 90); pointer(v.surface, 'pointerup', 120, 90);
    pointer(v.surface, 'pointerdown', 120, 90); pointer(v.surface, 'pointercancel', 200, 160); pointer(v.surface, 'pointerup', 220, 190);
    expect(v.onshape).not.toHaveBeenCalled();
  });
  it('cancels an unfinished gesture when the active tool changes', async () => {
    const v = setup();
    pointer(v.surface, 'pointerdown', 120, 90);
    await v.rerender({ tool: 'ellipse' });
    pointer(v.surface, 'pointerup', 220, 190);
    expect(v.onshape).not.toHaveBeenCalled();
  });
  it('places text without manufacturing a bitmap', () => {
    const v = setup('text'); pointer(v.surface, 'pointerdown', 120, 90);
    expect(v.ontext).toHaveBeenCalledExactlyOnceWith({ x: 200, y: 100 });
    expect(v.onshape).not.toHaveBeenCalled();
  });
  it('keeps typing local until explicit commit and lets Escape cancel', async () => {
    const v = setup('text'); const layer = createTextLayer('t', 'سلام', 0, 0);
    if (layer.content.type !== 'text') throw new Error('text expected');
    await v.rerender({ edit: { id: layer.id, content: layer.content } });
    const input = v.getByLabelText('Canvas text content');
    await fireEvent.input(input, { target: { value: 'سلام\nHello é' } });
    await fireEvent.keyDown(input, { key: 'Enter' });
    expect(v.oncommittext).not.toHaveBeenCalled();
    await fireEvent.keyDown(input, { key: 'Enter', ctrlKey: true, isComposing: true });
    expect(v.oncommittext).not.toHaveBeenCalled();
    await fireEvent.keyDown(input, { key: 'Enter', ctrlKey: true });
    expect(v.oncommittext).toHaveBeenCalledExactlyOnceWith('سلام\nHello é');
    await fireEvent.keyDown(input, { key: 'Escape' });
    expect(v.oncanceltext).toHaveBeenCalledOnce();
    expect(layer.content.text).toBe('سلام');
  });
});
