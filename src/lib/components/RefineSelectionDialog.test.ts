import { fireEvent, render, screen, waitFor } from '@testing-library/svelte';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import type { MaskSnapshot } from '../selections/types';
import RefineSelectionDialog, { REFINE_SELECTION_DEFAULTS } from './RefineSelectionDialog.svelte';

const originalMask: MaskSnapshot = {
  version: 1,
  width: 2,
  height: 1,
  encoding: 'base64_u8',
  data: 'AP8',
  checksum: 'fnv1a64:0000000000000001'
};
const previewMask: MaskSnapshot = {
  ...originalMask,
  data: 'QMA',
  checksum: 'fnv1a64:0000000000000002'
};
const parameters = {
  smooth: 4,
  feather: 3,
  contrast: 0.2,
  shiftEdge: -1,
  decontaminate: false,
  decontaminateStrength: 0.5,
  decontaminateRadius: 4
};

function props(overrides: Record<string, unknown> = {}) {
  return {
    originalMask,
    previewMask,
    originalImageUrl: '',
    busy: false,
    error: '',
    parameters,
    onparameterschange: vi.fn(),
    onapply: vi.fn(),
    oncancel: vi.fn(),
    ...overrides
  };
}

beforeEach(() => {
  if (typeof HTMLDialogElement.prototype.showModal !== 'function') {
    Object.defineProperty(HTMLDialogElement.prototype, 'showModal', {
      configurable: true,
      value(this: HTMLDialogElement) { this.setAttribute('open', ''); }
    });
  }
  if (typeof HTMLDialogElement.prototype.close !== 'function') {
    Object.defineProperty(HTMLDialogElement.prototype, 'close', {
      configurable: true,
      value(this: HTMLDialogElement) { this.removeAttribute('open'); }
    });
  }
  vi.spyOn(HTMLCanvasElement.prototype, 'getContext').mockImplementation(() => ({
    createImageData: (width: number, height: number) => ({
      data: new Uint8ClampedArray(width * height * 4), width, height
    }),
    drawImage: vi.fn(),
    getImageData: (_x: number, _y: number, width: number, height: number) => ({
      data: new Uint8ClampedArray(width * height * 4), width, height
    }),
    putImageData: vi.fn(),
    clearRect: vi.fn()
  }) as unknown as CanvasRenderingContext2D);
});

afterEach(() => {
  vi.restoreAllMocks();
  vi.unstubAllGlobals();
});

