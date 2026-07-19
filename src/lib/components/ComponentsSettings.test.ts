import { fireEvent, render, waitFor, within } from '@testing-library/svelte';
import { invoke } from '@tauri-apps/api/core';
import { beforeEach, describe, expect, it, vi } from 'vitest';
import type { ComponentSnapshot } from '../types/editor';
import ComponentsSettings from './ComponentsSettings.svelte';

vi.mock('@tauri-apps/api/core', () => ({ invoke: vi.fn() }));
const invokeMock = vi.mocked(invoke);

function snapshot(): ComponentSnapshot {
  return {
    applicationVersion: '0.4.0',
    configuration: {
      activePlanner: 'rule', activeEngine: 'deterministic',
      plannerEndpoint: 'http://localhost:11434', initializationTimeoutMs: 5000,
      modelDirectories: ['C:\\Models'], pluginDirectory: 'C:\\Plugins'
    },
    planners: [
      { id: 'rule', name: 'Rule Planner', version: '0.4.0', provider: 'PhotoForge', memoryEstimateMb: 1, installed: true, loaded: true, active: true, unavailableReason: null, capabilities: { supportsGuidedEditing: true, supportsReasoning: false, requiresModel: false, offline: true } },
      { id: 'ollama', name: 'Ollama Planner', version: 'Not installed', provider: 'Ollama (future adapter)', memoryEstimateMb: 1024, installed: false, loaded: false, active: false, unavailableReason: 'Component not installed.', capabilities: { supportsGuidedEditing: true, supportsReasoning: true, requiresModel: true, offline: true } }
    ],
    engines: [
      { id: 'deterministic', name: 'Deterministic Engine', version: '0.4.0', provider: 'PhotoForge', memoryEstimateMb: 4, installed: true, loaded: true, active: true, unavailableReason: null, capabilities: { supportsRestoration: true, supportsNeuralModels: false, requiresModel: false, offline: true, preservesAlpha: true, maxInputMegapixels: 80 } },
      { id: 'onnx', name: 'ONNX Restoration', version: 'Not installed', provider: 'ONNX (future adapter)', memoryEstimateMb: 2048, installed: false, loaded: false, active: false, unavailableReason: 'Component not installed.', capabilities: { supportsRestoration: true, supportsNeuralModels: true, requiresModel: true, offline: true, preservesAlpha: true, maxInputMegapixels: 40 } }
    ]
  };
}

beforeEach(() => invokeMock.mockReset());

