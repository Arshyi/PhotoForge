import { fireEvent, render, screen, waitFor } from '@testing-library/svelte';
import { describe, expect, it, vi } from 'vitest';
import CommandPalette from './CommandPalette.svelte';
import type { PaletteCommand } from '../commands/registry';

function command(id: string, title: string, extra: Partial<PaletteCommand> = {}): PaletteCommand {
  return {
    id, title, group: 'File', source: id.startsWith('plugin:') ? 'plugin' : 'core',
    unavailable: () => null, run: vi.fn(), ...extra
  };
}

function mount(commands: PaletteCommand[]) {
  const onclose = vi.fn();
  render(CommandPalette, { commands, onclose });
  return { onclose, input: screen.getByRole('combobox', { name: 'Search commands' }) as HTMLInputElement };
}

const options = () => screen.queryAllByRole('option');

describe('the command palette', () => {
  const open = command('core.open_image', 'Open image…', { shortcut: 'Ctrl+O', description: 'Choose a photograph' });
  const save = command('core.save_project', 'Save project');
  const merge = command('core.merge_down', 'Merge down', { group: 'Layer', unavailable: () => 'Select a layer first.' });

  it('takes focus, lists everything, and is an accessible combobox over a listbox', () => {
    const { input } = mount([open, save, merge]);
    expect(document.activeElement).toBe(input);
    expect(screen.getByRole('listbox', { name: 'Commands' })).toBeTruthy();
    expect(options().map((option) => option.textContent?.replace(/\s+/g, ' ').trim())).toEqual([
      expect.stringContaining('Open image…'), expect.stringContaining('Save project'), expect.stringContaining('Merge down')
    ]);
    expect(input.getAttribute('aria-controls')).toBe('palette-results');
    expect(input.getAttribute('aria-activedescendant')).toBe('palette-core.open_image');
    expect(options()[0].getAttribute('aria-selected')).toBe('true');
    expect(screen.getByRole('status').textContent).toContain('3 commands');
    // The shortcut is shown beside the command it triggers.
    expect(screen.getByText('Ctrl+O')).toBeTruthy();
  });

  it('narrows as you type and announces what is left', async () => {
    const { input } = mount([open, save, merge]);
    await fireEvent.input(input, { target: { value: 'merge' } });
    expect(options()).toHaveLength(1);
    expect(screen.getByRole('status').textContent).toContain('1 command.');
    await fireEvent.input(input, { target: { value: 'zzzz' } });
    expect(options()).toHaveLength(0);
    expect(screen.getByText(/No commands match “zzzz”/)).toBeTruthy();
    expect(screen.getByRole('status').textContent).toBe('No commands match.');
  });

  it('moves with the arrow keys, wrapping, and runs the chosen command on Enter', async () => {
    const { input, onclose } = mount([open, save]);
    await fireEvent.keyDown(input, { key: 'ArrowDown' });
    expect(options()[1].getAttribute('aria-selected')).toBe('true');
    expect(input.getAttribute('aria-activedescendant')).toBe('palette-core.save_project');
    await fireEvent.keyDown(input, { key: 'ArrowDown' });
    expect(options()[0].getAttribute('aria-selected')).toBe('true');
    await fireEvent.keyDown(input, { key: 'ArrowUp' });
    expect(options()[1].getAttribute('aria-selected')).toBe('true');
    await fireEvent.keyDown(input, { key: 'Enter' });
    expect(save.run).toHaveBeenCalledTimes(1);
    expect(open.run).not.toHaveBeenCalled();
    expect(onclose).toHaveBeenCalled();
  });

  it('jumps to the first and last with Ctrl+Home and Ctrl+End', async () => {
    const { input } = mount([open, save, merge]);
    await fireEvent.keyDown(input, { key: 'End', ctrlKey: true });
    expect(options()[2].getAttribute('aria-selected')).toBe('true');
    await fireEvent.keyDown(input, { key: 'Home', ctrlKey: true });
    expect(options()[0].getAttribute('aria-selected')).toBe('true');
  });

  it('runs a command on click', async () => {
    const { onclose } = mount([open, save]);
    await fireEvent.click(options()[1]);
    expect(save.run).toHaveBeenCalled();
    expect(onclose).toHaveBeenCalled();
  });

  it('shows why a command cannot run, and keeps the palette open when it is chosen', async () => {
    const { input, onclose } = mount([merge]);
    const row = options()[0];
    expect(row.getAttribute('aria-disabled')).toBe('true');
    expect(row.textContent).toContain('Select a layer first.');
    expect(screen.getByRole('status').textContent).toContain('unavailable: Select a layer first.');
    await fireEvent.keyDown(input, { key: 'Enter' });
    await fireEvent.click(row);
    expect(merge.run).not.toHaveBeenCalled();
    expect(onclose).not.toHaveBeenCalled();
  });

  it('shows a command\'s description only for the selected one', () => {
    mount([open, save]);
    expect(screen.getByText('Choose a photograph')).toBeTruthy();
  });

  it('closes on Escape and when the key that opened it is pressed again', async () => {
    const first = mount([open]);
    await fireEvent.keyDown(first.input, { key: 'Escape' });
    expect(first.onclose).toHaveBeenCalledTimes(1);
    await fireEvent.keyDown(first.input, { key: 'p', ctrlKey: true, shiftKey: true });
    expect(first.onclose).toHaveBeenCalledTimes(2);
  });

  it('returns focus to where it was when it closes', async () => {
    const button = document.createElement('button');
    document.body.append(button);
    button.focus();
    const { unmount } = render(CommandPalette, { commands: [open], onclose: vi.fn() });
    await waitFor(() => expect(document.activeElement).not.toBe(button));
    unmount();
    expect(document.activeElement).toBe(button);
    button.remove();
  });

  it('survives a command that throws, closing first', async () => {
    const failing = command('core.bad', 'Bad', { run: () => { throw new Error('boom'); } });
    const { input, onclose } = mount([failing]);
    await fireEvent.keyDown(input, { key: 'Enter' });
    expect(onclose).toHaveBeenCalled();
  });

  it('offers plugin commands beside built-in ones, labelled with their plugin', () => {
    mount([open, command('plugin:com.example.a:go', 'Shapes: Add shape', { group: 'Shapes' })]);
    expect(screen.getByText('Shapes: Add shape')).toBeTruthy();
    expect(screen.getByText('Shapes')).toBeTruthy();
  });
});