describe('RefineSelectionDialog', () => {
  it('opens as a modal with real before and after coverage previews', async () => {
    const before = structuredClone(originalMask);
    const after = structuredClone(previewMask);
    const showModal = vi.spyOn(HTMLDialogElement.prototype, 'showModal').mockImplementation(function (this: HTMLDialogElement) {
      this.setAttribute('open', '');
    });
    render(RefineSelectionDialog, { props: props() });

    await waitFor(() => expect(showModal).toHaveBeenCalledTimes(1));
    expect(screen.getByRole('dialog', { name: 'Refine Selection' }).getAttribute('aria-modal')).toBe('true');
    expect(screen.getByRole('img', { name: 'Selection before refinement' })).toBeTruthy();
    expect(screen.getByRole('img', { name: 'Selection after refinement' })).toBeTruthy();
    await waitFor(() => {
      const preview = screen.getByRole('img', { name: 'Selection before refinement' });
      expect((preview.querySelector('canvas') as HTMLCanvasElement).width).toBeGreaterThan(0);
    });
    expect(originalMask).toEqual(before);
    expect(previewMask).toEqual(after);
    expect(screen.getByText(/Representative thumbnail preview/i)).toBeTruthy();
  });

  it('closes the native modal lifecycle on settlement and unmount', async () => {
    vi.spyOn(HTMLDialogElement.prototype, 'showModal').mockImplementation(function (this: HTMLDialogElement) {
      this.setAttribute('open', '');
    });
    const close = vi.spyOn(HTMLDialogElement.prototype, 'close').mockImplementation(function (this: HTMLDialogElement) {
      this.removeAttribute('open');
    });
    const view = render(RefineSelectionDialog, { props: props() });
    await fireEvent.click(screen.getByRole('button', { name: 'Cancel' }));
    expect(close).toHaveBeenCalledTimes(1);
    view.unmount();
    expect(close).toHaveBeenCalledTimes(1);
  });

  it('switches comparison and background modes locally', async () => {
    const view = render(RefineSelectionDialog, { props: props() });
    await fireEvent.click(screen.getByRole('button', { name: 'Toggle' }));
    expect(view.container.querySelector('.preview-grid')?.classList.contains('single')).toBe(true);
    await fireEvent.click(screen.getByRole('button', { name: 'Show before' }));
    expect(view.container.querySelector('.preview-card.before')?.classList.contains('hidden')).toBe(false);
    expect(view.container.querySelector('.preview-card.after')?.classList.contains('hidden')).toBe(true);
    await fireEvent.click(screen.getByRole('button', { name: 'Black' }));
    expect(view.container.querySelector('.preview-grid')?.getAttribute('data-background')).toBe('black');
    await fireEvent.click(screen.getByRole('button', { name: 'White' }));
    expect(view.container.querySelector('.preview-grid')?.getAttribute('data-background')).toBe('white');
    await fireEvent.click(screen.getByRole('button', { name: 'Mask only' }));
    expect(view.container.querySelector('.preview-grid')?.getAttribute('data-background')).toBe('mask_only');
  });

  it('emits documented defaults without mutating controlled parameters', async () => {
    const value = props();
    const originalParameters = structuredClone(parameters);
    render(RefineSelectionDialog, { props: value });
    await fireEvent.input(screen.getByRole('slider', { name: 'Refine smooth' }), { target: { value: '12' } });
    expect(value.onparameterschange).toHaveBeenCalledWith({ ...parameters, smooth: 12 });
    await fireEvent.click(screen.getByRole('button', { name: 'Reset' }));
    expect(value.onparameterschange).toHaveBeenLastCalledWith({ ...REFINE_SELECTION_DEFAULTS });
    expect(parameters).toEqual(originalParameters);
  });

  it('keeps color decontamination off by default and emits bounded controls', async () => {
    const value = props();
    const originalParameters = structuredClone(parameters);
    const view = render(RefineSelectionDialog, { props: value });
    const toggle = screen.getByRole('checkbox', { name: 'Decontaminate Colors' }) as HTMLInputElement;
    const strength = screen.getByRole('slider', { name: 'Decontaminate strength' }) as HTMLInputElement;
    const radius = screen.getByRole('slider', { name: 'Decontaminate radius' }) as HTMLInputElement;

    expect(REFINE_SELECTION_DEFAULTS.decontaminate).toBe(false);
    expect(toggle.checked).toBe(false);
    expect(strength.disabled).toBe(true);
    expect(radius.disabled).toBe(true);
    await fireEvent.click(toggle);
    expect(value.onparameterschange).toHaveBeenLastCalledWith({
      ...parameters,
      decontaminate: true
    });

    await view.rerender({
      ...value,
      parameters: { ...parameters, decontaminate: true }
    });
    await fireEvent.input(screen.getByRole('slider', { name: 'Decontaminate strength' }), {
      target: { value: '0.85' }
    });
    expect(value.onparameterschange).toHaveBeenLastCalledWith({
      ...parameters,
      decontaminate: true,
      decontaminateStrength: 0.85
    });
    await fireEvent.input(screen.getByRole('slider', { name: 'Decontaminate radius' }), {
      target: { value: '32' }
    });
    expect(value.onparameterschange).toHaveBeenLastCalledWith({
      ...parameters,
      decontaminate: true,
      decontaminateRadius: 32
    });
    expect(parameters).toEqual(originalParameters);
  });

  it('applies at most once and requires a current preview', async () => {
    const value = props();
    render(RefineSelectionDialog, { props: value });
    const apply = screen.getByRole('button', { name: 'Apply' });
    await fireEvent.click(apply);
    await fireEvent.click(apply);
    expect(value.onapply).toHaveBeenCalledTimes(1);
  });

  it('cancels exactly once from Cancel or Escape', async () => {
    const value = props();
    const view = render(RefineSelectionDialog, { props: value });
    await fireEvent.click(screen.getByRole('button', { name: 'Cancel' }));
    await fireEvent.keyDown(window, { key: 'Escape' });
    expect(value.oncancel).toHaveBeenCalledTimes(1);
    expect(value.onapply).not.toHaveBeenCalled();
    view.unmount();

    const escapeValue = props();
    render(RefineSelectionDialog, { props: escapeValue });
    await fireEvent.keyDown(window, { key: 'Escape' });
    await fireEvent.keyDown(window, { key: 'Escape' });
    expect(escapeValue.oncancel).toHaveBeenCalledTimes(1);
  });

  it('uses Enter only when focus is outside input and action controls', async () => {
    const value = props();
    render(RefineSelectionDialog, { props: value });
    const smooth = screen.getByRole('slider', { name: 'Refine smooth' });
    smooth.focus();
    await fireEvent.keyDown(smooth, { key: 'Enter' });
    expect(value.onapply).not.toHaveBeenCalled();
    const dialog = screen.getByRole('dialog', { name: 'Refine Selection' });
    dialog.focus();
    expect(await fireEvent.keyDown(dialog, { key: 'Enter' })).toBe(false);
    expect(value.onapply).toHaveBeenCalledTimes(1);
  });

  it('disables confirmation and parameter changes while busy or without a preview', () => {
    const view = render(RefineSelectionDialog, { props: props({ busy: true }) });
    expect((screen.getByRole('button', { name: 'Apply' }) as HTMLButtonElement).disabled).toBe(true);
    expect((screen.getByRole('button', { name: 'Reset' }) as HTMLButtonElement).disabled).toBe(true);
    expect((screen.getByRole('slider', { name: 'Refine smooth' }) as HTMLInputElement).disabled).toBe(true);
    expect((screen.getByRole('checkbox', { name: 'Decontaminate Colors' }) as HTMLInputElement).disabled).toBe(true);
    view.unmount();

    render(RefineSelectionDialog, { props: props({ previewMask: null }) });
    expect((screen.getByRole('button', { name: 'Apply' }) as HTMLButtonElement).disabled).toBe(true);
    expect(screen.getByText('Preview unavailable')).toBeTruthy();
  });

  it('surfaces thumbnail render failures and keeps Apply and Enter fail-closed', async () => {
    const value = props({
      previewMask: { ...previewMask, width: 0, data: '' }
    });
    const view = render(RefineSelectionDialog, { props: value });

    expect((await screen.findByRole('alert')).textContent).toContain(
      'Representative preview unavailable. Mask dimensions are invalid for thumbnail generation.'
    );
    expect((screen.getByRole('button', { name: 'Apply' }) as HTMLButtonElement).disabled).toBe(true);
    expect((screen.getByRole('button', { name: 'Cancel' }) as HTMLButtonElement).disabled).toBe(false);
    expect((screen.getByRole('button', { name: 'Reset' }) as HTMLButtonElement).disabled).toBe(false);

    const dialog = screen.getByRole('dialog', { name: 'Refine Selection' });
    dialog.focus();
    await fireEvent.keyDown(dialog, { key: 'Enter' });
    expect(value.onapply).not.toHaveBeenCalled();

    await fireEvent.click(screen.getByRole('button', { name: 'Reset' }));
    expect(value.onparameterschange).toHaveBeenCalledWith({ ...REFINE_SELECTION_DEFAULTS });
    expect(screen.queryByRole('alert')).toBeNull();
    expect((screen.getByRole('button', { name: 'Apply' }) as HTMLButtonElement).disabled).toBe(true);

    await view.rerender({ ...value, previewMask, parameters: { ...REFINE_SELECTION_DEFAULTS } });
    await waitFor(() => expect(screen.queryByRole('alert')).toBeNull());
    expect((screen.getByRole('button', { name: 'Apply' }) as HTMLButtonElement).disabled).toBe(false);
  });

  it('surfaces a decontamination work-limit failure from the current representative render', async () => {
    class LoadedImage {
      onload: ((event: Event) => void) | null = null;
      onerror: ((event: Event) => void) | null = null;
      set src(_value: string) {
        queueMicrotask(() => this.onload?.(new Event('load')));
      }
    }
    vi.stubGlobal('Image', LoadedImage);
    const width = 240;
    const height = 180;
    const edgeCoverage = new Uint8Array(width * height).fill(128);
    const boundedMask: MaskSnapshot = {
      version: 1,
      width,
      height,
      encoding: 'base64_u8',
      data: Buffer.from(edgeCoverage).toString('base64'),
      checksum: 'fnv1a64:0000000000000128'
    };
    const value = props({
      previewMask: boundedMask,
      originalImageUrl: 'data:image/png;base64,AA==',
      parameters: {
        ...parameters,
        decontaminate: true,
        decontaminateStrength: 1,
        decontaminateRadius: 32
      }
    });

    render(RefineSelectionDialog, { props: value });

    await waitFor(() => {
      expect(screen.getByRole('alert').textContent).toContain(
        'Representative preview unavailable. Decontamination preview exceeds its bounded work limit.'
      );
    });
    expect((screen.getByRole('button', { name: 'Apply' }) as HTMLButtonElement).disabled).toBe(true);
    expect(
      await fireEvent.keyDown(screen.getByRole('dialog', { name: 'Refine Selection' }), { key: 'Enter' })
    ).toBe(false);
    expect(value.onapply).not.toHaveBeenCalled();
  });

  it('fails closed when Decontaminate Colors cannot render the source image', async () => {
    class FailedImage {
      onload: ((event: Event) => void) | null = null;
      onerror: ((event: Event) => void) | null = null;
      set src(_value: string) {
        queueMicrotask(() => this.onerror?.(new Event('error')));
      }
    }
    vi.stubGlobal('Image', FailedImage);
    const value = props({
      originalImageUrl: 'data:image/png;base64,invalid',
      parameters: { ...parameters, decontaminate: true }
    });

    render(RefineSelectionDialog, { props: value });

    expect((await screen.findByRole('alert')).textContent).toContain(
      'Representative preview unavailable. The source image is unavailable for the Decontaminate Colors preview.'
    );
    expect((screen.getByRole('button', { name: 'Apply' }) as HTMLButtonElement).disabled).toBe(true);
    await fireEvent.keyDown(screen.getByRole('dialog', { name: 'Refine Selection' }), { key: 'Enter' });
    expect(value.onapply).not.toHaveBeenCalled();
  });
});
