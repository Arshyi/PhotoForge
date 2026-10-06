import { tick } from 'svelte';
import { afterEach, describe, expect, it, vi } from 'vitest';
import { cleanup, fireEvent, render, screen } from '@testing-library/svelte';
import RegionSelector from './RegionSelector.svelte';
import { admissionFixture, layout, pointer } from '../source/testing';
import type { Rect } from '../source/types';

afterEach(cleanup);

// The fixture's source is 12,000 x 9,000; the stage is laid out at 600 wide, so
// one pixel on screen is twenty source pixels.
const SCALE = 20;

function setup(initial: Rect = { x: 2_000, y: 2_000, width: 4_000, height: 3_000 }) {
  const onconfirm = vi.fn();
  const oncancel = vi.fn();
  const onback = vi.fn();
  const view = render(RegionSelector, {
    admission: admissionFixture(),
    previewState: 'ready',
    preview: { dataUrl: 'data:image/png;base64,AAAA', width: 600, height: 450 },
    initial,
    onconfirm,
    oncancel,
    onback
  });
  const stage = screen.getByTestId('region-stage');
  layout(stage, 600, 450);
  const box = screen.getByTestId('region-box');
  layout(box, 200, 150);
  return { ...view, onconfirm, oncancel, onback, stage, box };
}

const field = (name: string) => screen.getByLabelText(name) as HTMLInputElement;

