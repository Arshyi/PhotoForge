import { render, screen } from '@testing-library/svelte';
import { afterEach, describe, expect, it, vi } from 'vitest';
import AutomationDialog from './AutomationDialog.svelte';
import PluginCommandDialog from './PluginCommandDialog.svelte';
import PluginFilterDialog from './PluginFilterDialog.svelte';
import PluginManager from './PluginManager.svelte';
import { inspectorManifest, manifestFixture, passingTest, pluginFixture, statusFixture } from '../plugins/testing';

vi.mock('@tauri-apps/plugin-dialog', () => ({ open: vi.fn() }));

/**
 * Every dialog that takes the keyboard when it opens gives it back when it closes:
 * a person who opened it with the keyboard is where they were, not at the top of the page.
 */
afterEach(() => {
  document.body.innerHTML = '';
});

const flush = () => new Promise<void>((resolve) => queueMicrotask(resolve));

function opener() {
  const button = document.createElement('button');
  button.textContent = 'Open the dialog';
  document.body.appendChild(button);
  button.focus();
  expect(document.activeElement).toBe(button);
  return button;
}

describe('a dialog gives focus back', () => {
  it('the automation editor', async () => {
    const button = opener();
    const view = render(AutomationDialog, { props: { specs: [], macros: [] } });
    // It took the keyboard...
    expect(document.activeElement).not.toBe(button);
    view.unmount();
    await flush();
    // ...and returned it.
    expect(document.activeElement).toBe(button);
  });

  it('the plugin filter dialog', async () => {
    const button = opener();
    const view = render(PluginFilterDialog, { props: { plugins: [], layer: null } });
    expect(document.activeElement).not.toBe(button);
    view.unmount();
    await flush();
    expect(document.activeElement).toBe(button);
  });

  it('the plugin command dialog', async () => {
    const button = opener();
    const plugin = pluginFixture(inspectorManifest());
    const command = plugin.manifest!.commands![0];
    const view = render(PluginCommandDialog, { props: { plugin, command, operations: [], remembered: {} } });
    expect(document.activeElement).not.toBe(button);
    view.unmount();
    await flush();
    expect(document.activeElement).toBe(button);
  });

  it('the plugin manager', async () => {
    const button = opener();
    const api = {
      listPlugins: vi.fn(async () => statusFixture([pluginFixture(manifestFixture())])),
      inspectPluginPackage: vi.fn(),
      installPluginPackage: vi.fn(),
      setPluginEnabled: vi.fn(),
      setPluginGrants: vi.fn(),
      removePlugin: vi.fn(),
      removePluginVersion: vi.fn(),
      testPlugin: vi.fn(async () => passingTest())
    };
    const view = render(PluginManager, { props: { api, onmessage: vi.fn(), onchange: vi.fn(), onclose: vi.fn() } });
    await screen.findByRole('heading', { name: 'Plugins' });
    expect(document.activeElement).not.toBe(button);
    view.unmount();
    await flush();
    expect(document.activeElement).toBe(button);
  });
});
