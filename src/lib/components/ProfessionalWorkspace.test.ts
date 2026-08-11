import { fireEvent, render, screen, waitFor } from '@testing-library/svelte';
import { beforeEach, describe, expect, it, vi } from 'vitest';
import { invoke } from '@tauri-apps/api/core';
import ProfessionalWorkspace from './ProfessionalWorkspace.svelte';
import type { EditOperation, HistogramChannels, ImageMetadata } from '../types/editor';
import { createWorkflow, saveWorkflows } from '../utils/workflows';

vi.mock('@tauri-apps/api/core', () => ({ invoke: vi.fn() }));
vi.mock('@tauri-apps/plugin-dialog', () => ({ open: vi.fn(), save: vi.fn() }));

const bins = (): HistogramChannels => ({ red: Array(256).fill(1), green: Array(256).fill(1), blue: Array(256).fill(1), luminance: Array(256).fill(1), shadowClipping: 0, highlightClipping: 0, pixelCount: 256 });
const metadata: ImageMetadata = { filename: 'photo.png', width: 100, height: 80, format: 'PNG', fileSize: 2048, colorSpace: 'sRGB', bitDepth: 8, hasAlpha: true, createdAt: '1Z', modifiedAt: '2Z', cameraModel: 'Test Camera', exifAvailable: true };

function setup(
  oncommit = vi.fn(),
  onmessage = vi.fn(),
  operations: EditOperation[] = [{ type: 'brightness', amount: 0.1 }]
) {
  return render(ProfessionalWorkspace, {
    documentId: 1,
    metadata,
    operations,
    oncommit,
    onmessage,
    onviewchange: vi.fn()
  });
}