describe('region selector', () => {
  it('shows the original, the selection and the estimate', () => {
    setup();
    expect(screen.getByText(/12,000 × 9,000 px · 108\.0 MP/)).toBeTruthy();
    expect(screen.getByTestId('selected-summary').textContent).toContain('4,000 × 3,000 px · 12.0 MP');
    // 12 MP at 44 bytes per pixel plus the fixed part is a few hundred megabytes.
    expect(screen.getByTestId('estimate').textContent).toMatch(/MB|GB/);
    expect(screen.getByTestId('admission').textContent).toContain('Supported');
  });

  it('moves in source pixels, whatever size the preview is drawn at', async () => {
    const { box } = setup();
    pointer(box, 'pointerdown', 100, 100);
    pointer(box, 'pointermove', 130, 110);
    pointer(box, 'pointerup', 130, 110);
    await tick();
    // 30 screen pixels right and 10 down is 600 and 200 source pixels.
    expect(field('X').value).toBe('2600');
    expect(field('Y').value).toBe('2200');
    expect(field('Width').value).toBe('4000');
  });

  it('keeps the region inside the source when dragged past an edge', async () => {
    const { box } = setup();
    pointer(box, 'pointerdown', 100, 100);
    pointer(box, 'pointermove', 5_000, 5_000);
    pointer(box, 'pointerup', 5_000, 5_000);
    await tick();
    // It moved, and is pressed against the far edges rather than lost beyond them.
    expect(field('X').value).toBe(String(12_000 - 4_000));
    expect(field('Y').value).toBe(String(9_000 - 3_000));
    expect(Number(field('X').value) + Number(field('Width').value)).toBeLessThanOrEqual(12_000);
    expect(Number(field('Y').value) + Number(field('Height').value)).toBeLessThanOrEqual(9_000);
  });

  it('resizes from a handle with the opposite corner held still', async () => {
    setup();
    const handle = screen.getByRole('button', { name: 'Resize bottom-right corner' });
    pointer(handle, 'pointerdown', 300, 250);
    pointer(handle, 'pointermove', 310, 255);
    pointer(handle, 'pointerup', 310, 255);
    await tick();
    expect(field('X').value).toBe('2000');
    expect(field('Y').value).toBe('2000');
    expect(field('Width').value).toBe('4200');
    expect(field('Height').value).toBe('3100');
  });

  it('does not start a drag on a right click', async () => {
    const { box } = setup();
    pointer(box, 'pointerdown', 100, 100, 2);
    pointer(box, 'pointermove', 200, 200);
    pointer(box, 'pointerup', 200, 200);
    await tick();
    expect(field('X').value).toBe('2000');
  });

  it('locks the aspect ratio from the preset and while resizing', async () => {
    setup();
    await fireEvent.change(screen.getByLabelText('Aspect ratio'), { target: { value: '1:1' } });
    expect(field('Width').value).toBe(field('Height').value);
    const handle = screen.getByRole('button', { name: 'Resize bottom-right corner' });
    pointer(handle, 'pointerdown', 300, 250);
    pointer(handle, 'pointermove', 340, 252);
    pointer(handle, 'pointerup', 340, 252);
    await tick();
    expect(Math.abs(Number(field('Width').value) - Number(field('Height').value))).toBeLessThanOrEqual(1);
  });

  it('follows the other dimension when one is typed under a lock', async () => {
    setup({ x: 0, y: 0, width: 1_600, height: 1_200 });
    await fireEvent.change(screen.getByLabelText('Aspect ratio'), { target: { value: '4:3' } });
    await fireEvent.change(field('Width'), { target: { value: '2400' } });
    expect(field('Width').value).toBe('2400');
    expect(field('Height').value).toBe('1800');
  });

  it('clamps typed values and ignores text that is not a number', async () => {
    setup();
    await fireEvent.change(field('X'), { target: { value: '99999' } });
    expect(Number(field('X').value) + Number(field('Width').value)).toBeLessThanOrEqual(12_000);
    await fireEvent.change(field('Width'), { target: { value: 'wide' } });
    expect(field('Width').value).toBe('4000');
    await fireEvent.change(field('Height'), { target: { value: '-5' } });
    expect(Number(field('Height').value)).toBeGreaterThanOrEqual(1);
  });

  /** Svelte rewrites a field only when its expression changes, so an entry that
   *  clamps back to the old value used to leave the rejected text on screen. */
  it('shows the real value after an entry that clamps back to where it was', async () => {
    setup({ x: 0, y: 0, width: 1_000, height: 1_000 });
    await fireEvent.change(field('X'), { target: { value: '-40' } });
    expect(field('X').value).toBe('0');
    await fireEvent.change(field('Width'), { target: { value: '999999' } });
    expect(field('Width').value).toBe('12000');
    await fireEvent.change(field('Width'), { target: { value: '999999' } });
    expect(field('Width').value).toBe('12000');
  });

  /** The estimate and the verdict must follow the rectangle, not be fixed at open. */
  it('updates the estimate and flips to "Too large" past the budget, and refuses to open', async () => {
    const { onconfirm } = setup();
    const before = screen.getByTestId('estimate').textContent;
    await fireEvent.change(field('Width'), { target: { value: '6000' } });
    await fireEvent.change(field('Height'), { target: { value: '5000' } });
    // 30 MP is over the fixture's 20 MP ceiling.
    expect(screen.getByTestId('estimate').textContent).not.toBe(before);
    expect(screen.getByTestId('admission').textContent).toContain('Too large');
    expect(screen.getByTestId('admission').textContent).toMatch(/at most 20,000,000 pixels/);
    const open = screen.getByRole('button', { name: 'Open region' }) as HTMLButtonElement;
    expect(open.disabled).toBe(true);
    await fireEvent.click(open);
    expect(onconfirm).not.toHaveBeenCalled();
    expect(screen.getByTestId('region-frame').className).toContain('unsupported');
  });

  it('opens the region that is selected', async () => {
    const { onconfirm } = setup();
    await fireEvent.click(screen.getByRole('button', { name: 'Open region' }));
    expect(onconfirm).toHaveBeenCalledExactlyOnceWith({ x: 2_000, y: 2_000, width: 4_000, height: 3_000 });
  });

  it('centres and maximises', async () => {
    setup({ x: 0, y: 0, width: 1_000, height: 1_000 });
    await fireEvent.click(screen.getByRole('button', { name: 'Center' }));
    expect(field('X').value).toBe('5500');
    expect(field('Y').value).toBe('4000');
    await fireEvent.click(screen.getByRole('button', { name: 'Maximize' }));
    const pixels = Number(field('Width').value) * Number(field('Height').value);
    expect(pixels).toBeLessThanOrEqual(20_000_000);
    expect(pixels).toBeGreaterThan(19_000_000);
    expect(screen.getByTestId('admission').textContent).toContain('Supported');
  });

  describe('from the keyboard', () => {
    it('moves with the arrow keys by one, ten and a hundred pixels', async () => {
      const { box } = setup();
      await fireEvent.keyDown(box, { key: 'ArrowRight' });
      expect(field('X').value).toBe('2001');
      await fireEvent.keyDown(box, { key: 'ArrowRight', ctrlKey: true });
      expect(field('X').value).toBe('2011');
      await fireEvent.keyDown(box, { key: 'ArrowDown', altKey: true });
      expect(field('Y').value).toBe('2100');
    });

    it('resizes with Shift and the arrow keys', async () => {
      const { box } = setup();
      await fireEvent.keyDown(box, { key: 'ArrowRight', shiftKey: true, ctrlKey: true });
      expect(field('Width').value).toBe('4010');
      expect(field('X').value).toBe('2000');
    });

    it('resizes one edge from its own handle', async () => {
      setup();
      const handle = screen.getByRole('button', { name: 'Resize left edge' });
      await fireEvent.keyDown(handle, { key: 'ArrowLeft', ctrlKey: true });
      expect(field('X').value).toBe('1990');
      expect(field('Width').value).toBe('4010');
    });

    it('ignores keys that are not arrows', async () => {
      const { box } = setup();
      await fireEvent.keyDown(box, { key: 'a' });
      expect(field('X').value).toBe('2000');
    });
  });

  describe('for assistive technology', () => {
    it('labels the region and every handle', () => {
      setup();
      expect(screen.getByRole('button', { name: /Working region/ })).toBeTruthy();
      for (const name of [
        'top-left corner',
        'top edge',
        'top-right corner',
        'right edge',
        'bottom-right corner',
        'bottom edge',
        'bottom-left corner',
        'left edge'
      ]) {
        expect(screen.getByRole('button', { name: `Resize ${name}` })).toBeTruthy();
      }
    });

    it('labels every coordinate field and the aspect control', () => {
      setup();
      for (const name of ['X', 'Y', 'Width', 'Height', 'Aspect ratio']) {
        expect(screen.getByLabelText(name)).toBeTruthy();
      }
    });

    it('announces a change when it settles, with the memory and the verdict', async () => {
      const { box } = setup();
      const live = screen.getByTestId('announcement');
      expect(live.getAttribute('aria-live')).toBe('polite');
      expect(live.textContent).toBe('');
      // Mid-drag nothing is announced...
      pointer(box, 'pointerdown', 100, 100);
      pointer(box, 'pointermove', 130, 110);
      expect(live.textContent).toBe('');
      // ...and on release it is.
      pointer(box, 'pointerup', 130, 110);
      await tick();
      expect(live.textContent).toMatch(/Region 4000 by 3000 pixels at 2600, 2200/);
      expect(live.textContent).toMatch(/12\.0 megapixels/);
      expect(live.textContent).toMatch(/can be opened/);
    });

    it('announces why a region is refused', async () => {
      setup();
      await fireEvent.change(field('Width'), { target: { value: '9000' } });
      expect(screen.getByTestId('announcement').textContent).toMatch(/Too large/);
    });
  });

  it('cancels and goes back', async () => {
    const { oncancel, onback } = setup();
    await fireEvent.click(screen.getByRole('button', { name: 'Cancel' }));
    await fireEvent.click(screen.getByRole('button', { name: 'Back' }));
    expect(oncancel).toHaveBeenCalledOnce();
    expect(onback).toHaveBeenCalledOnce();
  });

  it('still works when no preview could be made', async () => {
    const onconfirm = vi.fn();
    render(RegionSelector, {
      admission: admissionFixture(),
      previewState: 'unavailable',
      previewMessage: 'Out of memory.',
      initial: { x: 0, y: 0, width: 2_000, height: 1_000 },
      onconfirm
    });
    // Two live regions exist, the stage's note and the announcer; find the note.
    expect(screen.getByText(/Enter the region numerically/)).toBeTruthy();
    await fireEvent.click(screen.getByRole('button', { name: 'Open region' }));
    expect(onconfirm).toHaveBeenCalledOnce();
  });

  it('says so when this file cannot be opened a region at a time', () => {
    render(RegionSelector, {
      admission: admissionFixture({ options: [], report: { regionCost: null } }),
      previewState: 'unavailable'
    });
    expect(screen.getByTestId('admission').textContent).toContain('Too large');
    expect((screen.getByRole('button', { name: 'Open region' }) as HTMLButtonElement).disabled).toBe(true);
    expect(screen.getByTestId('estimate').textContent).toBe('Unknown');
  });
});
