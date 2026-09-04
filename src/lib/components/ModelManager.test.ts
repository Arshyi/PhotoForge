import { fireEvent, render, waitFor } from '@testing-library/svelte';
import { invoke } from '@tauri-apps/api/core';
import { beforeEach, describe, expect, it, vi } from 'vitest';
import ModelManager from './ModelManager.svelte';

vi.mock('@tauri-apps/api/core', () => ({ invoke: vi.fn() }));
const invokeMock = vi.mocked(invoke);

// Braces matter: mockReset() returns the mock, and a value returned from
// beforeEach is treated by vitest as a teardown callback.
beforeEach(() => {
  invokeMock.mockReset();
});

const capabilities = (available: boolean, reason: string) =>
  [
    'superResolution',
    'denoise',
    'deblur',
    'artifactRemoval',
    'segmentation',
    'faceRestoration',
    'inpainting'
  ].map((capability) => ({ capability, available, models: [], reason }));

const emptyStatus = {
  runtimeCompiled: true,
  runtime: 'tract (pure Rust, CPU)',
  modelDirectory: 'C:\\Local\\PhotoForge\\inference-models',
  installed: [],
  capabilities: capabilities(false, 'No model for this capability is installed.'),
  classicalOnly: true
};

const model = {
  id: 'upscale-2x',
  name: 'Test Upscaler',
  version: '1.0',
  architecture: 'test',
  capability: 'superResolution',
  format: 'onnx',
  colorSpace: 'encodedSrgb',
  normalization: 'unitRange',
  inputChannels: 3,
  outputChannels: 3,
  scale: 2,
  tileSize: 128,
  tileOverlap: 16,
  fileBytes: 4096,
  sha256: 'a'.repeat(64),
  license: 'CC0',
  source: 'authored for tests'
};

describe('ModelManager', () => {
  it('says plainly that the editor works with no model installed', async () => {
    invokeMock.mockResolvedValue(emptyStatus as never);
    const view = render(ModelManager);
    expect(await view.findByText('No models installed.')).toBeTruthy();
    expect(view.getByTestId('classical-only')).toBeTruthy();
    expect(view.getByText(/never downloads models/)).toBeTruthy();
  });

  it('reports why each capability is unavailable rather than hiding it', async () => {
    invokeMock.mockResolvedValue(emptyStatus as never);
    const view = render(ModelManager);
    await view.findByText('No models installed.');
    const states = view.getAllByText('Unavailable');
    expect(states).toHaveLength(7);
    expect(view.getAllByText(/No model for this capability is installed/)).toHaveLength(7);
  });

  it('shows an installed model with the hash it was installed under', async () => {
    invokeMock.mockResolvedValue({
      ...emptyStatus,
      installed: [model],
      classicalOnly: false,
      capabilities: emptyStatus.capabilities.map((capability) =>
        capability.capability === 'superResolution'
          ? { ...capability, available: true, models: ['upscale-2x'], reason: '' }
          : capability
      )
    } as never);
    const view = render(ModelManager);
    expect(await view.findByText('Test Upscaler')).toBeTruthy();
    expect(view.getByText('a'.repeat(64))).toBeTruthy();
    expect(view.getByText('Licence: CC0')).toBeTruthy();
    expect(view.getByText('Available')).toBeTruthy();
    expect(view.queryByTestId('classical-only')).toBeNull();
  });

  it('reports a missing licence as unknown rather than as permissive', async () => {
    invokeMock.mockResolvedValue({
      ...emptyStatus,
      installed: [{ ...model, license: '' }],
      classicalOnly: false
    } as never);
    const view = render(ModelManager);
    expect(await view.findByText('Licence: not stated')).toBeTruthy();
  });

  it('removes a model by id and shows the result', async () => {
    invokeMock.mockImplementation((command) => {
      if (command === 'remove_inference_model') return Promise.resolve(emptyStatus) as never;
      return Promise.resolve({
        ...emptyStatus,
        installed: [model],
        classicalOnly: false
      }) as never;
    });
    const view = render(ModelManager);
    await fireEvent.click(await view.findByRole('button', { name: 'Remove Test Upscaler' }));
    await waitFor(() => expect(view.getByText('No models installed.')).toBeTruthy());
    expect(invokeMock).toHaveBeenCalledWith('remove_inference_model', { id: 'upscale-2x' });
  });

  it('surfaces a failure instead of rendering nothing', async () => {
    invokeMock.mockRejectedValue(new Error('the model store is unreadable') as never);
    const view = render(ModelManager);
    expect(await view.findByRole('alert')).toBeTruthy();
    expect(view.getByText(/unreadable/)).toBeTruthy();
  });
});