describe('ProfessionalWorkspace', () => {
  beforeEach(() => {
    localStorage.clear();
    vi.mocked(invoke).mockReset().mockImplementation(async (command) => {
      if (command === 'generate_histogram') return { before: bins(), after: bins(), documentId: 1, requestId: 1, processingTimeMs: 1, isCurrent: true };
      if (command === 'inspect_image_pixel') return { x: 1, y: 2, red: 3, green: 4, blue: 5, alpha: 255, hue: 210, saturation: .4, value: .5 };
      return {};
    });
  });

  it('exposes accessible professional tabs', () => {
    setup();
    expect(screen.getAllByRole('tab')).toHaveLength(5);
    expect(screen.getByRole('tab', { name: 'Tools' }).getAttribute('aria-selected')).toBe('true');
  });

  it.each(['Curves', 'Levels', 'White & black point', 'Crop', 'Straighten', 'Perspective', 'Lens correction', 'HSL', 'Temperature & tint', 'Selective color'])('renders %s professional tool', (name) => {
    setup(); expect(screen.getByText(name)).toBeTruthy();
  });

  it.each(['linear', 'contrast', 'matte', 'bright'])('applies %s curve preset', async (name) => {
    const commit = vi.fn(); setup(commit);
    await fireEvent.click(screen.getByRole('button', { name }));
    expect(commit).toHaveBeenCalled();
  });

  it('hydrates lens controls across replay and undo without clobbering a queued slider edit', async () => {
    const commit = vi.fn();
    const brightness: EditOperation = { type: 'brightness', amount: 0.1 };
    const initialLens: EditOperation = {
      type: 'lens_correction', distortion: 0.2, vignetting: 0.4, chromatic_aberration: 0.6
    };
    const view = setup(commit, vi.fn(), [brightness, initialLens]);
    const distortionControl = screen.getByLabelText('Barrel / pincushion') as HTMLInputElement;
    const vignettingControl = screen.getByLabelText('Vignetting') as HTMLInputElement;
    const chromaticControl = screen.getByLabelText('Chromatic aberration') as HTMLInputElement;

    expect(distortionControl.value).toBe('0.2');
    expect(vignettingControl.value).toBe('0.4');
    expect(chromaticControl.value).toBe('0.6');

    await fireEvent.input(distortionControl, { target: { value: '0.25' } });
    expect(commit).toHaveBeenLastCalledWith([
      brightness,
      { type: 'lens_correction', distortion: 0.25, vignetting: 0.4, chromatic_aberration: 0.6 }
    ], 'lens_correction');

    await fireEvent.input(vignettingControl, { target: { value: '0.5' } });
    const latestLens: EditOperation = {
      type: 'lens_correction', distortion: 0.25, vignetting: 0.5, chromatic_aberration: 0.6
    };
    expect(commit).toHaveBeenLastCalledWith([brightness, latestLens], 'lens_correction');

    // A slower geometry transaction may acknowledge the previous input first.
    await view.rerender({
      operations: [brightness, {
        type: 'lens_correction', distortion: 0.25, vignetting: 0.4, chromatic_aberration: 0.6
      }]
    });
    expect(vignettingControl.value).toBe('0.5');

    await view.rerender({ operations: [brightness, latestLens] });
    expect(vignettingControl.value).toBe('0.5');

    const replayedLens: EditOperation = {
      type: 'lens_correction', distortion: -0.1, vignetting: -0.2, chromatic_aberration: 0.8
    };
    await view.rerender({ operations: [brightness, replayedLens] });
    expect(distortionControl.value).toBe('-0.1');
    expect(vignettingControl.value).toBe('-0.2');
    expect(chromaticControl.value).toBe('0.8');

    await fireEvent.input(chromaticControl, { target: { value: '0.7' } });
    expect(commit).toHaveBeenLastCalledWith([
      brightness,
      { type: 'lens_correction', distortion: -0.1, vignetting: -0.2, chromatic_aberration: 0.7 }
    ], 'lens_correction');

    await view.rerender({ operations: [brightness] });
    expect(distortionControl.value).toBe('0');
    expect(vignettingControl.value).toBe('0');
    expect(chromaticControl.value).toBe('0');
  });

  it('keeps an asynchronously accepted lens draft and rolls back an asynchronously rejected one', async () => {
    let resolveAccepted: ((accepted: boolean) => void) | undefined;
    const commit = vi.fn()
      .mockImplementationOnce(() => new Promise<boolean>((resolve) => { resolveAccepted = resolve; }))
      .mockResolvedValueOnce(false);
    const initialLens: EditOperation = {
      type: 'lens_correction', distortion: 0.2, vignetting: 0.4, chromatic_aberration: 0.6
    };
    setup(commit, vi.fn(), [initialLens]);
    const distortionControl = screen.getByLabelText('Barrel / pincushion') as HTMLInputElement;

    await fireEvent.input(distortionControl, { target: { value: '0.3' } });
    expect(distortionControl.value).toBe('0.3');
    resolveAccepted?.(true);
    await waitFor(() => expect(commit).toHaveBeenCalledTimes(1));
    expect(distortionControl.value).toBe('0.3');

    await fireEvent.input(distortionControl, { target: { value: '0.5' } });
    await waitFor(() => expect(distortionControl.value).toBe('0.2'));
    expect(commit).toHaveBeenLastCalledWith([
      { type: 'lens_correction', distortion: 0.5, vignetting: 0.4, chromatic_aberration: 0.6 }
    ], 'lens_correction');
  });

  it('shows live before and after histogram', async () => {
    setup(); await fireEvent.click(screen.getByRole('tab', { name: 'Scopes' }));
    await waitFor(() => expect(screen.getByRole('img', { name: /after RGB/ })).toBeTruthy());
    expect(screen.getByText('Shadow clipping')).toBeTruthy();
  });

  it.each(['swipe', 'split', 'blink', 'difference'])('offers %s comparison', async (mode) => {
    setup(); await fireEvent.click(screen.getByRole('tab', { name: 'Scopes' }));
    expect(screen.getByRole('button', { name: mode })).toBeTruthy();
  });

  it('records a workflow locally', async () => {
    setup(); await fireEvent.click(screen.getByRole('tab', { name: 'Flows' }));
    await fireEvent.input(screen.getByLabelText('Workflow name'), { target: { value: 'My Flow' } });
    await fireEvent.click(screen.getByRole('button', { name: /Save workflow/ }));
    expect(localStorage.getItem('photoforge.workflows.v1')).toContain('My Flow');
  });

  it('provides workflow search and JSON transfer', async () => {
    setup(); await fireEvent.click(screen.getByRole('tab', { name: 'Flows' }));
    expect(screen.getByLabelText('Search workflows')).toBeTruthy();
    expect(screen.getByRole('button', { name: 'Import JSON' })).toBeTruthy();
    expect((screen.getByRole('button', { name: 'Export JSON' }) as HTMLButtonElement).disabled).toBe(true);
  });

  it('waits for asynchronous workflow replay without claiming premature success', async () => {
    let resolveCommit: ((value: boolean) => void) | undefined;
    const commit = vi.fn(() => new Promise<boolean>((resolve) => { resolveCommit = resolve; }));
    const message = vi.fn();
    saveWorkflows([createWorkflow('Async Flow', [{ type: 'brightness', amount: 0.2 }])]);
    setup(commit, message);
    await fireEvent.click(screen.getByRole('tab', { name: 'Flows' }));
    await fireEvent.click(screen.getByRole('button', { name: 'Replay' }));
    expect(commit).toHaveBeenCalledWith([{ type: 'brightness', amount: 0.2 }]);
    expect(message).not.toHaveBeenCalled();
    resolveCommit?.(true);
    await waitFor(() => expect(commit).toHaveBeenCalledTimes(1));
    expect(message).not.toHaveBeenCalled();
  });

  it('reports asynchronous workflow replay failure', async () => {
    const commit = vi.fn(async () => { throw new Error('geometry reconciliation failed'); });
    const message = vi.fn();
    saveWorkflows([createWorkflow('Failing Flow', [{ type: 'brightness', amount: 0.2 }])]);
    setup(commit, message);
    await fireEvent.click(screen.getByRole('tab', { name: 'Flows' }));
    await fireEvent.click(screen.getByRole('button', { name: 'Replay' }));
    await waitFor(() => expect(message).toHaveBeenCalledWith('geometry reconciliation failed', 'error'));
  });

  it('rejects malformed typed operation JSON without changing storage', async () => {
    const message = vi.fn();
    saveWorkflows([createWorkflow('Editable Flow', [{ type: 'brightness', amount: 0.2 }])]);
    setup(vi.fn(), message);
    await fireEvent.click(screen.getByRole('tab', { name: 'Flows' }));
    await fireEvent.click(screen.getByText('Editable Flow'));
    const before = localStorage.getItem('photoforge.workflows.v1');
    await fireEvent.input(screen.getByLabelText('Typed operation JSON'), { target: { value: '[{"type":"brightness","amount":99}]' } });
    await fireEvent.click(screen.getByRole('button', { name: 'Validate & update' }));
    expect(message).toHaveBeenCalledWith(expect.stringMatching(/Operation 1/), 'error');
    expect(localStorage.getItem('photoforge.workflows.v1')).toBe(before);
  });

  it.each(['Input folder', 'Output folder', 'Workflow', 'Filename template', 'Export profile', 'Bounded workers'])('exposes batch field %s', async (label) => {
    setup(); await fireEvent.click(screen.getByRole('tab', { name: 'Batch' }));
    expect(screen.getByText(label)).toBeTruthy();
  });

  it('explains bounded offline batch behavior', async () => {
    setup(); await fireEvent.click(screen.getByRole('tab', { name: 'Batch' }));
    expect(screen.getByText(/stays offline and uses bounded workers/i)).toBeTruthy();
  });

  it.each(['Dimensions', 'Color space', 'Bit depth', 'Alpha', 'Camera', 'EXIF', 'Created', 'Modified'])('shows metadata %s', async (label) => {
    setup(); await fireEvent.click(screen.getByRole('tab', { name: 'Inspect' }));
    expect(screen.getByText(label)).toBeTruthy();
  });

  it('supports pixel inspection and measurement', async () => {
    setup(); await fireEvent.click(screen.getByRole('tab', { name: 'Inspect' }));
    await fireEvent.click(screen.getByRole('button', { name: 'Inspect pixel' }));
    await waitFor(() => expect(screen.getByText(/RGB 3, 4, 5/)).toBeTruthy());
    expect(screen.getByText('0.00 pixels')).toBeTruthy();
  });

  it.each(['Crosshair', 'Pixel grid', 'Zoom 1600%'])('provides inspection control %s', async (name) => {
    setup(); await fireEvent.click(screen.getByRole('tab', { name: 'Inspect' }));
    expect(screen.getByRole('button', { name })).toBeTruthy();
  });
});
