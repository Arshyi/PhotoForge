import { fireEvent, render, waitFor, within } from '@testing-library/svelte';
import { invoke } from '@tauri-apps/api/core';
import { beforeEach, describe, expect, it, vi } from 'vitest';
import type { ComponentSnapshot } from '../types/editor';
import ComponentsSettings from './ComponentsSettings.svelte';

vi.mock('@tauri-apps/api/core', () => ({ invoke: vi.fn() }));
const invokeMock = vi.mocked(invoke);

function snapshot(): ComponentSnapshot {
  return {
    applicationVersion: '0.5.0',
    configuration: {
      activePlanner: 'rule', activeEngine: 'deterministic',
      plannerEndpoint: 'http://localhost:11434', initializationTimeoutMs: 5000,
      ollamaTimeoutMs: 15000, ollamaMaxResponseBytes: 262144,
      ollamaSelectedModel: null, ollamaMaxOperations: 8,
      modelDirectories: ['C:\\Models'], pluginDirectory: 'C:\\Plugins'
    },
    planners: [
      { id: 'rule', name: 'Rule Planner', version: '0.5.0', provider: 'PhotoForge', memoryEstimateMb: 1, installed: true, loaded: true, active: true, unavailableReason: null, capabilities: { supportsGuidedEditing: true, supportsReasoning: false, requiresModel: false, offline: true } },
      { id: 'ollama', name: 'Ollama Planner', version: '0.5.0', provider: 'Ollama local API', memoryEstimateMb: 1, installed: true, loaded: false, active: false, unavailableReason: null, capabilities: { supportsGuidedEditing: true, supportsReasoning: true, requiresModel: true, offline: true } }
    ],
    engines: [
      { id: 'deterministic', name: 'Deterministic Engine', version: '0.5.0', provider: 'PhotoForge', memoryEstimateMb: 4, installed: true, loaded: true, active: true, unavailableReason: null, capabilities: { supportsRestoration: true, supportsNeuralModels: false, requiresModel: false, offline: true, preservesAlpha: true, maxInputMegapixels: 80 } },
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
    expect(view.getAllByText('Unavailable')).toHaveLength(1);
    expect(view.getAllByText('Component not installed.')).toHaveLength(1);
  });

  it('keeps unavailable provider choices visible and disabled', async () => {
    invokeMock.mockResolvedValueOnce(snapshot());
    const view = render(ComponentsSettings);
    const planner = await view.findByLabelText('Active planner');
    expect((within(planner).getByRole('option', { name: /Ollama Planner/ }) as HTMLOptionElement).disabled).toBe(false);
    expect((within(view.getByLabelText('Active restoration engine')).getByRole('option', { name: /ONNX Restoration/ }) as HTMLOptionElement).disabled).toBe(true);
  });

  it('shows the local Ollama endpoint without connecting', async () => {
    invokeMock.mockResolvedValueOnce(snapshot());
    const view = render(ComponentsSettings);
    expect(await view.findByDisplayValue('http://localhost:11434')).toBeTruthy();
    expect(invokeMock).toHaveBeenCalledTimes(1);
  });

  it('tests Ollama only after an explicit click', async () => {
    invokeMock.mockResolvedValueOnce(snapshot()).mockResolvedValueOnce({ connected: true, message: 'Connected to local Ollama 0.11.0.', version: '0.11.0', responseTimeMs: 2.4 });
    const view = render(ComponentsSettings);
    await fireEvent.click(await view.findByRole('button', { name: 'Test Connection' }));
    await waitFor(() => expect(view.getByText('Connected to local Ollama 0.11.0. 2.4 ms')).toBeTruthy());
    expect(invokeMock).toHaveBeenLastCalledWith('test_ollama_connection');
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

  it('exposes every bounded Ollama setting with an accessible label', async () => {
    invokeMock.mockResolvedValueOnce(snapshot());
    const view = render(ComponentsSettings);
    expect(await view.findByLabelText('Ollama timeout')).toBeTruthy();
    expect(view.getByLabelText('Maximum response size')).toBeTruthy();
    expect(view.getByLabelText('Maximum generated operations')).toBeTruthy();
    expect(view.getByLabelText('Planner model')).toBeTruthy();
    expect(view.getByRole('button', { name: 'Reset Ollama defaults' })).toBeTruthy();
  });

  it('refreshes and displays installed Ollama model metadata', async () => {
    invokeMock.mockResolvedValueOnce(snapshot()).mockResolvedValueOnce({
      models: [{ name: 'gemma3:4b', sizeBytes: 4 * 1024 ** 3, modifiedAt: '2026-07-01T00:00:00Z', capabilities: ['completion', 'format: gguf'] }],
      message: 'Found 1 installed local model(s).', responseTimeMs: 3
    });
    const view = render(ComponentsSettings);
    await fireEvent.click(await view.findByRole('button', { name: 'Refresh Models' }));
    await waitFor(() => expect(view.getByText(/gemma3:4b · 4.0 GiB/)).toBeTruthy());
    expect(view.getByText(/2026-07-01 · completion · format: gguf/)).toBeTruthy();
    expect(invokeMock).toHaveBeenLastCalledWith('refresh_ollama_models');
  });

  it('persists the selected installed planner model', async () => {
    const saved = snapshot();
    saved.configuration.ollamaSelectedModel = 'gemma3:4b';
    invokeMock
      .mockResolvedValueOnce(snapshot())
      .mockResolvedValueOnce({ models: [{ name: 'gemma3:4b', sizeBytes: 1, modifiedAt: '2026-07-01', capabilities: [] }], message: 'Found 1 installed local model(s).', responseTimeMs: 1 })
      .mockResolvedValueOnce(saved);
    const view = render(ComponentsSettings);
    await fireEvent.click(await view.findByRole('button', { name: 'Refresh Models' }));
    await fireEvent.change(view.getByLabelText('Planner model'), { target: { value: 'gemma3:4b' } });
    await fireEvent.click(view.getByRole('button', { name: 'Save locally' }));
    await waitFor(() => expect(invokeMock.mock.calls.some(([command, args]) =>
      command === 'update_component_configuration' &&
      (args as { configuration: ComponentSnapshot['configuration'] }).configuration.ollamaSelectedModel === 'gemma3:4b'
    )).toBe(true));
  });

  it('resets visible Ollama settings without saving automatically', async () => {
    const customized = snapshot();
    customized.configuration = { ...customized.configuration, plannerEndpoint: 'http://localhost:9999', ollamaTimeoutMs: 30000, ollamaMaxResponseBytes: 1048576, ollamaSelectedModel: 'custom', ollamaMaxOperations: 3 };
    invokeMock.mockResolvedValueOnce(customized);
    const view = render(ComponentsSettings);
    await fireEvent.click(await view.findByRole('button', { name: 'Reset Ollama defaults' }));
    expect(view.getByDisplayValue('http://127.0.0.1:11434')).toBeTruthy();
    expect((view.getByLabelText('Ollama timeout') as HTMLInputElement).value).toBe('15000');
    expect(invokeMock).toHaveBeenCalledTimes(1);
  });

  it('can activate the installed Ollama adapter without connecting', async () => {
    const selected = snapshot();
    selected.configuration.activePlanner = 'ollama';
    invokeMock.mockResolvedValueOnce(snapshot()).mockResolvedValueOnce(selected);
    const view = render(ComponentsSettings);
    await fireEvent.change(await view.findByLabelText('Active planner'), { target: { value: 'ollama' } });
    await waitFor(() => expect(invokeMock).toHaveBeenLastCalledWith('select_planner_provider', { provider: 'ollama' }));
  });
});
