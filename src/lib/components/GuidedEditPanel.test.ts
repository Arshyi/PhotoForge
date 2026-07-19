import { fireEvent, render, waitFor, within } from '@testing-library/svelte';
import { invoke } from '@tauri-apps/api/core';
import { beforeEach, describe, expect, it, vi } from 'vitest';
import type { EditPlan } from '../types/editor';
import { defaultGuidedSettings } from '../utils/guided';
import GuidedEditPanel from './GuidedEditPanel.svelte';

vi.mock('@tauri-apps/api/core', () => ({ invoke: vi.fn() }));
const invokeMock = vi.mocked(invoke);
let commandResults = new Map<string, Array<Promise<unknown>>>();

let componentState: {
  configuration: { ollamaSelectedModel: string | null; ollamaMaxOperations: number };
} = {
  configuration: { ollamaSelectedModel: null, ollamaMaxOperations: 8 }
};
let ollamaState = { connected: false };

function enqueue(command: string, value: unknown, reject = false) {
  const results = commandResults.get(command) ?? [];
  results.push(reject ? Promise.reject(value) : Promise.resolve(value));
  commandResults.set(command, results);
}

function enqueuePromise(command: string, value: Promise<unknown>) {
  const results = commandResults.get(command) ?? [];
  results.push(value);
  commandResults.set(command, results);
}

function planned(): EditPlan {
  return {
    summary: 'Reduce noise before applying restrained edge clarity.',
    confidence: 0.81,
    warnings: ['Sharpening may amplify noise.'],
    operations: [
      { type: 'denoise', strength: 0.3, preserve_edges: 0.84 },
      { type: 'edge_aware_sharpen', strength: 0.25, radius: 1, threshold: 0.04 }
    ],
    operationExplanations: ['Reduce small pixel variation.', 'Improve captured edges.']
  };
}

function report(valid = true) {
  return {
    valid,
    originalResponse: '{"summary":"local"}',
    validatedResponse: valid ? '{"summary":"validated"}' : null,
    rejectedFields: valid ? [] : ['operations[0].path'],
    errors: valid ? [] : ['unknown fields are not allowed'],
    validationTimeMs: 0.12
  };
}

function enableOllama() {
  componentState = {
    configuration: { ollamaSelectedModel: 'gemma3:4b', ollamaMaxOperations: 8 }
  };
  ollamaState = { connected: true };
}

function renderPanel(overrides: Record<string, unknown> = {}) {
  return render(GuidedEditPanel, {
    props: {
      documentId: 7,
      ready: true,
      disabled: false,
      settings: defaultGuidedSettings,
      onapply: vi.fn(),
      onmessage: vi.fn(),
      ...overrides
    }
  });
}

beforeEach(() => {
  localStorage.clear();
  invokeMock.mockReset();
  commandResults = new Map();
  componentState = { configuration: { ollamaSelectedModel: null, ollamaMaxOperations: 8 } };
  ollamaState = { connected: false };
  invokeMock.mockImplementation((command) => {
    if (command === 'get_component_snapshot') return Promise.resolve(componentState) as never;
    if (command === 'get_ollama_diagnostics') return Promise.resolve(ollamaState) as never;
    const next = commandResults.get(command)?.shift();
    return (next ?? Promise.reject(new Error(`Unexpected command: ${command}`))) as never;
  });
});

