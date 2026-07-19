import { fireEvent, render, waitFor } from '@testing-library/svelte';
import { invoke } from '@tauri-apps/api/core';
import { beforeEach, describe, expect, it, vi } from 'vitest';
import DiagnosticsSettings from './DiagnosticsSettings.svelte';

vi.mock('@tauri-apps/api/core', () => ({ invoke: vi.fn() }));
const invokeMock = vi.mocked(invoke);

const diagnostics = {
  applicationVersion: '0.4.0',
  registeredPlanners: ['Rule Planner', 'Ollama Planner'],
  registeredEngines: ['Deterministic Engine', 'ONNX Restoration'],
  loadedComponents: ['Rule Planner', 'Deterministic Engine'],
  unavailableComponents: ['Ollama Planner', 'ONNX Restoration'],
  initializationFailures: [], pluginValidationErrors: [],
  configurationPath: 'C:\\Local\\PhotoForge\\components.json'
};

beforeEach(() => invokeMock.mockReset());

describe('DiagnosticsSettings', () => {
  it('shows app version and registry counts', async () => {
    invokeMock.mockResolvedValueOnce(diagnostics);
    const view = render(DiagnosticsSettings);
    expect(await view.findByText('PhotoForge 0.4.0')).toBeTruthy();
    expect(view.getByText('C:\\Local\\PhotoForge\\components.json')).toBeTruthy();
  });

  it('shows registered, loaded, and unavailable names', async () => {
    invokeMock.mockResolvedValueOnce(diagnostics);
    const view = render(DiagnosticsSettings);
    expect(await view.findByText('Rule Planner · Ollama Planner')).toBeTruthy();
    expect(view.getByText('Rule Planner · Deterministic Engine')).toBeTruthy();
    expect(view.getByText('Ollama Planner · ONNX Restoration')).toBeTruthy();
  });

  it('reports empty failure and plugin validation state', async () => {
    invokeMock.mockResolvedValueOnce(diagnostics);
    const view = render(DiagnosticsSettings);
    await view.findAllByText('None recorded');
    expect(view.getAllByText('None recorded')).toHaveLength(2);
  });

  it('does not measure component overhead automatically', async () => {
    invokeMock.mockResolvedValueOnce(diagnostics);
    const view = render(DiagnosticsSettings);
    await view.findByRole('button', { name: 'Measure' });
    expect(invokeMock).toHaveBeenCalledTimes(1);
  });

  it('runs and displays local component measurements explicitly', async () => {
    invokeMock.mockResolvedValueOnce(diagnostics).mockResolvedValueOnce({ samples: 250, registryLookupAverageNs: 420, plannerDispatchAverageNs: 12500, componentFactoryAverageNs: 2500000, note: 'No network or plugin execution occurred.' });
    const view = render(DiagnosticsSettings);
    await fireEvent.click(await view.findByRole('button', { name: 'Measure' }));
    await waitFor(() => expect(view.getByText('420 ns')).toBeTruthy());
    expect(view.getByText('12.50 µs')).toBeTruthy();
    expect(view.getByText('2.50 ms')).toBeTruthy();
    expect(invokeMock).toHaveBeenLastCalledWith('measure_component_performance', { samples: 250 });
  });

  it('surfaces diagnostics failures without crashing', async () => {
    invokeMock.mockRejectedValueOnce({ message: 'Diagnostics unavailable' });
    const view = render(DiagnosticsSettings);
    expect((await view.findByRole('alert')).textContent).toContain('Diagnostics unavailable');
  });
});
