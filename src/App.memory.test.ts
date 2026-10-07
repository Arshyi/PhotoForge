import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { fireEvent, render, screen, waitFor, within } from '@testing-library/svelte';
import { invoke } from '@tauri-apps/api/core';
import App from './App.svelte';
import { statusFixture } from './lib/resources/testing';
import { statusFixture as pluginStatus } from './lib/plugins/testing';

vi.mock('@tauri-apps/api/core', () => ({ invoke: vi.fn() }));
vi.mock('@tauri-apps/plugin-dialog', () => ({ open: vi.fn(), save: vi.fn() }));
vi.mock('@tauri-apps/api/webview', () => ({
  getCurrentWebview: () => ({ onDragDropEvent: async () => () => undefined })
}));
vi.mock('@tauri-apps/api/window', () => ({
  getCurrentWindow: () => ({ onCloseRequested: async () => () => undefined })
}));

beforeEach(() => {
  localStorage.clear();
  vi.mocked(invoke).mockReset().mockImplementation(async (command) => {
    switch (command) {
      case 'list_plugins':
        return pluginStatus([]);
      case 'resource_status':
        return statusFixture();
      case 'document_plugin_status':
        return [];
      case 'list_recovery_snapshots':
        return { snapshots: [] };
      default:
        return {};
    }
  });
});

afterEach(() => vi.restoreAllMocks());

describe('the memory page of the settings', () => {
  it('is one of the settings pages, and shows the budget the backend reports', async () => {
    render(App);
    await fireEvent.click(screen.getByRole('button', { name: 'Settings' }));
    const dialog = await screen.findByRole('dialog');
    const pages = within(dialog).getByRole('navigation', { name: 'Settings pages' });
    await fireEvent.click(within(pages).getByRole('button', { name: 'Memory' }));
    expect(await within(dialog).findByRole('heading', { name: 'Memory' })).toBeTruthy();
    await waitFor(() => expect(within(dialog).getByRole('group', { name: /Memory budget: 6\.00 GB/ })).toBeTruthy());
    expect(vi.mocked(invoke)).toHaveBeenCalledWith('resource_status');
  });
});
