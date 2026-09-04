import { fireEvent, render, screen } from '@testing-library/svelte';
import { describe, expect, it, vi } from 'vitest';
import RawSourcePanel from './RawSourcePanel.svelte';
import type { RawDevelopmentParameters, RawLayerSource } from '../types/editor';

const source = (): RawLayerSource => ({
  reference: { filename: 'camera.dng', format: 'DNG', fileSize: 100, sha256: 'a'.repeat(64), width: 8, height: 8 },
  mode: { mode: 'linked', path: 'C:/photos/camera.dng' },
  parameters: { whiteBalance: { mode: 'custom', multipliers: [2, 1, 1.5] }, exposureEv: 0, contrast: 0, highlights: 0, shadows: 0, whites: 0, blacks: 0 },
  decoder: 'photoforge-dng', decoderVersion: '2',
  capture: { manufacturer: null, model: null, lens: null, focalLengthMm: null, aperture: null, shutterSpeedSeconds: null, iso: null, captureTime: null, orientation: null, exposureCompensation: null, whiteBalanceMultipliers: null },
});

describe('full-resolution RAW source development', () => {
  it('keeps slider drafts separate from immutable source/history until Apply', async () => {
    const original = source();
    const apply = vi.fn<(parameters: RawDevelopmentParameters) => Promise<void>>(async () => {});
    render(RawSourcePanel, { source: original, onapply: apply });
    await fireEvent.click(screen.getByText('Develop selected RAW source'));
    await fireEvent.input(screen.getByLabelText('Source exposure'), { target: { value: '0.25' } });
    expect(original.parameters.exposureEv).toBe(0);
    expect(apply).not.toHaveBeenCalled();
    await fireEvent.click(screen.getByRole('button', { name: 'Apply source development' }));
    expect(apply).toHaveBeenCalledExactlyOnceWith({ ...original.parameters, exposureEv: 0.25 });
    expect(apply.mock.calls[0][0]).not.toBe(original.parameters);
  });

  it('disables source actions while a document operation owns the session', async () => {
    render(RawSourcePanel, { source: source(), onapply: vi.fn(), disabled: true });
    await fireEvent.click(screen.getByText('Develop selected RAW source'));
    expect(screen.getByRole('button', { name: 'Apply source development' }).matches(':disabled')).toBe(true);
    expect(screen.getByLabelText('Source exposure').matches(':disabled')).toBe(true);
  });

  it('refreshes the draft when Undo replaces the RAW source parameters', async () => {
    const current = source();
    const { rerender } = render(RawSourcePanel, { source: current, onapply: vi.fn() });
    const restored = source();
    restored.parameters.exposureEv = -1;
    await rerender({ source: restored });
    expect((screen.getByLabelText('Source exposure') as HTMLInputElement).value).toBe('-1');
  });
});