describe('GuidedEditPanel', () => {
  it('renders a local rule-based request surface and all suggestions', () => {
    const view = renderPanel();
    expect(view.getByLabelText('Editing request')).toBeTruthy();
    expect(view.getByText('Rule · on-device')).toBeTruthy();
    expect(view.getByText(/deterministic engine can modify pixels/)).toBeTruthy();
    expect(view.getByText('Suggested prompts')).toBeTruthy();
  });

  it('generates a plan with Enter and shows explainable operations', async () => {
    enqueue('generate_edit_plan', {
      plan: planned(),
      documentId: 7,
      requestId: 1,
      processingTimeMs: 0.42,
      isCurrent: true
    });
    const view = renderPanel();
    const input = view.getByLabelText('Editing request');
    await fireEvent.input(input, { target: { value: 'Reduce noise and sharpen slightly' } });
    await fireEvent.keyDown(input, { key: 'Enter' });
    await waitFor(() => expect(view.getByText(/Reduce noise before/)).toBeTruthy());
    expect(invokeMock).toHaveBeenCalledWith('generate_edit_plan', {
      request: 'Reduce noise and sharpen slightly',
      documentId: 7,
      requestId: 1
    });
    expect(view.getByText('Denoise')).toBeTruthy();
    expect(view.getByText('Improve captured edges.')).toBeTruthy();
  });

  it('shows heuristic confidence and warnings when enabled', async () => {
    enqueue('generate_edit_plan', { plan: planned(), documentId: 7, requestId: 1, processingTimeMs: 1, isCurrent: true });
    const view = renderPanel();
    await fireEvent.input(view.getByLabelText('Editing request'), { target: { value: 'Reduce noise' } });
    await fireEvent.click(view.getByRole('button', { name: 'Generate Plan' }));
    await waitFor(() => expect(view.getByText('High · 81%')).toBeTruthy());
    expect(view.getByText(/never permission to apply automatically/)).toBeTruthy();
    expect(view.getByText('Sharpening may amplify noise.')).toBeTruthy();
  });

  it('can hide confidence and warnings through local settings', async () => {
    enqueue('generate_edit_plan', { plan: planned(), documentId: 7, requestId: 1, processingTimeMs: 1, isCurrent: true });
    const view = renderPanel({ settings: { ...defaultGuidedSettings, showWarnings: false, showConfidence: false } });
    await fireEvent.input(view.getByLabelText('Editing request'), { target: { value: 'Reduce noise' } });
    await fireEvent.click(view.getByRole('button', { name: 'Generate Plan' }));
    await waitFor(() => expect(view.getByText('Denoise')).toBeTruthy());
    expect(view.queryByText('High · 81%')).toBeNull();
    expect(view.queryByText('Sharpening may amplify noise.')).toBeNull();
  });

  it('supports deleting and reordering operations in the inspector', async () => {
    enqueue('generate_edit_plan', { plan: planned(), documentId: 7, requestId: 1, processingTimeMs: 1, isCurrent: true });
    const view = renderPanel();
    await fireEvent.input(view.getByLabelText('Editing request'), { target: { value: 'Reduce noise' } });
    await fireEvent.click(view.getByRole('button', { name: 'Generate Plan' }));
    await waitFor(() => expect(view.getByLabelText('Move Edge-Aware Sharpen up')).toBeTruthy());
    await fireEvent.click(view.getByLabelText('Move Edge-Aware Sharpen up'));
    expect(within(view.getByRole('list', { name: 'Planned operations' })).getAllByRole('listitem')[0].textContent).toContain('Edge-Aware Sharpen');
    await fireEvent.click(view.getByLabelText('Delete Denoise'));
    expect(view.queryByText('Reduce small pixel variation.')).toBeNull();
  });

  it('lets users adjust operation strength before validation', async () => {
    enqueue('generate_edit_plan', { plan: planned(), documentId: 7, requestId: 1, processingTimeMs: 1, isCurrent: true });
    const view = renderPanel();
    await fireEvent.input(view.getByLabelText('Editing request'), { target: { value: 'Reduce noise' } });
    await fireEvent.click(view.getByRole('button', { name: 'Generate Plan' }));
    await waitFor(() => expect(view.getByRole('slider', { name: 'Denoise strength' })).toBeTruthy());
    await fireEvent.input(view.getByRole('slider', { name: 'Denoise strength' }), { target: { value: '0.6' } });
    expect(view.getByText('0.60')).toBeTruthy();
  });

  it('validates the edited plan before applying it', async () => {
    const apply = vi.fn();
    enqueue('generate_edit_plan', { plan: planned(), documentId: 7, requestId: 1, processingTimeMs: 1, isCurrent: true });
    enqueue('validate_guided_plan', planned());
    const view = renderPanel({ onapply: apply });
    await fireEvent.input(view.getByLabelText('Editing request'), { target: { value: 'Reduce noise' } });
    await fireEvent.click(view.getByRole('button', { name: 'Generate Plan' }));
    await waitFor(() => expect(view.getByRole('button', { name: 'Apply' })).toBeTruthy());
    await fireEvent.click(view.getByRole('button', { name: 'Apply' }));
    await waitFor(() => expect(apply).toHaveBeenCalledWith(planned().operations));
    expect(invokeMock.mock.calls.some(([command]) => command === 'validate_guided_plan')).toBe(true);
  });

  it('applies a reviewed plan with Ctrl+Enter', async () => {
    const apply = vi.fn();
    enqueue('generate_edit_plan', { plan: planned(), documentId: 7, requestId: 1, processingTimeMs: 1, isCurrent: true });
    enqueue('validate_guided_plan', planned());
    const view = renderPanel({ onapply: apply });
    const input = view.getByLabelText('Editing request');
    await fireEvent.input(input, { target: { value: 'Reduce noise' } });
    await fireEvent.keyDown(input, { key: 'Enter' });
    await waitFor(() => expect(view.getByRole('button', { name: 'Apply' })).toBeTruthy());
    await fireEvent.keyDown(input, { key: 'Enter', ctrlKey: true });
    await waitFor(() => expect(apply).toHaveBeenCalledWith(planned().operations));
  });

  it('cancels a plan with Escape', async () => {
    enqueue('generate_edit_plan', { plan: planned(), documentId: 7, requestId: 1, processingTimeMs: 1, isCurrent: true });
    const view = renderPanel();
    const input = view.getByLabelText('Editing request');
    await fireEvent.input(input, { target: { value: 'Reduce noise' } });
    await fireEvent.keyDown(input, { key: 'Enter' });
    await waitFor(() => expect(view.getByText('Denoise')).toBeTruthy());
    await fireEvent.keyDown(input, { key: 'Escape' });
    expect(view.queryByLabelText('Guided edit plan')).toBeNull();
  });

  it('keeps only the latest rapid plan result', async () => {
    let resolveFirst: (value: unknown) => void = () => undefined;
    const first = new Promise((resolve) => { resolveFirst = resolve; });
    enqueuePromise('generate_edit_plan', first);
    enqueue('generate_edit_plan', { plan: { ...planned(), summary: 'Latest plan' }, documentId: 7, requestId: 3, processingTimeMs: 1, isCurrent: true });
    const view = renderPanel();
    const input = view.getByLabelText('Editing request');
    await fireEvent.input(input, { target: { value: 'Reduce noise' } });
    await fireEvent.keyDown(input, { key: 'Enter' });
    await fireEvent.input(input, { target: { value: 'Sharpen slightly' } });
    await fireEvent.keyDown(input, { key: 'Enter' });
    await waitFor(() => expect(view.getByText('Latest plan')).toBeTruthy());
    resolveFirst({ plan: planned(), documentId: 7, requestId: 1, processingTimeMs: 1, isCurrent: true });
    expect(view.queryByText(/Reduce noise before/)).toBeNull();
  });

  it('keeps Ollama disabled until a model is configured', async () => {
    const view = renderPanel();
    const option = within(view.getByLabelText('Planner')).getByRole('option', { name: 'Ollama Planner' });
    expect((option as HTMLOptionElement).disabled).toBe(true);
    expect((view.getByRole('button', { name: 'Compare Planners' }) as HTMLButtonElement).disabled).toBe(true);
  });

  it('enables Ollama and identifies the selected local model', async () => {
    enableOllama();
    const view = renderPanel();
    await waitFor(() => expect(view.getByText('gemma3:4b')).toBeTruthy());
    const option = within(view.getByLabelText('Planner')).getByRole('option', { name: 'Ollama Planner' });
    expect((option as HTMLOptionElement).disabled).toBe(false);
  });

  it('refreshes Ollama availability after component settings close', async () => {
    const view = renderPanel({ configurationRevision: 0 });
    const option = within(view.getByLabelText('Planner')).getByRole('option', {
      name: 'Ollama Planner'
    });
    expect((option as HTMLOptionElement).disabled).toBe(true);
    enableOllama();
    await view.rerender({ configurationRevision: 1 });
    await waitFor(() => expect((option as HTMLOptionElement).disabled).toBe(false));
    expect(view.getByText('gemma3:4b')).toBeTruthy();
  });

  it('tests the connection only after an explicit click', async () => {
    enqueue('test_ollama_connection', { connected: true, message: 'Connected to local Ollama 0.11.0.', version: '0.11.0', responseTimeMs: 2.5 });
    const view = renderPanel();
    expect(invokeMock.mock.calls.some(([command]) => command === 'test_ollama_connection')).toBe(false);
    await fireEvent.click(view.getByRole('button', { name: 'Test Connection' }));
    await waitFor(() => expect(view.getByText(/Connected to local Ollama 0.11.0/)).toBeTruthy());
  });

  it('refreshes installed models without selecting or downloading one', async () => {
    enqueue('refresh_ollama_models', { models: [], message: 'No compatible local models found.', responseTimeMs: 1 });
    const view = renderPanel();
    await fireEvent.click(view.getByRole('button', { name: 'Refresh Models' }));
    await waitFor(() => expect(view.getByText('No compatible local models found.')).toBeTruthy());
    expect(invokeMock.mock.calls.some(([command]) => command === 'refresh_ollama_models')).toBe(true);
  });

  it('generates a validated Ollama plan and exposes inspection controls', async () => {
    enableOllama();
    enqueue('generate_ollama_plan', { plan: planned(), documentId: 7, requestId: 1, model: 'gemma3:4b', generationTimeMs: 3, validationTimeMs: 0.12, totalTimeMs: 3.12, isCurrent: true, error: null, validationReport: report() });
    const view = renderPanel();
    await waitFor(() => expect(view.getByText('gemma3:4b')).toBeTruthy());
    await fireEvent.change(view.getByLabelText('Planner'), { target: { value: 'ollama' } });
    await fireEvent.input(view.getByLabelText('Editing request'), { target: { value: 'Reduce noise' } });
    await fireEvent.click(view.getByRole('button', { name: 'Generate Plan' }));
    await waitFor(() => expect(view.getByText('Validated Ollama plan')).toBeTruthy());
    expect(view.getByRole('button', { name: 'View Raw JSON' })).toBeTruthy();
    expect(view.getByRole('button', { name: 'Validate JSON' })).toBeTruthy();
    expect(view.getByRole('button', { name: 'View Validation Report' })).toBeTruthy();
  });

  it('shows rejected plans and offers an explicit Rule Planner fallback', async () => {
    enableOllama();
    enqueue('generate_ollama_plan', { plan: null, documentId: 7, requestId: 1, model: 'gemma3:4b', generationTimeMs: 3, validationTimeMs: 0.12, totalTimeMs: 3.12, isCurrent: true, error: 'Ollama plan validation failed.', validationReport: report(false) });
    const view = renderPanel();
    await waitFor(() => expect(view.getByText('gemma3:4b')).toBeTruthy());
    await fireEvent.change(view.getByLabelText('Planner'), { target: { value: 'ollama' } });
    await fireEvent.input(view.getByLabelText('Editing request'), { target: { value: 'Reduce noise' } });
    await fireEvent.click(view.getByRole('button', { name: 'Generate Plan' }));
    await waitFor(() => expect(view.getByRole('button', { name: 'Use Rule Planner Instead' })).toBeTruthy());
    expect(view.getByLabelText('Validation report').textContent).toContain('operations[0].path');
  });

  it('renders original Ollama JSON as read-only', async () => {
    enableOllama();
    enqueue('generate_ollama_plan', { plan: planned(), documentId: 7, requestId: 1, model: 'gemma3:4b', generationTimeMs: 3, validationTimeMs: 0.12, totalTimeMs: 3.12, isCurrent: true, error: null, validationReport: report() });
    const view = renderPanel();
    await waitFor(() => expect(view.getByText('gemma3:4b')).toBeTruthy());
    await fireEvent.change(view.getByLabelText('Planner'), { target: { value: 'ollama' } });
    await fireEvent.input(view.getByLabelText('Editing request'), { target: { value: 'Reduce noise' } });
    await fireEvent.click(view.getByRole('button', { name: 'Generate Plan' }));
    await fireEvent.click(await view.findByRole('button', { name: 'View Raw JSON' }));
    expect((view.getByLabelText('Original Ollama response') as HTMLTextAreaElement).readOnly).toBe(true);
  });

  it('revalidates raw JSON through the backend', async () => {
    enableOllama();
    enqueue('generate_ollama_plan', { plan: planned(), documentId: 7, requestId: 1, model: 'gemma3:4b', generationTimeMs: 3, validationTimeMs: 0.12, totalTimeMs: 3.12, isCurrent: true, error: null, validationReport: report() });
    enqueue('validate_ollama_json', report());
    const view = renderPanel();
    await waitFor(() => expect(view.getByText('gemma3:4b')).toBeTruthy());
    await fireEvent.change(view.getByLabelText('Planner'), { target: { value: 'ollama' } });
    await fireEvent.input(view.getByLabelText('Editing request'), { target: { value: 'Reduce noise' } });
    await fireEvent.click(view.getByRole('button', { name: 'Generate Plan' }));
    await fireEvent.click(await view.findByRole('button', { name: 'Validate JSON' }));
    await waitFor(() => expect(invokeMock.mock.calls.some(([command]) => command === 'validate_ollama_json')).toBe(true));
  });

  it('shows Rule and Ollama plans side by side without choosing a winner', async () => {
    enableOllama();
    enqueue('compare_planners', { rule: { provider: 'Rule', plan: planned(), executionTimeMs: 0.2, error: null }, ollama: { provider: 'Ollama', plan: { ...planned(), summary: 'Ollama summary' }, executionTimeMs: 4.2, error: null }, validationReport: report(), totalTimeMs: 4.4 });
    const view = renderPanel();
    await waitFor(() => expect(view.getByText('gemma3:4b')).toBeTruthy());
    await fireEvent.input(view.getByLabelText('Editing request'), { target: { value: 'Reduce noise' } });
    await fireEvent.click(view.getByRole('button', { name: 'Compare Planners' }));
    const comparison = await view.findByLabelText('Planner comparison');
    expect(comparison.textContent).toContain('no automatic winner');
    expect(comparison.textContent).toContain('Ollama summary');
  });

  it('cancels a running Ollama request when the prompt changes', async () => {
    enableOllama();
    enqueuePromise('generate_ollama_plan', new Promise(() => undefined));
    const view = renderPanel();
    await waitFor(() => expect(view.getByText('gemma3:4b')).toBeTruthy());
    await fireEvent.change(view.getByLabelText('Planner'), { target: { value: 'ollama' } });
    const input = view.getByLabelText('Editing request');
    await fireEvent.input(input, { target: { value: 'Reduce noise' } });
    await fireEvent.click(view.getByRole('button', { name: 'Generate Plan' }));
    await view.findByRole('button', { name: 'Cancel' });
    await fireEvent.input(input, { target: { value: 'Sharpen instead' } });
    await waitFor(() => expect(invokeMock.mock.calls.some(([command]) => command === 'cancel_ollama_plan')).toBe(true));
  });

  it('tags local prompt history entries with their provider', async () => {
    localStorage.setItem('photoforge.guided-recent.v1', JSON.stringify([
      { prompt: 'From rules', provider: 'Rule' },
      { prompt: 'From Ollama', provider: 'Ollama' }
    ]));
    const view = renderPanel();
    expect(await view.findByText('From rules')).toBeTruthy();
    expect(view.getByText('From Ollama')).toBeTruthy();
    expect(view.getAllByText(/Rule|Ollama/).length).toBeGreaterThan(1);
  });
});
