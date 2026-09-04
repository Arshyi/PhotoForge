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

const cache = {
  hits: 9, misses: 1, evictions: 2, refusals: 1, entries: 3,
  bytes: 1024, capacityBytes: 256 * 1024 * 1024,
  diskEntries: 0, diskBytes: 0, diskCapacityBytes: 0
};
const renderer = {
  tileSize: 256, maxThreads: 8, cache,
  gpuCompiled: true, gpuAvailable: true,
  gpu: { backend: 'Vulkan', adapter: 'Test GPU', deviceType: 'DiscreteGpu', driver: 'test 1', maxBufferBytes: 1024, healthy: true },
  gpuStats: { dispatches: 4, megapixels: 96, declined: 3, failures: 1 }, gpuNote: ''
};
const policy = {
  mode: 'auto', active: true, effectiveBackend: 'cpu', adapterStatus: 'available', hardDisabled: false,
  fallbackReason: null
};

function mockDiagnostics() {
  invokeMock.mockImplementation((command) => {
    if (command === 'get_component_diagnostics') return Promise.resolve(diagnostics) as never;
    if (command === 'get_ollama_diagnostics') return Promise.resolve(ollama) as never;
    if (command === 'render_diagnostics') return Promise.resolve(renderer) as never;
    if (command === 'get_render_backend_mode') return Promise.resolve(policy) as never;
    if (command === 'default_render_cache_budget') return Promise.resolve(256 * 1024 * 1024) as never;
    return Promise.resolve(ollama) as never;
  });
}

