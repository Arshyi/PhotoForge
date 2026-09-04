import { fireEvent, render, screen, within } from '@testing-library/svelte';
import { describe, expect, it, vi } from 'vitest';
import TransformPanel from './TransformPanel.svelte';
import { MAX_LAYER_SCALE, MIN_LAYER_SCALE } from '../layers/transformTool';
import { identityTransform, type LayerTransform } from '../layers/types';

const LAYER_W = 100;
const LAYER_H = 60;

function mount(transform: Partial<LayerTransform> = {}, extra: Record<string, unknown> = {}) {
  const handlers = {
    onchange: vi.fn(),
    onaspectchange: vi.fn(),
    ontoggle: vi.fn(),
    onreset: vi.fn(),
    onrasterize: vi.fn(),
    onflip: vi.fn()
  };
  const result = render(TransformPanel, {
    props: {
      transform: { ...identityTransform, ...transform },
      layerWidth: LAYER_W,
      layerHeight: LAYER_H,
      canvasWidth: 200,
      canvasHeight: 200,
      layerName: 'Sky',
      aspectLocked: false,
      ...handlers,
      ...extra
    }
  });
  return { ...result, handlers };
}

function fieldNamed(name: string): HTMLInputElement {
  return screen.getByLabelText(name) as HTMLInputElement;
}

async function type(name: string, value: string) {
  const input = fieldNamed(name);
  await fireEvent.change(input, { target: { value } });
}

describe('TransformPanel readout', () => {
  it('shows the layer centre and its scaled size', () => {
    mount({ translateX: 30, scaleX: 2 });
    expect(fieldNamed('Center X').value).toBe('80');
    expect(fieldNamed('Center Y').value).toBe('30');
    expect(fieldNamed('Width').value).toBe('200');
    expect(fieldNamed('Height').value).toBe('60');
  });

  it('reports the transformed bounds in document coordinates', () => {
    mount({ translateX: 10, translateY: 5 });
    expect(screen.getByText(/Bounds 10, 5 to 110, 65/)).toBeTruthy();
  });

  it('warns when the layer has been moved entirely off the canvas', () => {
    mount({ translateX: -500 });
    expect(screen.getByText(/Entirely off canvas/)).toBeTruthy();
  });

  it('says whether the layer is transformed at all', () => {
    const plain = mount();
    expect(plain.container.textContent).toContain('sits on its own grid');
    plain.unmount();
    const moved = mount({ rotationDegrees: 4 });
    expect(moved.container.textContent).toContain('is transformed');
  });

  it('asks for a pixel layer when there is nothing to transform', () => {
    mount({}, { layerWidth: 0, layerHeight: 0 });
    expect(screen.getByText(/Select a pixel layer/)).toBeTruthy();
    expect(screen.queryByLabelText('Center X')).toBeNull();
  });
});

describe('TransformPanel editing', () => {
  it('moves the layer when a centre is typed in', async () => {
    const { handlers } = mount();
    await type('Center X', '150');
    const [next, label] = handlers.onchange.mock.calls[0];
    expect(next.translateX).toBe(100);
    expect(label).toBe('Move layer');
  });

  it('rescales when a width is typed in', async () => {
    const { handlers } = mount();
    await type('Width', '250');
    const [next, label] = handlers.onchange.mock.calls[0];
    expect(next.scaleX).toBeCloseTo(2.5, 9);
    expect(next.scaleY).toBe(1);
    expect(label).toBe('Scale layer');
  });

  it('carries proportions to the other axis when the lock is on', async () => {
    const { handlers } = mount({}, { aspectLocked: true });
    await type('Width', '200');
    const [next] = handlers.onchange.mock.calls[0];
    expect(next.scaleX).toBeCloseTo(2, 9);
    expect(next.scaleY).toBeCloseTo(2, 9);
  });

  it('rotates when an angle is typed in', async () => {
    const { handlers } = mount();
    await type('Rotation', '35');
    expect(handlers.onchange.mock.calls[0][0].rotationDegrees).toBe(35);
    expect(handlers.onchange.mock.calls[0][1]).toBe('Rotate layer');
  });

  it('repairs a value the renderer could not use rather than committing it', async () => {
    const { handlers } = mount();
    await type('Width', '999999999');
    expect(Math.abs(handlers.onchange.mock.calls[0][0].scaleX)).toBeLessThanOrEqual(
      MAX_LAYER_SCALE
    );
    await type('Height', '0');
    expect(Math.abs(handlers.onchange.mock.calls[1][0].scaleY)).toBeGreaterThanOrEqual(
      MIN_LAYER_SCALE
    );
    await type('Rotation', '5000');
    expect(Math.abs(handlers.onchange.mock.calls[2][0].rotationDegrees)).toBeLessThanOrEqual(360);
  });

  it('ignores an emptied or unparseable field instead of committing a NaN', async () => {
    const { handlers } = mount();
    await type('Center X', '');
    await type('Center Y', 'abc');
    expect(handlers.onchange).not.toHaveBeenCalled();
  });

  it('reports a change to the proportion lock', async () => {
    const { handlers } = mount();
    await fireEvent.click(screen.getByLabelText('Keep proportions'));
    expect(handlers.onaspectchange).toHaveBeenCalledWith(true);
  });
});

