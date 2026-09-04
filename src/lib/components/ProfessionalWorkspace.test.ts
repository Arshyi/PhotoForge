import { fireEvent, render, screen, waitFor } from '@testing-library/svelte';
import { beforeEach, describe, expect, it, vi } from 'vitest';
import { invoke } from '@tauri-apps/api/core';
import { open, save } from '@tauri-apps/plugin-dialog';
import ProfessionalWorkspace from './ProfessionalWorkspace.svelte';
import type { EditOperation, HistogramChannels, ImageMetadata, Workflow } from '../types/editor';
import { createWorkflow, saveWorkflows } from '../utils/workflows';
import { createDocument, createPixelLayer } from '../layers/tree';
import { tick } from 'svelte';

vi.mock('@tauri-apps/api/core', () => ({ invoke: vi.fn() }));
vi.mock('@tauri-apps/plugin-dialog', () => ({ open: vi.fn(), save: vi.fn() }));

const bins = (): HistogramChannels => ({ red: Array(256).fill(1), green: Array(256).fill(1), blue: Array(256).fill(1), luminance: Array(256).fill(1), shadowClipping: 0, highlightClipping: 0, pixelCount: 256 });
const metadata: ImageMetadata = { filename: 'photo.png', width: 100, height: 80, format: 'PNG', fileSize: 2048, colorSpace: 'sRGB', bitDepth: 8, hasAlpha: true, createdAt: '1Z', modifiedAt: '2Z', cameraModel: 'Test Camera', exifAvailable: true };

function setup(
  oncommit = vi.fn(),
  onmessage = vi.fn(),
  operations: EditOperation[] = [{ type: 'brightness', amount: 0.1 }],
  onworkflow: (workflow: Workflow) => boolean | Promise<boolean> = vi.fn(() => true)
) {
  return render(ProfessionalWorkspace, {
    documentId: 1,
    metadata,
    operations,
    oncommit,
    onworkflow,
    onmessage,
    onviewchange: vi.fn()
  });
}