// Braces matter: mockReset() returns the mock, and a function returned from
// beforeEach is treated by vitest as a teardown callback, so the concise form
// makes vitest call invoke() with no arguments after every test.
beforeEach(() => {
  invokeMock.mockReset();
});

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
    expect(invokeMock).toHaveBeenCalledTimes(5);
  });

  it('runs and displays local component measurements explicitly', async () => {
    mockDiagnostics();
    invokeMock.mockImplementation((command) => {
      if (command === 'get_component_diagnostics') return Promise.resolve(diagnostics) as never;
      if (command === 'get_ollama_diagnostics') return Promise.resolve(ollama) as never;
      if (command === 'render_diagnostics') return Promise.resolve(renderer) as never;
      if (command === 'get_render_backend_mode') return Promise.resolve(policy) as never;
      if (command === 'default_render_cache_budget') return Promise.resolve(256 * 1024 * 1024) as never;
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

  it('reports the truthful tiled CPU base and bounded optional GPU work', async () => {
    mockDiagnostics();
    const view = render(DiagnosticsSettings);
    const panel = await view.findByLabelText('Render engine diagnostics');
    expect(panel.textContent).toContain('tiled CPU compositor is authoritative');
    expect(panel.textContent).toContain('eligible wide Gaussian blurs');
    expect(panel.textContent).toContain('Test GPU');
    expect(panel.textContent).toContain('Completed 4');
    expect(panel.textContent).toContain('Fallbacks 1');
    expect(panel.textContent).toContain('90.0% hit rate');
  });

  it('changes the renderer policy explicitly and refreshes diagnostics', async () => {
    mockDiagnostics();
    invokeMock.mockImplementation((command) => {
      if (command === 'get_component_diagnostics') return Promise.resolve(diagnostics) as never;
      if (command === 'get_ollama_diagnostics') return Promise.resolve(ollama) as never;
      if (command === 'render_diagnostics') return Promise.resolve(renderer) as never;
      if (command === 'get_render_backend_mode') return Promise.resolve(policy) as never;
      if (command === 'default_render_cache_budget') return Promise.resolve(256 * 1024 * 1024) as never;
      if (command === 'set_render_backend_mode') return Promise.resolve({ ...policy, mode: 'cpu' }) as never;
      if (command === undefined) {
        console.trace('unexpected invoke without command');
        return Promise.resolve(undefined) as never;
      }
      return Promise.reject(new Error(`Unexpected ${String(command)} after ${JSON.stringify(invokeMock.mock.calls)}`)) as never;
    });
    const view = render(DiagnosticsSettings);
    const select = await view.findByLabelText('Render acceleration policy');
    await fireEvent.change(select, { target: { value: 'cpu' } });
    await waitFor(() => expect(invokeMock).toHaveBeenCalledWith('set_render_backend_mode', { mode: 'cpu' }));
    expect((select as HTMLSelectElement).value).toBe('cpu');
    expect(invokeMock.mock.calls.filter(([command]) => command === 'render_diagnostics')).toHaveLength(2);
  });

  it('keeps unsupported GPU preference disabled in a CPU-only build', async () => {
    mockDiagnostics();
    invokeMock.mockImplementation((command) => {
      if (command === 'render_diagnostics') return Promise.resolve({ ...renderer, gpuCompiled: false, gpuAvailable: false, gpu: null, gpuStats: null, gpuNote: 'This build was compiled without GPU support.' }) as never;
      if (command === 'get_render_backend_mode') return Promise.resolve({ ...policy, active: false, adapterStatus: 'not_probed' }) as never;
      if (command === 'default_render_cache_budget') return Promise.resolve(256 * 1024 * 1024) as never;
      if (command === 'get_component_diagnostics') return Promise.resolve(diagnostics) as never;
      return Promise.resolve(ollama) as never;
    });
    const view = render(DiagnosticsSettings);
    expect((await view.findByRole('option', { name: 'Prefer GPU for eligible blur' }) as HTMLOptionElement).disabled).toBe(true);
    expect(view.getByText('This build was compiled without GPU support.')).toBeTruthy();
  });

  it('sets and clears the render cache without enabling a disk tier by default', async () => {
    mockDiagnostics();
    invokeMock.mockImplementation((command, args) => {
      if (command === 'get_component_diagnostics') return Promise.resolve(diagnostics) as never;
      if (command === 'get_ollama_diagnostics') return Promise.resolve(ollama) as never;
      if (command === 'render_diagnostics') return Promise.resolve(renderer) as never;
      if (command === 'get_render_backend_mode') return Promise.resolve(policy) as never;
      if (command === 'default_render_cache_budget') return Promise.resolve(256 * 1024 * 1024) as never;
      if (command === 'set_render_cache_budget') return Promise.resolve({ ...cache, capacityBytes: (args as { bytes: number }).bytes }) as never;
      if (command === 'clear_render_cache') return Promise.resolve({ ...cache, hits: 0, misses: 0, entries: 0, bytes: 0 }) as never;
      return Promise.reject(new Error(`Unexpected ${String(command)} after ${JSON.stringify(invokeMock.mock.calls)}`)) as never;
      return Promise.reject(new Error('Unexpected ' + String(command))) as never;
    });
    const view = render(DiagnosticsSettings);
    const budget = await view.findByLabelText('Render cache budget (MiB)');
    await fireEvent.input(budget, { target: { value: '128' } });
    await fireEvent.click(view.getByRole('button', { name: 'Apply cache budget' }));
    await waitFor(() => expect(invokeMock).toHaveBeenCalledWith('set_render_cache_budget', { bytes: 128 * 1024 * 1024 }));
    await fireEvent.click(view.getByRole('button', { name: 'Clear render cache' }));
    await waitFor(() => expect(view.getByText(/0 entries/)).toBeTruthy());
    expect(invokeMock).toHaveBeenCalledWith('clear_render_cache');
    expect(view.getByText(/disk tier is disabled by default/)).toBeTruthy();
  });

  it('reports an explicitly enabled bounded disk tier', async () => {
    mockDiagnostics();
    const diskRenderer = {
      ...renderer,
      cache: { ...cache, diskEntries: 2, diskBytes: 4096, diskCapacityBytes: 1024 * 1024 }
    };
    invokeMock.mockImplementation((command) => {
      if (command === 'get_component_diagnostics') return Promise.resolve(diagnostics) as never;
      if (command === 'get_ollama_diagnostics') return Promise.resolve(ollama) as never;
      if (command === 'render_diagnostics') return Promise.resolve(diskRenderer) as never;
      if (command === 'get_render_backend_mode') return Promise.resolve(policy) as never;
      if (command === 'default_render_cache_budget') return Promise.resolve(256 * 1024 * 1024) as never;
      return Promise.resolve(ollama) as never;
    });
    const view = render(DiagnosticsSettings);
    const panel = await view.findByLabelText('Render tile cache');
    expect(panel.textContent).toContain('Optional disk tier');
    expect(panel.textContent).toContain('2 entries');
    expect(panel.textContent).toContain('checksummed');
  });
});