describe('TransformPanel actions', () => {
  it('toggles the on-canvas box and says whether it is showing', async () => {
    const { handlers, rerender } = mount();
    const toggle = screen.getByRole('button', { name: 'Transform' });
    expect(toggle.getAttribute('aria-pressed')).toBe('false');
    await fireEvent.click(toggle);
    expect(handlers.ontoggle).toHaveBeenCalledTimes(1);
    await rerender({ active: true });
    expect(screen.getByRole('button', { name: 'Transforming' }).getAttribute('aria-pressed')).toBe(
      'true'
    );
  });

  it('raises each flip separately', async () => {
    const { handlers } = mount();
    await fireEvent.click(screen.getByRole('button', { name: 'Flip H' }));
    await fireEvent.click(screen.getByRole('button', { name: 'Flip V' }));
    expect(handlers.onflip.mock.calls.map((call) => call[0])).toEqual(['horizontal', 'vertical']);
  });

  it('offers Reset only once there is something to reset', async () => {
    const plain = mount();
    expect((screen.getByRole('button', { name: 'Reset' }) as HTMLButtonElement).disabled).toBe(
      true
    );
    plain.unmount();
    const { handlers } = mount({ translateX: 3 });
    await fireEvent.click(screen.getByRole('button', { name: 'Reset' }));
    expect(handlers.onreset).toHaveBeenCalledTimes(1);
  });

  it('offers Rasterize, and withholds it while the backend is busy', async () => {
    const { handlers } = mount();
    await fireEvent.click(screen.getByRole('button', { name: 'Rasterize' }));
    expect(handlers.onrasterize).toHaveBeenCalledTimes(1);
    const busy = mount({}, { busy: true });
    expect(
      (
        within(busy.container).getByRole('button', { name: 'Rasterize' }) as HTMLButtonElement
      ).disabled
    ).toBe(true);
  });

  it('switches the sampling mode and marks the current one', async () => {
    const { handlers, rerender } = mount();
    const smooth = screen.getByRole('button', { name: 'Smooth' });
    expect(smooth.getAttribute('aria-pressed')).toBe('true');
    await fireEvent.click(screen.getByRole('button', { name: 'Hard edge' }));
    expect(handlers.onchange.mock.calls[0][0].interpolation).toBe('nearest');
    await rerender({ transform: { ...identityTransform, interpolation: 'nearest' } });
    expect(screen.getByRole('button', { name: 'Hard edge' }).getAttribute('aria-pressed')).toBe(
      'true'
    );
  });

  it('disables every control when the panel is disabled', () => {
    const { container } = mount({}, { disabled: true });
    const controls = [...container.querySelectorAll('button, input')];
    expect(controls.length).toBeGreaterThan(8);
    expect(controls.every((control) => (control as HTMLInputElement).disabled)).toBe(true);
  });
});