describe('ProfessionalWorkspace', () => {
  beforeEach(() => {
    localStorage.clear();
    vi.mocked(open).mockReset();
    vi.mocked(save).mockReset();
    vi.mocked(invoke).mockReset().mockImplementation(async (command, args) => {
      if (command === 'generate_histogram') return { before: bins(), after: bins(), documentId: (args as { documentId: number }).documentId, requestId: (args as { requestId: number }).requestId, processingTimeMs: 1, isCurrent: true };
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

  it('includes the current layer tree when requesting histogram, pixel, and point samples', async () => {
    const layerDocument = createDocument(100, 80, [createPixelLayer('Image', 'image', 100, 80)]);
    const view = setup(); await view.rerender({ layerDocument, layerRevision: 2 });
    await fireEvent.click(screen.getByRole('tab', { name: 'Scopes' }));
    await fireEvent.click(screen.getByRole('button', { name: 'Refresh' }));
    expect(invoke).toHaveBeenCalledWith('generate_histogram', expect.objectContaining({ layerDocument, documentId: 1 }));
    await fireEvent.click(screen.getByRole('tab', { name: 'Inspect' }));
    await fireEvent.click(screen.getByRole('button', { name: 'Inspect pixel' }));
    expect(invoke).toHaveBeenCalledWith('inspect_image_pixel', expect.objectContaining({ layerDocument, documentId: 1 }));
    await fireEvent.click(screen.getByRole('tab', { name: 'Tools' }));
    await fireEvent.click(screen.getByText('White & black point'));
    await fireEvent.click(screen.getByRole('button', { name: 'Pick white' }));
    expect(invoke).toHaveBeenCalledWith('create_point_operation', expect.objectContaining({ layerDocument, documentId: 1 }));
  });

  it('invalidates a displayed histogram and ignores pending results after a layer revision change', async () => {
    const layerDocument = createDocument(100, 80, [createPixelLayer('Image', 'image', 100, 80)]);
    const view = setup(); await view.rerender({ layerDocument, layerRevision: 1 });
    await fireEvent.click(screen.getByRole('tab', { name: 'Scopes' }));
    await fireEvent.click(screen.getByRole('button', { name: 'Refresh' }));
    await waitFor(() => expect(screen.getByRole('img', { name: /after RGB/ })).toBeTruthy());
    let resolveHistogram: (value: unknown) => void = () => undefined;
    let requestId = 0;
    vi.mocked(invoke).mockImplementation(async (command, args) => {
      if (command === 'generate_histogram') {
        requestId = (args as { requestId: number }).requestId;
        return new Promise((resolve) => { resolveHistogram = resolve; });
      }
      return {};
    });
    await fireEvent.click(screen.getByRole('button', { name: 'Refresh' }));
    await view.rerender({ layerRevision: 2 });
    expect(screen.queryByRole('img', { name: /after RGB/ })).toBeNull();
    resolveHistogram({ before: bins(), after: bins(), documentId: 1, requestId, processingTimeMs: 1, isCurrent: true });
    await tick(); await tick();
    expect(screen.queryByRole('img', { name: /after RGB/ })).toBeNull();
    await waitFor(() => expect(invoke).toHaveBeenCalledTimes(3));
  });

  it('clears pixel inspection and rejects an old sample when layer content changes', async () => {
    const view = setup();
    await fireEvent.click(screen.getByRole('tab', { name: 'Inspect' }));
    await fireEvent.click(screen.getByRole('button', { name: 'Inspect pixel' }));
    await waitFor(() => expect(screen.getByText(/RGB 3, 4, 5/)).toBeTruthy());
    let resolveSample: (value: unknown) => void = () => undefined;
    vi.mocked(invoke).mockImplementation(async (command) => command === 'inspect_image_pixel'
      ? new Promise((resolve) => { resolveSample = resolve; }) : {});
    await fireEvent.click(screen.getByRole('button', { name: 'Inspect pixel' }));
    await view.rerender({ layerRevision: 1 });
    expect(screen.queryByText(/RGB 3, 4, 5/)).toBeNull();
    resolveSample({ x: 1, y: 2, red: 3, green: 4, blue: 5, alpha: 255, hue: 210, saturation: .4, value: .5 });
    await tick(); await tick();
    expect(screen.queryByText(/RGB 3, 4, 5/)).toBeNull();
  });

  it('never commits a white-point sample from an older layer revision', async () => {
    let resolvePoint: (value: unknown) => void = () => undefined;
    vi.mocked(invoke).mockImplementation(async (command) => command === 'create_point_operation'
      ? new Promise((resolve) => { resolvePoint = resolve; }) : {});
    const commit = vi.fn(); const message = vi.fn(); const view = setup(commit, message);
    await fireEvent.click(screen.getByText('White & black point'));
    await fireEvent.click(screen.getByRole('button', { name: 'Pick white' }));
    await view.rerender({ layerRevision: 1 });
    resolvePoint({ type: 'white_point', red: 200, green: 210, blue: 220 });
    await tick(); await tick();
    expect(commit).not.toHaveBeenCalled();
    expect(message).not.toHaveBeenCalled();
  });

  it('awaits a point edit and does not announce an asynchronously rejected commit', async () => {
    vi.mocked(invoke).mockResolvedValue({ type: 'white_point', red: 200, green: 210, blue: 220 });
    const commit = vi.fn(async () => false); const message = vi.fn(); setup(commit, message);
    await fireEvent.click(screen.getByText('White & black point'));
    await fireEvent.click(screen.getByRole('button', { name: 'Pick white' }));
    await waitFor(() => expect(commit).toHaveBeenCalled());
    expect(message).not.toHaveBeenCalled();
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
    setup(vi.fn(), message, [], commit);
    await fireEvent.click(screen.getByRole('tab', { name: 'Flows' }));
    await fireEvent.click(screen.getByRole('button', { name: 'Replay' }));
    expect(commit).toHaveBeenCalledWith(expect.objectContaining({ operations: [{ type: 'brightness', amount: 0.2 }], layerSteps: [] }));
    expect(message).not.toHaveBeenCalled();
    resolveCommit?.(true);
    await waitFor(() => expect(commit).toHaveBeenCalledTimes(1));
    expect(message).not.toHaveBeenCalled();
  });

  it('reports asynchronous workflow replay failure', async () => {
    const commit = vi.fn(async () => { throw new Error('geometry reconciliation failed'); });
    const message = vi.fn();
    saveWorkflows([createWorkflow('Failing Flow', [{ type: 'brightness', amount: 0.2 }])]);
    setup(vi.fn(), message, [], commit);
    await fireEvent.click(screen.getByRole('tab', { name: 'Flows' }));
    await fireEvent.click(screen.getByRole('button', { name: 'Replay' }));
    await waitFor(() => expect(message).toHaveBeenCalledWith('geometry reconciliation failed', 'error'));
  });

  it('ignores an obsolete workflow completion after the layer revision changes', async () => {
    let rejectReplay: (reason: unknown) => void = () => undefined;
    const replay = vi.fn(() => new Promise<boolean>((_resolve, reject) => { rejectReplay = reject; }));
    const message = vi.fn();
    saveWorkflows([createWorkflow('Old Flow', [{ type: 'grayscale' }])]);
    const view = setup(vi.fn(), message, [], replay);
    await fireEvent.click(screen.getByRole('tab', { name: 'Flows' }));
    await fireEvent.click(screen.getByRole('button', { name: 'Replay' }));
    await view.rerender({ layerRevision: 1 });
    rejectReplay(new Error('obsolete replay error'));
    await tick(); await tick();
    expect(message).not.toHaveBeenCalled();
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

  it('replays the full layer-only workflow without falling back to operation commits', async () => {
    const value = createWorkflow('Layer Flow', [], '', new Date(), [
      { type: 'set_opacity', selector: { type: 'active' }, opacity: 0.3 }
    ]);
    saveWorkflows([value]);
    const commit = vi.fn(); const replay = vi.fn(() => true);
    setup(commit, vi.fn(), [], replay);
    await fireEvent.click(screen.getByRole('tab', { name: 'Flows' }));
    await fireEvent.click(screen.getByText('Layer Flow'));
    expect(screen.getByRole('list', { name: 'Workflow layer steps' }).textContent).toContain('Layer opacity');
    await fireEvent.click(screen.getByRole('button', { name: 'Replay' }));
    expect(replay).toHaveBeenCalledWith(value);
    expect(commit).not.toHaveBeenCalled();
  });

  it('preserves layer steps when editing operations and exporting version 2', async () => {
    const value = createWorkflow('Mixed Flow', [{ type: 'grayscale' }], '', new Date(), [
      { type: 'set_visibility', selector: { type: 'active' }, visible: true }
    ]);
    saveWorkflows([value]); setup();
    await fireEvent.click(screen.getByRole('tab', { name: 'Flows' }));
    await fireEvent.click(screen.getByText('Mixed Flow'));
    await fireEvent.input(screen.getByLabelText('Typed operation JSON'), { target: { value: '[]' } });
    await fireEvent.click(screen.getByRole('button', { name: 'Validate & update' }));
    vi.mocked(save).mockResolvedValueOnce('C:\\test\\workflow.json');
    await fireEvent.click(screen.getByRole('button', { name: 'Export JSON' }));
    await waitFor(() => expect(invoke).toHaveBeenCalledWith('export_workflow', {
      path: 'C:\\test\\workflow.json', document: {
        schemaVersion: 2, workflow: expect.objectContaining({ operations: [], layerSteps: value.layerSteps })
      }
    }));
  });

  it('imports a layer-only version 2 workflow', async () => {
    const value = createWorkflow('Imported Layers', [], '', new Date(), [{ type: 'flatten' }]);
    vi.mocked(open).mockResolvedValueOnce('C:\\test\\workflow.json');
    vi.mocked(invoke).mockImplementation(async (command) => command === 'import_workflow' ? { schemaVersion: 2, workflow: value } : {});
    setup();
    await fireEvent.click(screen.getByRole('tab', { name: 'Flows' }));
    await fireEvent.click(screen.getByRole('button', { name: 'Import JSON' }));
    await waitFor(() => expect(screen.getByText('Imported Layers')).toBeTruthy());
    expect(screen.getByRole('list', { name: 'Workflow layer steps' }).textContent).toContain('Flatten image');
  });

  it('validates layer-step JSON before updating the saved workflow', async () => {
    saveWorkflows([createWorkflow('Editable Layer Flow', [{ type: 'grayscale' }])]);
    const message = vi.fn(); setup(vi.fn(), message);
    await fireEvent.click(screen.getByRole('tab', { name: 'Flows' }));
    await fireEvent.click(screen.getByText('Editable Layer Flow'));
    const original = localStorage.getItem('photoforge.workflows.v1');
    await fireEvent.input(screen.getByLabelText('Typed layer-step JSON'), { target: { value: '[{"type":"future"}]' } });
    await fireEvent.click(screen.getByRole('button', { name: 'Validate & update layer steps' }));
    expect(message).toHaveBeenCalledWith(expect.stringMatching(/unsupported layer step/), 'error');
    expect(localStorage.getItem('photoforge.workflows.v1')).toBe(original);
    await fireEvent.input(screen.getByLabelText('Typed layer-step JSON'), { target: { value: '[{"type":"flatten"}]' } });
    await fireEvent.click(screen.getByRole('button', { name: 'Validate & update layer steps' }));
    expect(JSON.parse(localStorage.getItem('photoforge.workflows.v1')!)[0].layerSteps).toEqual([{ type: 'flatten' }]);
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
