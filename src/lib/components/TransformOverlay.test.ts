import { render } from '@testing-library/svelte';
import { beforeEach, describe, expect, it, vi } from 'vitest';
import TransformOverlay from './TransformOverlay.svelte';
import { identityTransform, type LayerTransform } from '../layers/types';

const CANVAS = 200;
const LAYER_W = 100;
const LAYER_H = 100;

/**
 * jsdom gives every element a zero-sized box, so the component's own
 * `getBoundingClientRect` would divide by zero and every pointer position would
 * collapse onto the origin. Pinning a box lets the real coordinate mapping run.
 */
function stubLayout(width = CANVAS, height = CANVAS) {
  Element.prototype.getBoundingClientRect = function bounds(this: Element) {
    if ((this as HTMLElement).classList?.contains('transform-surface')) {
      return {
        left: 0,
        top: 0,
        width,
        height,
        right: width,
        bottom: height,
        x: 0,
        y: 0,
        toJSON: () => ({})
      } as DOMRect;
    }
    return { left: 0, top: 0, width: 0, height: 0, right: 0, bottom: 0, x: 0, y: 0, toJSON: () => ({}) } as DOMRect;
  };
}

interface Harness {
  preview: ReturnType<typeof vi.fn>;
  commit: ReturnType<typeof vi.fn>;
  cancel: ReturnType<typeof vi.fn>;
}

function mount(transform: Partial<LayerTransform> = {}, extra: Record<string, unknown> = {}) {
  const handlers: Harness = { preview: vi.fn(), commit: vi.fn(), cancel: vi.fn() };
  const result = render(TransformOverlay, {
    props: {
      transform: { ...identityTransform, ...transform },
      layerWidth: LAYER_W,
      layerHeight: LAYER_H,
      canvasWidth: CANVAS,
      canvasHeight: CANVAS,
      layerName: 'Sky',
      onpreview: handlers.preview,
      oncommit: handlers.commit,
      oncancel: handlers.cancel,
      ...extra
    }
  });
  const surface = result.container.querySelector('.transform-surface') as HTMLButtonElement;
  // jsdom has no pointer capture; the component only calls it, never reads it
  // back for anything but the release.
  surface.setPointerCapture = vi.fn();
  surface.releasePointerCapture = vi.fn();
  surface.hasPointerCapture = vi.fn(() => true);
  return { ...result, surface, handlers };
}

/** Builds a PointerEvent jsdom will carry clientX and clientY through. */
function pointer(type: string, x: number, y: number, init: PointerEventInit = {}) {
  const event = new MouseEvent(type, { bubbles: true, cancelable: true, button: 0, ...init });
  Object.defineProperty(event, 'clientX', { value: x });
  Object.defineProperty(event, 'clientY', { value: y });
  Object.defineProperty(event, 'pointerId', { value: 1 });
  return event;
}

/** A document-space point expressed in the surface's client coordinates. */
function client(documentX: number, documentY: number) {
  return [(documentX / CANVAS) * CANVAS, (documentY / CANVAS) * CANVAS] as const;
}

beforeEach(() => {
  stubLayout();
});

describe('TransformOverlay rendering', () => {
  it('draws the box, eight scale handles, a rotation grip, and a pivot', () => {
    const { container } = mount();
    expect(container.querySelector('polygon.bounds')).not.toBeNull();
    expect(container.querySelectorAll('rect.handle')).toHaveLength(8);
    expect(container.querySelector('circle.grip')).not.toBeNull();
    expect(container.querySelector('circle.pivot')).not.toBeNull();
  });

  it('names every handle so a failure points at the right one', () => {
    const { container } = mount();
    const names = [...container.querySelectorAll('rect.handle')].map((node) =>
      node.getAttribute('data-handle')
    );
    expect(names.sort()).toEqual(['e', 'n', 'ne', 'nw', 's', 'se', 'sw', 'w']);
  });

  it('places the box on the layer, not on the canvas', () => {
    const { container } = mount();
    const points = container.querySelector('polygon.bounds')?.getAttribute('points');
    expect(points).toBe('0,0 100,0 100,100 0,100');
  });

  it('follows a translation', () => {
    const { container } = mount({ translateX: 20, translateY: -5 });
    expect(container.querySelector('polygon.bounds')?.getAttribute('points')).toBe(
      '20,-5 120,-5 120,95 20,95'
    );
  });

  it('describes itself and its gestures for assistive technology', () => {
    const { surface } = mount();
    const label = surface.getAttribute('aria-label') ?? '';
    expect(label).toContain('Sky');
    expect(label).toMatch(/Arrow keys nudge/);
    expect(label).toMatch(/Escape cancels/);
  });

  it('cannot be focused or driven when disabled', () => {
    const { surface, handlers } = mount({}, { disabled: true });
    expect(surface.disabled).toBe(true);
    surface.dispatchEvent(pointer('pointerdown', ...client(50, 50)));
    expect(handlers.preview).not.toHaveBeenCalled();
  });
});

