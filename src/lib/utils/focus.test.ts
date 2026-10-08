import { afterEach, describe, expect, it } from 'vitest';
import { rememberFocus } from './focus';

afterEach(() => {
  document.body.innerHTML = '';
});

const flush = () => new Promise<void>((resolve) => queueMicrotask(resolve));

function page() {
  document.body.innerHTML = `
    <header><button title="Command palette (Ctrl+Shift+P)" id="commands">Commands</button></header>
    <main><button id="tool">Tool</button><input id="field" /></main>`;
  return {
    commands: document.getElementById('commands') as HTMLElement,
    tool: document.getElementById('tool') as HTMLElement,
    field: document.getElementById('field') as HTMLElement
  };
}

describe('giving focus back', () => {
  it('returns focus to the element that had it when the dialog was created', async () => {
    const { tool, field } = page();
    tool.focus();
    const restore = rememberFocus();
    field.focus();
    restore();
    await flush();
    expect(document.activeElement).toBe(tool);
  });

  it('falls back to the Commands button when that element has gone, as after the palette closes', async () => {
    const { commands, tool } = page();
    tool.focus();
    const restore = rememberFocus();
    tool.remove();
    restore();
    await flush();
    expect(document.activeElement).toBe(commands);
  });

  it('uses the fallback when nothing had focus', async () => {
    const { commands } = page();
    (document.activeElement as HTMLElement | null)?.blur?.();
    const restore = rememberFocus();
    restore();
    await flush();
    expect(document.activeElement).toBe(commands);
  });

  it('does nothing, without error, when there is nowhere to go', async () => {
    document.body.innerHTML = '<p>nothing focusable</p>';
    const restore = rememberFocus();
    expect(() => restore()).not.toThrow();
    await flush();
    expect(document.activeElement).toBe(document.body);
  });

  it('waits until the update that removes the dialog has finished before moving focus', async () => {
    const { tool } = page();
    tool.focus();
    const restore = rememberFocus();
    restore();
    // Not yet: a page that is still inert for the dialog's sake would drop the focus.
    expect(document.activeElement).toBe(tool);
    document.getElementById('field')?.focus();
    expect(document.activeElement?.id).toBe('field');
    await flush();
    expect(document.activeElement).toBe(tool);
  });
});
