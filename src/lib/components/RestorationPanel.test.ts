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
});
