import { fireEvent, render, screen, waitFor } from '@testing-library/svelte';
import { invoke } from '@tauri-apps/api/core';
import { beforeEach, describe, expect, it, vi } from 'vitest';
import TextPanel from './TextPanel.svelte';
import type { TextContent } from '../layers/types';

vi.mock('@tauri-apps/api/core', () => ({ invoke: vi.fn() }));
const invokeMock = vi.mocked(invoke);

// Braces matter: mockReset() returns the mock, and a value returned from
// beforeEach is treated by vitest as a teardown callback.
beforeEach(() => {
  invokeMock.mockReset();
});

const families = ['Arial', 'Georgia', 'Segoe UI'];

function content(overrides: Partial<TextContent> = {}): TextContent {
  return {
    text: 'Hello',
    fontFamily: 'Georgia',
    fontSize: 48,
    fontWeight: 400,
    italic: false,
    align: 'start',
    lineHeight: 1.2,
    letterSpacing: 0,
    originX: 20,
    originY: 60,
    fill: { red: 0, green: 0, blue: 0, alpha: 1 },
    ...overrides
  };
}

describe('TextPanel', () => {
  it('lists the fonts installed on this machine', async () => {
    invokeMock.mockResolvedValue({ families });
    render(TextPanel, { content: content(), layerId: 'l1' });

    await waitFor(() => expect(invokeMock).toHaveBeenCalledWith('list_system_fonts'));
    const select = (await screen.findByLabelText('Font family')) as HTMLSelectElement;
    await waitFor(() => expect(select.options.length).toBe(4));
    const options = Array.from(select.options).map((option) => option.value);
    expect(options).toEqual(['', 'Arial', 'Georgia', 'Segoe UI']);
  });

  it('reports a missing font without rewriting the request', async () => {
    invokeMock.mockResolvedValue({ families });
    const changes: TextContent[] = [];
    render(TextPanel, {
      content: content({ fontFamily: 'Some Absent Face' }),
      layerId: 'l1',
      onchange: (_id: string, next: TextContent) => changes.push(next)
    });

    const warning = await screen.findByTestId('missing-font');
    expect(warning.textContent).toContain('Some Absent Face');
    // The control still shows the requested face rather than the substitute,
    // and nothing was changed just by noticing it was missing.
    const select = (await screen.findByLabelText('Font family')) as HTMLSelectElement;
    expect(select.value).toBe('Some Absent Face');
    expect(changes).toEqual([]);
  });

  it('says nothing about a font that is installed', async () => {
    invokeMock.mockResolvedValue({ families });
    render(TextPanel, { content: content({ fontFamily: 'Arial' }), layerId: 'l1' });
    await waitFor(() => expect(invokeMock).toHaveBeenCalled());
    expect(screen.queryByTestId('missing-font')).toBeNull();
  });

  it('raises the edited content rather than mutating in place', async () => {
    invokeMock.mockResolvedValue({ families });
    const original = content();
    const changes: { id: string; next: TextContent }[] = [];
    render(TextPanel, {
      content: original,
      layerId: 'l7',
      onchange: (id: string, next: TextContent) => changes.push({ id, next })
    });

    const textarea = await screen.findByLabelText('Text content');
    await fireEvent.input(textarea, { target: { value: 'Rewritten' } });

    expect(changes).toHaveLength(1);
    expect(changes[0].id).toBe('l7');
    expect(changes[0].next.text).toBe('Rewritten');
    // Everything else survives the edit.
    expect(changes[0].next.fontSize).toBe(48);
    expect(changes[0].next.fontFamily).toBe('Georgia');
    // And the original object is untouched, so undo has something to go back to.
    expect(original.text).toBe('Hello');
  });

  it('keeps a non-numeric size rather than writing NaN into the document', async () => {
    invokeMock.mockResolvedValue({ families });
    const changes: TextContent[] = [];
    render(TextPanel, {
      content: content(),
      layerId: 'l1',
      onchange: (_id: string, next: TextContent) => changes.push(next)
    });

    const size = await screen.findByLabelText('Font size');
    await fireEvent.change(size, { target: { value: 'not a number' } });
    expect(changes.at(-1)?.fontSize).toBe(48);
  });

  it('rasterizes only when asked', async () => {
    invokeMock.mockResolvedValue({ families });
    const asked: string[] = [];
    render(TextPanel, {
      content: content(),
      layerId: 'l9',
      onrasterize: (id: string) => asked.push(id)
    });

    await waitFor(() => expect(invokeMock).toHaveBeenCalled());
    expect(asked).toEqual([]);
    await fireEvent.click(screen.getByText('Rasterize to pixels'));
    expect(asked).toEqual(['l9']);
  });

  it('survives font discovery failing', async () => {
    invokeMock.mockRejectedValue(new Error('no font service'));
    render(TextPanel, { content: content(), layerId: 'l1' });

    const alert = await screen.findByRole('alert');
    expect(alert.textContent).toContain('no font service');
    // The rest of the panel still works: a missing font list is not a reason to
    // stop editing the words.
    expect(await screen.findByLabelText('Text content')).toBeTruthy();
  });

  it('shows nothing to edit when the selection is not a text layer', () => {
    invokeMock.mockResolvedValue({ families });
    render(TextPanel, { content: null, layerId: '' });
    expect(screen.getByText(/Select a text layer/)).toBeTruthy();
  });
});
