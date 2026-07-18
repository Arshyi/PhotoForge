import { fireEvent, render, waitFor, within } from '@testing-library/svelte';
import { invoke } from '@tauri-apps/api/core';
import { beforeEach, describe, expect, it, vi } from 'vitest';
import type { EditPlan } from '../types/editor';
import { defaultGuidedSettings } from '../utils/guided';
import GuidedEditPanel from './GuidedEditPanel.svelte';

vi.mock('@tauri-apps/api/core', () => ({ invoke: vi.fn() }));
const invokeMock = vi.mocked(invoke);

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
});

describe('GuidedEditPanel', () => {
  it('renders a local rule-based request surface and all suggestions', () => {
    const view = renderPanel();
    expect(view.getByLabelText('Editing request')).toBeTruthy();
    expect(view.getByText('Rule-based · local')).toBeTruthy();
    expect(view.getByText(/never edits pixels/)).toBeTruthy();
    expect(view.getByText('Suggested prompts')).toBeTruthy();
  });

  it('generates a plan with Enter and shows explainable operations', async () => {
    invokeMock.mockResolvedValueOnce({
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
    invokeMock.mockResolvedValueOnce({ plan: planned(), documentId: 7, requestId: 1, processingTimeMs: 1, isCurrent: true });
    const view = renderPanel();
    await fireEvent.input(view.getByLabelText('Editing request'), { target: { value: 'Reduce noise' } });
    await fireEvent.click(view.getByRole('button', { name: 'Generate Plan' }));
    await waitFor(() => expect(view.getByText('High · 81%')).toBeTruthy());
    expect(view.getByText('Heuristic rule-match strength, not AI certainty.')).toBeTruthy();
    expect(view.getByText('Sharpening may amplify noise.')).toBeTruthy();
  });

  it('can hide confidence and warnings through local settings', async () => {
    invokeMock.mockResolvedValueOnce({ plan: planned(), documentId: 7, requestId: 1, processingTimeMs: 1, isCurrent: true });
    const view = renderPanel({ settings: { ...defaultGuidedSettings, showWarnings: false, showConfidence: false } });
    await fireEvent.input(view.getByLabelText('Editing request'), { target: { value: 'Reduce noise' } });
    await fireEvent.click(view.getByRole('button', { name: 'Generate Plan' }));
    await waitFor(() => expect(view.getByText('Denoise')).toBeTruthy());
    expect(view.queryByText('High · 81%')).toBeNull();
    expect(view.queryByText('Sharpening may amplify noise.')).toBeNull();
  });

  it('supports deleting and reordering operations in the inspector', async () => {
    invokeMock.mockResolvedValueOnce({ plan: planned(), documentId: 7, requestId: 1, processingTimeMs: 1, isCurrent: true });
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
    invokeMock.mockResolvedValueOnce({ plan: planned(), documentId: 7, requestId: 1, processingTimeMs: 1, isCurrent: true });
    const view = renderPanel();
    await fireEvent.input(view.getByLabelText('Editing request'), { target: { value: 'Reduce noise' } });
    await fireEvent.click(view.getByRole('button', { name: 'Generate Plan' }));
    await waitFor(() => expect(view.getByRole('slider', { name: 'Denoise strength' })).toBeTruthy());
    await fireEvent.input(view.getByRole('slider', { name: 'Denoise strength' }), { target: { value: '0.6' } });
    expect(view.getByText('0.60')).toBeTruthy();
  });

  it('validates the edited plan before applying it', async () => {
    const apply = vi.fn();
    invokeMock
      .mockResolvedValueOnce({ plan: planned(), documentId: 7, requestId: 1, processingTimeMs: 1, isCurrent: true })
      .mockResolvedValueOnce(planned());
    const view = renderPanel({ onapply: apply });
    await fireEvent.input(view.getByLabelText('Editing request'), { target: { value: 'Reduce noise' } });
    await fireEvent.click(view.getByRole('button', { name: 'Generate Plan' }));
    await waitFor(() => expect(view.getByRole('button', { name: 'Apply' })).toBeTruthy());
    await fireEvent.click(view.getByRole('button', { name: 'Apply' }));
    await waitFor(() => expect(apply).toHaveBeenCalledWith(planned().operations));
    expect(invokeMock.mock.calls[1][0]).toBe('validate_guided_plan');
  });

  it('applies a reviewed plan with Ctrl+Enter', async () => {
    const apply = vi.fn();
    invokeMock
      .mockResolvedValueOnce({ plan: planned(), documentId: 7, requestId: 1, processingTimeMs: 1, isCurrent: true })
      .mockResolvedValueOnce(planned());
    const view = renderPanel({ onapply: apply });
    const input = view.getByLabelText('Editing request');
    await fireEvent.input(input, { target: { value: 'Reduce noise' } });
    await fireEvent.keyDown(input, { key: 'Enter' });
    await waitFor(() => expect(view.getByRole('button', { name: 'Apply' })).toBeTruthy());
    await fireEvent.keyDown(input, { key: 'Enter', ctrlKey: true });
    await waitFor(() => expect(apply).toHaveBeenCalledWith(planned().operations));
  });

  it('cancels a plan with Escape', async () => {
    invokeMock.mockResolvedValueOnce({ plan: planned(), documentId: 7, requestId: 1, processingTimeMs: 1, isCurrent: true });
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
    invokeMock
      .mockReturnValueOnce(first as Promise<never>)
      .mockResolvedValueOnce({ plan: { ...planned(), summary: 'Latest plan' }, documentId: 7, requestId: 2, processingTimeMs: 1, isCurrent: true });
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
});