describe('ComponentsSettings', () => {
  it('shows active and unavailable registered components', async () => {
    invokeMock.mockResolvedValueOnce(snapshot());
    const view = render(ComponentsSettings);
    await waitFor(() => expect(view.getAllByText('Rule Planner')).toHaveLength(2));
    expect(view.getAllByText('ONNX Restoration')).toHaveLength(1);
    expect(view.getAllByText('Unavailable')).toHaveLength(2);
    expect(view.getAllByText('Component not installed.')).toHaveLength(2);
  });

  it('keeps unavailable provider choices visible and disabled', async () => {
    invokeMock.mockResolvedValueOnce(snapshot());
    const view = render(ComponentsSettings);
    const planner = await view.findByLabelText('Active planner');
    expect((within(planner).getByRole('option', { name: /Ollama Planner/ }) as HTMLOptionElement).disabled).toBe(true);
    expect((within(view.getByLabelText('Active restoration engine')).getByRole('option', { name: /ONNX Restoration/ }) as HTMLOptionElement).disabled).toBe(true);
  });

  it('shows the local Ollama placeholder endpoint without connecting', async () => {
    invokeMock.mockResolvedValueOnce(snapshot());
    const view = render(ComponentsSettings);
    expect(await view.findByDisplayValue('http://localhost:11434')).toBeTruthy();
    expect(invokeMock).toHaveBeenCalledTimes(1);
  });

  it('tests the Ollama placeholder only after an explicit click', async () => {
    invokeMock.mockResolvedValueOnce(snapshot()).mockResolvedValueOnce({ success: false, message: 'Planner not installed. No connection attempted.' });
    const view = render(ComponentsSettings);
    await fireEvent.click(await view.findByRole('button', { name: 'Test Connection' }));
    await waitFor(() => expect(view.getByText('Planner not installed. No connection attempted.')).toBeTruthy());
    expect(invokeMock).toHaveBeenLastCalledWith('test_planner_connection', { provider: 'ollama' });
  });

  it('saves bounded model directory rows as local configuration', async () => {
    invokeMock.mockResolvedValueOnce(snapshot()).mockResolvedValueOnce(snapshot());
    const view = render(ComponentsSettings);
    await fireEvent.input(await view.findByLabelText('Model directories'), { target: { value: ' C:\\One \n\nD:\\Two ' } });
    await fireEvent.click(view.getByRole('button', { name: 'Save locally' }));
    await waitFor(() => expect(invokeMock).toHaveBeenCalledTimes(2));
    expect(invokeMock.mock.calls[1]).toEqual(['update_component_configuration', {
      configuration: { ...snapshot().configuration, modelDirectories: ['C:\\One', 'D:\\Two'] }
    }]);
  });

  it('reports exact empty local-model discovery result', async () => {
    invokeMock.mockResolvedValueOnce(snapshot()).mockResolvedValueOnce({ searchedDirectories: ['C:\\Models'], models: [], message: 'No compatible models found.', processingTimeMs: 0 });
    const view = render(ComponentsSettings);
    await fireEvent.click(await view.findByRole('button', { name: 'Discover Models' }));
    await waitFor(() => expect(view.getAllByText('No compatible models found.').length).toBeGreaterThan(0));
    expect(invokeMock).toHaveBeenLastCalledWith('discover_models');
  });

  it('shows discovered model metadata as unavailable', async () => {
    invokeMock.mockResolvedValueOnce(snapshot()).mockResolvedValueOnce({ searchedDirectories: ['C:\\Models'], message: 'Found 1 model file(s). No inference runtime is installed, so none were loaded.', processingTimeMs: 1, models: [{ name: 'restore.onnx', path: 'C:\\Models\\restore.onnx', format: 'ONNX', fileSizeBytes: 10, memoryEstimateMb: 2, supportedTasks: [], expectedInput: 'not loaded', expectedOutput: 'not produced', compatible: false, unavailableReason: 'No compatible restoration engine is installed.' }] });
    const view = render(ComponentsSettings);
    await fireEvent.click(await view.findByRole('button', { name: 'Discover Models' }));
    await waitFor(() => expect(view.getByText(/restore.onnx/)).toBeTruthy());
    expect(view.getByText('No compatible restoration engine is installed.')).toBeTruthy();
  });

  it('labels validated manifests as execution disabled', async () => {
    invokeMock.mockResolvedValueOnce(snapshot()).mockResolvedValueOnce({ directory: 'C:\\Plugins', message: 'Validated 1 manifest(s); 0 invalid. Plugin execution is disabled in Phase 4.', records: [{ manifestPath: 'example.json', valid: true, error: null, executionAllowed: false, manifest: { schemaVersion: 1, name: 'Example Planner', version: '1.0.0', type: 'planner', provider: 'example', entry: 'disabled', memoryEstimateMb: 1, capabilities: [] } }] });
    const view = render(ComponentsSettings);
    await fireEvent.click(await view.findByRole('button', { name: 'Validate Plugins' }));
    await waitFor(() => expect(view.getByText('Example Planner')).toBeTruthy());
    expect(view.getByText('Manifest valid · execution disabled')).toBeTruthy();
  });

  it('surfaces registry loading failures safely', async () => {
    invokeMock.mockRejectedValueOnce({ message: 'registry is unavailable' });
    const view = render(ComponentsSettings);
    expect((await view.findByRole('alert')).textContent).toContain('registry is unavailable');
  });
});
