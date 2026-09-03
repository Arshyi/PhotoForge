import { fireEvent, render, screen } from '@testing-library/svelte';
import { describe, expect, it, vi } from 'vitest';
import CurveEditor from './CurveEditor.svelte';
import { identityCurves } from '../layers/adjustments';
import type { CurveSet } from '../types/editor';

function props(curves: CurveSet = identityCurves()) {
  return { curves, onchange: vi.fn(), disabled: false };
}

/** Parses the rendered viewBox into its four numbers. */
function viewBox(container: HTMLElement): number[] {
  const svg = container.querySelector('svg') as SVGSVGElement;
  return (svg.getAttribute('viewBox') ?? '').split(/\s+/).map(Number);
}

describe('CurveEditor', () => {
  it('renders the identity curve with both endpoints', () => {
    const { container } = render(CurveEditor, { props: props() });
    const points = container.querySelectorAll('.curve-point');
    expect(points).toHaveLength(2);
    expect(points[0].getAttribute('cx')).toBe('0');
    expect(points[1].getAttribute('cx')).toBe('100');
  });

  /**
   * Found in real-browser testing: with an unpadded `0 0 100 100` viewBox the
   * endpoint circles sat half outside the SVG and `overflow: hidden` clipped
   * them, so `elementFromPoint` at their centres returned the container and the
   * endpoints could never be grabbed. jsdom has no layout and cannot reproduce
   * the clipping, so this asserts the padding that prevents it.
   */
  it('pads the viewBox so points at the grid edges are not clipped', () => {
    const { container } = render(CurveEditor, { props: props() });
    const [minX, minY, width, height] = viewBox(container);
    expect(minX).toBeLessThan(0);
    expect(minY).toBeLessThan(0);
    // The drawing area must extend past the 0..100 grid on both sides.
    expect(minX + width).toBeGreaterThan(100);
    expect(minY + height).toBeGreaterThan(100);

    // The margin has to clear the point radius, or a corner point is still cut.
    const radius = Number(container.querySelector('.curve-point')?.getAttribute('r'));
    expect(Math.abs(minX)).toBeGreaterThanOrEqual(radius);
    expect(minX + width - 100).toBeGreaterThanOrEqual(radius);
  });

  it('keeps the grid itself at the full 0 to 100 range', () => {
    const { container } = render(CurveEditor, { props: props() });
    const grid = container.querySelector('.grid-background') as SVGRectElement;
    expect(grid.getAttribute('x')).toBe('0');
    expect(grid.getAttribute('y')).toBe('0');
    expect(grid.getAttribute('width')).toBe('100');
    expect(grid.getAttribute('height')).toBe('100');
  });

  it('offers a channel tab per curve channel and marks edited ones', async () => {
    const curves = identityCurves();
    curves.red = [
      { input: 0, output: 0 },
      { input: 0.5, output: 0.8 },
      { input: 1, output: 1 }
    ];
    const { container } = render(CurveEditor, { props: props(curves) });
    const tabs = container.querySelectorAll('.channel-tabs button');
    expect([...tabs].map((tab) => tab.textContent?.trim().split(/\s+/)[0])).toEqual([
      'RGB',
      'Red',
      'Green',
      'Blue'
    ]);
    // Only the edited channel carries the marker.
    expect(container.querySelectorAll('.channel-tabs i')).toHaveLength(1);
  });

  it('adds a point from the keyboard without a pointer', async () => {
    const value = props();
    const { container } = render(CurveEditor, { props: value });
    const grid = container.querySelector('.grid-background') as SVGRectElement;
    await fireEvent.keyDown(grid, { key: 'Enter' });
    expect(value.onchange).toHaveBeenCalled();
    const next = value.onchange.mock.calls[0][0] as CurveSet;
    expect(next.rgb).toHaveLength(3);
    // The new point splits the widest gap, which for an identity curve is 0.5.
    expect(next.rgb[1].input).toBeCloseTo(0.5, 2);
  });

  it('nudges a point with the arrow keys', async () => {
    const value = props();
    const { container } = render(CurveEditor, { props: value });
    const first = container.querySelector('.curve-point') as SVGCircleElement;
    await fireEvent.keyDown(first, { key: 'ArrowUp' });
    const next = value.onchange.mock.calls[0][0] as CurveSet;
    expect(next.rgb[0].output).toBeCloseTo(0.01, 5);
    // The endpoint's input stays anchored even when nudged.
    expect(next.rgb[0].input).toBe(0);
  });

  it('resets the active channel', async () => {
    const curves = identityCurves();
    curves.rgb = [
      { input: 0, output: 0.2 },
      { input: 1, output: 0.9 }
    ];
    const value = props(curves);
    render(CurveEditor, { props: value });
    await fireEvent.click(screen.getByRole('button', { name: /Reset RGB/ }));
    const next = value.onchange.mock.calls[0][0] as CurveSet;
    expect(next.rgb).toEqual([
      { input: 0, output: 0 },
      { input: 1, output: 1 }
    ]);
  });

  it('exposes each point as a slider for assistive technology', () => {
    const { container } = render(CurveEditor, { props: props() });
    const points = container.querySelectorAll('.curve-point[role="slider"]');
    expect(points).toHaveLength(2);
    expect(points[0].getAttribute('aria-valuetext')).toMatch(/input 0 percent/);
  });

  it('disables every control when disabled', () => {
    const { container } = render(CurveEditor, { props: { ...props(), disabled: true } });
    const focusable = [...container.querySelectorAll('[tabindex]')];
    expect(focusable.every((el) => el.getAttribute('tabindex') === '-1')).toBe(true);
    expect(
      [...container.querySelectorAll('button')].every((b) => (b as HTMLButtonElement).disabled)
    ).toBe(true);
  });
});