describe('pointer gestures', () => {
  it('previews on every move and commits exactly once on release', () => {
    const { surface, handlers } = mount();
    surface.dispatchEvent(pointer('pointerdown', ...client(50, 50)));
    for (let step = 1; step <= 40; step += 1) {
      surface.dispatchEvent(pointer('pointermove', ...client(50 + step, 50)));
    }
    surface.dispatchEvent(pointer('pointerup', ...client(90, 50)));

    expect(handlers.preview).toHaveBeenCalledTimes(40);
    // Forty pointer moves, one undo step.
    expect(handlers.commit).toHaveBeenCalledTimes(1);
    expect(handlers.commit.mock.calls[0][0].translateX).toBeCloseTo(40, 6);
  });

  it('scales from a corner handle', () => {
    const { surface, handlers } = mount();
    surface.dispatchEvent(pointer('pointerdown', ...client(100, 100)));
    surface.dispatchEvent(pointer('pointerup', ...client(200, 200)));
    const committed = handlers.commit.mock.calls[0][0] as LayerTransform;
    expect(committed.scaleX).toBeCloseTo(2, 6);
    expect(committed.scaleY).toBeCloseTo(2, 6);
  });

  it('rotates from the grip', () => {
    const { container, surface, handlers } = mount();
    const grip = container.querySelector('circle.grip') as SVGCircleElement;
    const gripX = Number(grip.getAttribute('cx'));
    const gripY = Number(grip.getAttribute('cy'));
    surface.dispatchEvent(pointer('pointerdown', ...client(gripX, gripY)));
    // Swing the pointer round to the right of the centre.
    surface.dispatchEvent(pointer('pointerup', ...client(150, 50)));
    const committed = handlers.commit.mock.calls[0][0] as LayerTransform;
    expect(committed.rotationDegrees).toBeCloseTo(90, 4);
  });

  it('ignores a press that lands outside the box', () => {
    const { surface, handlers } = mount();
    surface.dispatchEvent(pointer('pointerdown', ...client(190, 190)));
    surface.dispatchEvent(pointer('pointermove', ...client(150, 150)));
    expect(handlers.preview).not.toHaveBeenCalled();
    expect(handlers.commit).not.toHaveBeenCalled();
  });

  /**
   * The pointer is captured, so a release well outside the box still lands on
   * the surface and must finish the gesture rather than stranding it.
   */
  it('finishes a drag released outside the box', () => {
    const { surface, handlers } = mount();
    surface.dispatchEvent(pointer('pointerdown', ...client(50, 50)));
    surface.dispatchEvent(pointer('pointerup', ...client(-400, 900)));
    expect(handlers.commit).toHaveBeenCalledTimes(1);
    const committed = handlers.commit.mock.calls[0][0] as LayerTransform;
    expect(Number.isFinite(committed.translateX)).toBe(true);
    expect(Number.isFinite(committed.translateY)).toBe(true);
  });

  it('abandons the gesture when the system cancels the pointer', () => {
    const { surface, handlers } = mount();
    surface.dispatchEvent(pointer('pointerdown', ...client(50, 50)));
    surface.dispatchEvent(pointer('pointermove', ...client(70, 50)));
    surface.dispatchEvent(pointer('pointercancel', 0, 0));
    expect(handlers.cancel).toHaveBeenCalledTimes(1);
    expect(handlers.commit).not.toHaveBeenCalled();
  });

  it('does not start a gesture from a secondary mouse button', () => {
    const { surface, handlers } = mount();
    surface.dispatchEvent(pointer('pointerdown', ...client(50, 50), { button: 2 }));
    surface.dispatchEvent(pointer('pointermove', ...client(90, 50)));
    expect(handlers.preview).not.toHaveBeenCalled();
  });

  it('holds proportions while Shift is held on a free drag', () => {
    const { surface, handlers } = mount();
    surface.dispatchEvent(pointer('pointerdown', ...client(100, 100)));
    surface.dispatchEvent(pointer('pointerup', ...client(300, 110), { shiftKey: true }));
    const committed = handlers.commit.mock.calls[0][0] as LayerTransform;
    expect(committed.scaleX).toBeCloseTo(committed.scaleY, 6);
  });
});

describe('keyboard', () => {
  function key(surface: HTMLElement, init: KeyboardEventInit) {
    surface.dispatchEvent(new KeyboardEvent('keydown', { bubbles: true, cancelable: true, ...init }));
  }

  it('nudges one pixel per arrow key', () => {
    const { surface, handlers } = mount();
    key(surface, { key: 'ArrowRight' });
    expect(handlers.commit.mock.calls[0][0].translateX).toBe(1);
    key(surface, { key: 'ArrowUp' });
    expect(handlers.commit.mock.calls[1][0].translateY).toBe(-1);
  });

  it('nudges ten pixels with Shift', () => {
    const { surface, handlers } = mount();
    key(surface, { key: 'ArrowDown', shiftKey: true });
    expect(handlers.commit.mock.calls[0][0].translateY).toBe(10);
  });

  it('cancels an in-flight drag with Escape without committing it', () => {
    const { surface, handlers } = mount();
    surface.dispatchEvent(pointer('pointerdown', ...client(50, 50)));
    surface.dispatchEvent(pointer('pointermove', ...client(90, 50)));
    key(surface, { key: 'Escape' });
    expect(handlers.cancel).toHaveBeenCalledTimes(1);
    expect(handlers.commit).not.toHaveBeenCalled();
  });

  it('finishes the interaction with Enter, keeping the transform', () => {
    const { surface, handlers } = mount({ translateX: 12 });
    key(surface, { key: 'Enter' });
    expect(handlers.commit).toHaveBeenCalledTimes(1);
    expect(handlers.commit.mock.calls[0][0].translateX).toBe(12);
  });

  it('ignores keys it has no meaning for', () => {
    const { surface, handlers } = mount();
    key(surface, { key: 'x' });
    expect(handlers.commit).not.toHaveBeenCalled();
    expect(handlers.cancel).not.toHaveBeenCalled();
  });
});
