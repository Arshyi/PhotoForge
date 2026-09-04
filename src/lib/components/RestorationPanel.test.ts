import { fireEvent, render } from '@testing-library/svelte';
import { describe, expect, it, vi } from 'vitest';
import RestorationPanel from './RestorationPanel.svelte';

describe('RestorationPanel', () => {
  it('renders all restoration controls with accessible labels', () => {
    const view = render(RestorationPanel, { props: { operations: [], onset: vi.fn() } });
    for (const label of [
      'Auto White Balance',
      'Local Contrast',
      'Denoise',
      'JPEG Cleanup',
      'Edge-Aware Sharpen',
      'Mild Deblur',
      'Uneven Lighting',
      'Document Enhance'
    ]) {
      expect(view.getByLabelText(label)).toBeTruthy();
    }
    expect(view.getByText(/No content is generated/)).toBeTruthy();
  });

  it('expands advanced controls and emits a bounded typed operation', async () => {
    const onset = vi.fn();
    const view = render(RestorationPanel, { props: { operations: [], onset } });
    await fireEvent.click(view.getByRole('button', { name: 'Advanced contrast controls' }));
    expect(view.getByLabelText('Contrast tile size')).toBeTruthy();
    await fireEvent.input(view.getByLabelText('Contrast tile size'), { target: { value: '64' } });
    expect(onset).toHaveBeenLastCalledWith(
      { type: 'local_contrast', strength: 0.01, tile_size: 64, clip_limit: 1.5 },
      true,
      'local_contrast:tile_size'
    );
  });

  it('shows the strong deblur warning', () => {
    const view = render(RestorationPanel, {
      props: {
        operations: [{ type: 'mild_deblur', strength: 0.8, radius: 1.2 }],
        onset: vi.fn()
      }
    });
    expect(view.getByRole('note').textContent).toContain('may amplify noise or create halos');
  });

  it('offers the new defect and deconvolution tools with accessible labels', () => {
    const view = render(RestorationPanel, { props: { operations: [], onset: vi.fn() } });
    expect(view.getByLabelText('Dust & hot pixels')).toBeTruthy();
    expect(view.getByLabelText('Iterations')).toBeTruthy();
    expect(view.getByLabelText('Ringing control')).toBeTruthy();
    expect(view.getByRole('group', { name: 'Deconvolution' })).toBeTruthy();
  });

  it('emits a bounded defect operation with its sensitivity', async () => {
    const onset = vi.fn();
    const view = render(RestorationPanel, { props: { operations: [], onset } });
    await fireEvent.input(view.getByLabelText('Dust & hot pixels'), { target: { value: '0.8' } });
    expect(onset).toHaveBeenLastCalledWith(
      { type: 'remove_defects', strength: 0.8, threshold: 3 },
      true,
      'remove_defects'
    );
  });

  it('names the blur being reversed rather than guessing it', async () => {
    const onset = vi.fn();
    const view = render(RestorationPanel, { props: { operations: [], onset } });
    await fireEvent.click(view.getByRole('button', { name: 'Motion' }));
    expect(onset).toHaveBeenLastCalledWith(
      {
        type: 'deconvolve',
        kernel: { type: 'motion', angleDegrees: 0, distance: 7 },
        iterations: 12,
        damping: 0.3
      },
      true,
      'deconvolve:kernel'
    );
  });

  it('exposes colour noise separately from luminance noise', async () => {
    const onset = vi.fn();
    const view = render(RestorationPanel, {
      props: { operations: [{ type: 'denoise', strength: 0.5, preserve_edges: 0.82, color: 0.5 }], onset }
    });
    await fireEvent.click(view.getByRole('button', { name: 'Advanced denoise controls' }));
    await fireEvent.input(view.getByLabelText('Colour noise'), { target: { value: '0.9' } });
    expect(onset).toHaveBeenLastCalledWith(
      { type: 'denoise', strength: 0.5, preserve_edges: 0.82, color: 0.9 },
      true,
      'denoise:color'
    );
  });
});
