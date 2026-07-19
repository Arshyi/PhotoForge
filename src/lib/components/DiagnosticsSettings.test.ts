import { fireEvent, render, waitFor } from '@testing-library/svelte';
import { invoke } from '@tauri-apps/api/core';
import { beforeEach, describe, expect, it, vi } from 'vitest';
import DiagnosticsSettings from './DiagnosticsSettings.svelte';

vi.mock('@tauri-apps/api/core', () => ({ invoke: vi.fn() }));
const invokeMock = vi.mocked(invoke);

const diagnostics = {
  applicationVersion: '0.5.0',
  registeredPlanners: ['Rule Planner', 'Ollama Planner'],
  registeredEngines: ['Deterministic Engine', 'ONNX Restoration'],
  loadedComponents: ['Rule Planner', 'Deterministic Engine'],
  unavailableComponents: ['ONNX Restoration'],
  initializationFailures: [], pluginValidationErrors: [],
  configurationPath: 'C:\\Local\\PhotoForge\\components.json'
};

const ollama = {
  connected: false, lastError: null, lastResponseTimeMs: null,
  connectionLatencyMs: null, generationLatencyMs: null, validationLatencyMs: null,
  rulePlannerLatencyMs: null, comparisonLatencyMs: null, modelSelected: null,
  plannerVersion: '0.5.0', validationFailures: 0, rejectedPlans: 0,
  successfulPlans: 0, cancelledPlans: 0, localClientMemoryEstimateMb: 1,
  memoryNote: 'The Ollama process is external.'
};

function mockDiagnostics() {
  invokeMock.mockImplementation((command) => {
    if (command === 'get_component_diagnostics') return Promise.resolve(diagnostics) as never;
    if (command === 'get_ollama_diagnostics') return Promise.resolve(ollama) as never;
    return Promise.resolve(ollama) as never;
  });
}

beforeEach(() => invokeMock.mockReset());

describe('DiagnosticsSettings', () => {
  it('shows app version and registry counts', async () => {
    mockDiagnostics();
    const view = render(DiagnosticsSettings);
    expect(await view.findByText('PhotoForge 0.5.0')).toBeTruthy();
    expect(view.getByText('C:\\Local\\PhotoForge\\components.json')).toBeTruthy();
  });

  it('shows registered, loaded, and unavailable names', async () => {
    mockDiagnostics();
    const view = render(DiagnosticsSettings);
    expect(await view.findByText('Rule Planner · Ollama Planner')).toBeTruthy();
    expect(view.getByText('Rule Planner · Deterministic Engine')).toBeTruthy();
    expect(view.getByText('ONNX Restoration')).toBeTruthy();
  });

  it('reports empty failure and plugin validation state', async () => {
    mockDiagnostics();
    const view = render(DiagnosticsSettings);
    await view.findAllByText('None recorded');
    expect(view.getAllByText('None recorded')).toHaveLength(3);
  });

  it('does not measure component overhead automatically', async () => {
    mockDiagnostics();
    const view = render(DiagnosticsSettings);
    await view.findByRole('button', { name: 'Measure' });
    expect(invokeMock).toHaveBeenCalledTimes(2);
  });

  it('runs and displays local component measurements explicitly', async () => {
    mockDiagnostics();
    invokeMock.mockImplementation((command) => {
      if (command === 'get_component_diagnostics') return Promise.resolve(diagnostics) as never;
      if (command === 'get_ollama_diagnostics') return Promise.resolve(ollama) as never;
      if (command === 'measure_component_performance') return Promise.resolve({ samples: 250, registryLookupAverageNs: 420, plannerDispatchAverageNs: 12500, componentFactoryAverageNs: 2500000, note: 'No network or plugin execution occurred.' }) as never;
      return Promise.resolve(ollama) as never;
    });
    const view = render(DiagnosticsSettings);
    await fireEvent.click(await view.findByRole('button', { name: 'Measure' }));
    await waitFor(() => expect(view.getByText('420 ns')).toBeTruthy());
    expect(view.getByText('12.50 µs')).toBeTruthy();
    expect(view.getByText('2.50 ms')).toBeTruthy();
    expect(invokeMock).toHaveBeenLastCalledWith('measure_component_performance', { samples: 250 });
  });

  it('surfaces diagnostics failures without crashing', async () => {
    invokeMock.mockImplementation((command) => command === 'get_component_diagnostics'
      ? Promise.reject({ message: 'Diagnostics unavailable' }) as never
      : Promise.resolve(ollama) as never);
    const view = render(DiagnosticsSettings);
    expect((await view.findByRole('alert')).textContent).toContain('Diagnostics unavailable');
  });

  it('shows disconnected Ollama state, selected model, and planner version', async () => {
    mockDiagnostics();
    const view = render(DiagnosticsSettings);
    const panel = await view.findByLabelText('Ollama diagnostics');
    expect(panel.textContent).toContain('Disconnected');
    expect(panel.textContent).toContain('Planner version');
    expect(panel.textContent).toContain('0.5.0');
  });

  it('shows Ollama latency and plan counters without telemetry', async () => {
    invokeMock.mockImplementation((command) => command === 'get_component_diagnostics'
      ? Promise.resolve(diagnostics) as never
      : Promise.resolve({ ...ollama, connected: true, modelSelected: 'gemma3:4b', connectionLatencyMs: 2.5, generationLatencyMs: 10.25, validationLatencyMs: 0.5, rulePlannerLatencyMs: 0.1, comparisonLatencyMs: 11, lastResponseTimeMs: 10.75, successfulPlans: 4, rejectedPlans: 2, validationFailures: 2, cancelledPlans: 1 }) as never);
    const view = render(DiagnosticsSettings);
    const panel = await view.findByLabelText('Ollama diagnostics');
    expect(panel.textContent).toContain('gemma3:4b');
    expect(panel.textContent).toContain('10.25 ms');
    expect(panel.textContent).toContain('Successful 4');
    expect(panel.textContent).toContain('Cancelled 1');
  });

  it('displays the last actionable Ollama error', async () => {
    invokeMock.mockImplementation((command) => command === 'get_component_diagnostics'
      ? Promise.resolve(diagnostics) as never
      : Promise.resolve({ ...ollama, lastError: 'The local Ollama request timed out.' }) as never);
    const view = render(DiagnosticsSettings);
    expect(await view.findByText('The local Ollama request timed out.')).toBeTruthy();
  });
});
