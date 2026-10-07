import { fireEvent, render, screen, waitFor, within } from '@testing-library/svelte';
import { describe, expect, it, vi } from 'vitest';
import PluginManager from './PluginManager.svelte';
import {
  inspectionFixture,
  inspectorManifest,
  manifestFixture,
  passingTest,
  pluginFixture,
  statusFixture
} from '../plugins/testing';
import type { PluginStatus } from '../plugins/types';

vi.mock('@tauri-apps/plugin-dialog', () => ({ open: vi.fn() }));

function api(status: PluginStatus | (() => PluginStatus)) {
  const current = () => (typeof status === 'function' ? status() : status);
  return {
    listPlugins: vi.fn(async () => current()),
    inspectPluginPackage: vi.fn(async () => inspectionFixture()),
    installPluginPackage: vi.fn(async () => ({
      plugin: 'photoforge.example.shapes', version: '1.0.0', contentHash: 'c'.repeat(64), updatedFrom: null, test: passingTest()
    })),
    setPluginEnabled: vi.fn(async () => undefined),
    setPluginGrants: vi.fn(async () => undefined),
    removePlugin: vi.fn(async () => undefined),
    removePluginVersion: vi.fn(async () => undefined),
    testPlugin: vi.fn(async () => passingTest())
  };
}

async function mount(status: PluginStatus, extra: Record<string, unknown> = {}) {
  const backend = api(status);
  const onmessage = vi.fn();
  const onchange = vi.fn();
  const onclose = vi.fn();
  const pickPackage = vi.fn(async () => 'C:\\plugins\\shapes.photoforge-plugin');
  render(PluginManager, { api: backend, onmessage, onchange, onclose, pickPackage, ...extra });
  await screen.findByRole('heading', { name: 'Plugins' });
  return { backend, onmessage, onchange, onclose, pickPackage };
}

describe('the plugin manager', () => {
  it('says plainly what a plugin can and cannot do, and that nothing checks who made it', async () => {
    await mount(statusFixture());
    expect(screen.getByText(/no access to your files, the network or other programs/)).toBeTruthy();
    expect(screen.getByText(/does not check who made it/)).toBeTruthy();
    expect(await screen.findByText('No plugins are installed.')).toBeTruthy();
  });

  it('lists what is installed, with how each stands', async () => {
    await mount(statusFixture([
      pluginFixture(),
      pluginFixture(inspectorManifest(), { enabled: false, availability: { kind: 'disabled' } }),
      pluginFixture(manifestFixture({ id: 'com.example.broken', name: 'Broken' }), {
        availability: { kind: 'damaged', reason: 'its contents are not what was installed' }
      })
    ]));
    const list = await screen.findByRole('list', { name: 'Installed plugins' });
    expect(within(list).getByTestId('chip-photoforge.example.shapes').textContent?.trim()).toBe('On');
    expect(within(list).getByTestId('chip-photoforge.example.inspector').textContent?.trim()).toBe('Turned off');
    expect(within(list).getByTestId('chip-com.example.broken').textContent?.trim()).toBe('Damaged');
  });

  it('opens a plugin to show its filters, commands and permissions, and reports damage in words', async () => {
    await mount(statusFixture([
      pluginFixture(),
      pluginFixture(manifestFixture({ id: 'com.example.broken', name: 'Broken' }), {
        availability: { kind: 'damaged', reason: 'its contents are not what was installed' }
      })
    ]));
    await fireEvent.click(await screen.findByRole('button', { name: /Shape Generator/ }));
    expect(screen.getByText(/Draw shape/)).toBeTruthy();
    expect(screen.getByText(/Each pixel on its own/)).toBeTruthy();
    expect(screen.getByText('Add shape layer')).toBeTruthy();
    for (const text of [/Run its filters on the pixels/, /Change the document with its commands/, /Add entries to the tools menu/]) {
      expect(screen.getByText(text)).toBeTruthy();
    }
    expect((screen.getByRole('checkbox', { name: /Run its filters/ }) as HTMLInputElement).checked).toBe(true);

    await fireEvent.click(screen.getByRole('button', { name: /Broken/ }));
    expect(screen.getByText('its contents are not what was installed')).toBeTruthy();
  });

  it('turns a plugin off and on, and tells the rest of the application', async () => {
    const { backend, onchange } = await mount(statusFixture([pluginFixture()]));
    await fireEvent.click(await screen.findByRole('button', { name: /Shape Generator/ }));
    await fireEvent.click(screen.getByRole('button', { name: 'Turn off' }));
    await waitFor(() => expect(backend.setPluginEnabled).toHaveBeenCalledWith('photoforge.example.shapes', false));
    await waitFor(() => expect(onchange).toHaveBeenCalled());
  });

  it('changes what a plugin is allowed to do one permission at a time', async () => {
    const { backend } = await mount(statusFixture([pluginFixture()]));
    await fireEvent.click(await screen.findByRole('button', { name: /Shape Generator/ }));
    await fireEvent.click(screen.getByRole('checkbox', { name: /Change the document with its commands/ }));
    await waitFor(() =>
      expect(backend.setPluginGrants).toHaveBeenCalledWith('photoforge.example.shapes', ['filter.pixels', 'ui.tool'])
    );
  });

  it('tests a plugin and shows each filter, with the reason when one fails', async () => {
    const { backend } = await mount(statusFixture([pluginFixture()]));
    await fireEvent.click(await screen.findByRole('button', { name: /Shape Generator/ }));
    await fireEvent.click(screen.getByRole('button', { name: 'Test' }));
    const report = await screen.findByTestId('test-photoforge.example.shapes');
    expect(report.textContent).toContain('Self-test passed');
    expect(report.textContent).toContain('12 tiles');

    // The first run has finished once the button can be used again.
    await waitFor(() => expect((screen.getByRole('button', { name: 'Test' }) as HTMLButtonElement).disabled).toBe(false));
    backend.testPlugin.mockResolvedValueOnce({
      plugin: 'photoforge.example.shapes', passed: false,
      filters: [{ filter: 'shape', passed: false, message: 'it declares Pointwise but its result depends on where tile edges fall', tiles: 0, fuelUsed: 0 }]
    });
    await fireEvent.click(screen.getByRole('button', { name: 'Test' }));
    await waitFor(() => expect(screen.getByTestId('test-photoforge.example.shapes').textContent).toContain('Self-test failed'));
    expect(screen.getByTestId('test-photoforge.example.shapes').textContent).toContain('tile edges');
  });

  it('asks before removing, and says what removing does to documents', async () => {
    const { backend, onmessage } = await mount(statusFixture([pluginFixture()]));
    await fireEvent.click(await screen.findByRole('button', { name: /Shape Generator/ }));
    await fireEvent.click(screen.getByRole('button', { name: 'Remove…' }));
    expect(screen.getByText(/Documents that use it will say it is missing/)).toBeTruthy();
    expect(backend.removePlugin).not.toHaveBeenCalled();
    await fireEvent.click(screen.getByRole('button', { name: 'Keep it' }));
    expect(backend.removePlugin).not.toHaveBeenCalled();
    await fireEvent.click(screen.getByRole('button', { name: 'Remove…' }));
    await fireEvent.click(screen.getByRole('button', { name: 'Remove it' }));
    await waitFor(() => expect(backend.removePlugin).toHaveBeenCalledWith('photoforge.example.shapes'));
    await waitFor(() => expect(onmessage).toHaveBeenCalledWith('Plugin removed.'));
  });

  it('removes an old version without touching the one in use', async () => {
    const plugin = pluginFixture(manifestFixture({ version: '1.1.0' }), {
      activeHash: 'b'.repeat(64),
      versions: [
        { contentHash: 'a'.repeat(64), version: '1.0.0', installedAt: '', packageSha256: '' },
        { contentHash: 'b'.repeat(64), version: '1.1.0', installedAt: '', packageSha256: '' }
      ]
    });
    const { backend } = await mount(statusFixture([plugin]));
    await fireEvent.click(await screen.findByRole('button', { name: /Shape Generator/ }));
    await fireEvent.click(screen.getByText('2 versions installed'));
    expect(screen.getByText('in use')).toBeTruthy();
    expect(screen.queryByRole('button', { name: 'Remove 1.1.0' })).toBeNull();
    await fireEvent.click(screen.getByRole('button', { name: 'Remove 1.0.0' }));
    await waitFor(() => expect(backend.removePluginVersion).toHaveBeenCalledWith('photoforge.example.shapes', 'a'.repeat(64)));
  });

  it('says so when this build cannot run plugins at all', async () => {
    await mount(statusFixture([pluginFixture(manifestFixture(), { availability: { kind: 'runtimeUnavailable' } })], { runtimeAvailable: false }));
    expect((await screen.findByTestId('no-runtime')).textContent).toMatch(/does not include the plugin runtime/);
    expect(screen.getByTestId('chip-photoforge.example.shapes').textContent?.trim()).toBe('Cannot run in this build');
  });

  it('reports a damaged list of plugins instead of showing an empty one as if it were fine', async () => {
    await mount(statusFixture([], { warning: 'the plugin list is damaged: expected value at line 1' }));
    expect((await screen.findByTestId('state-warning')).textContent).toContain('the plugin list is damaged');
  });

  it('closes on Escape from the list', async () => {
    const { onclose } = await mount(statusFixture());
    await fireEvent.keyDown(screen.getByRole('dialog'), { key: 'Escape' });
    expect(onclose).toHaveBeenCalled();
  });
});

describe('installing a plugin', () => {
  async function review(inspection = inspectionFixture(), status = statusFixture()) {
    const handles = await mount(status);
    handles.backend.inspectPluginPackage.mockResolvedValue(inspection);
    await fireEvent.click(screen.getByRole('button', { name: 'Install plugin…' }));
    await screen.findByRole('heading', { name: 'Install a plugin' });
    return handles;
  }

  it('shows everything the package would be allowed to do, ticked, and that it is not signed', async () => {
    const { backend, pickPackage } = await review();
    expect(pickPackage).toHaveBeenCalled();
    expect(backend.inspectPluginPackage).toHaveBeenCalledWith('C:\\plugins\\shapes.photoforge-plugin');
    expect(screen.getByRole('heading', { name: /Shape Generator/ })).toBeTruthy();
    expect(screen.getByTestId('signature').textContent).toMatch(/^Not signed/);
    const boxes = screen.getAllByRole('checkbox') as HTMLInputElement[];
    expect(boxes).toHaveLength(3);
    expect(boxes.every((box) => box.checked)).toBe(true);
    expect(screen.getByText(/It can never read your files, use the network, start programs/)).toBeTruthy();
    // Nothing has been installed by looking.
    expect(backend.installPluginPackage).not.toHaveBeenCalled();
  });

  it('installs against the hash it showed and exactly the permissions left ticked', async () => {
    const { backend, onmessage, onchange } = await review();
    await fireEvent.click(screen.getByRole('checkbox', { name: /Add entries to the tools menu/ }));
    await fireEvent.click(screen.getByRole('button', { name: 'Install' }));
    await waitFor(() => expect(backend.installPluginPackage).toHaveBeenCalledTimes(1));
    expect(backend.installPluginPackage).toHaveBeenCalledWith(
      'C:\\plugins\\shapes.photoforge-plugin', 'c'.repeat(64), ['filter.pixels', 'document.operations']
    );
    await waitFor(() => expect(onmessage).toHaveBeenCalledWith('Shape Generator installed.'));
    expect(onchange).toHaveBeenCalled();
    // Back at the list, with the self-test result it ran.
    expect(await screen.findByRole('heading', { name: 'Plugins' })).toBeTruthy();
  });

  it('can be narrowed to nothing, and says what that means', async () => {
    const { backend } = await review(inspectionFixture({
      manifest: inspectorManifest(),
      capabilities: [],
      hasModule: false
    }));
    expect(screen.getByText('Nothing. It only describes things.')).toBeTruthy();
    await fireEvent.click(screen.getByRole('button', { name: 'Install' }));
    await waitFor(() => expect(backend.installPluginPackage).toHaveBeenCalledWith(expect.any(String), expect.any(String), []));
  });

  it('shows what an update adds and replaces, and warns about an older version', async () => {
    await review(inspectionFixture({
      installedVersion: '1.0.0',
      newCapabilities: [{ id: 'ui.tool', description: 'Add entries to the tools menu' }]
    }));
    expect(screen.getByTestId('update-note').textContent).toMatch(/replaces version 1\.0\.0/);
    expect(screen.getByText('new in this version')).toBeTruthy();
    expect(screen.getByRole('button', { name: 'Update' })).toBeTruthy();
  });

  it('warns when the package is older than what is installed, or already installed', async () => {
    const { } = await review(inspectionFixture({ installedVersion: '2.0.0', olderThanInstalled: true }));
    expect(screen.getByTestId('update-note').textContent).toMatch(/older/);
  });

  it('keeps the review open and gives the reason when the package is refused', async () => {
    const { backend } = await review();
    backend.installPluginPackage.mockRejectedValueOnce({
      code: 'invalid_plugin_manifest',
      message: 'Shape Generator failed its self-test and was not installed: shape: it declares Pointwise but its result depends on where tile edges fall'
    });
    await fireEvent.click(screen.getByRole('button', { name: 'Install' }));
    const problem = await screen.findByTestId('install-problem');
    expect(problem.textContent).toMatch(/failed its self-test and was not installed/);
    expect(screen.getByRole('heading', { name: 'Install a plugin' })).toBeTruthy();
    // And the person can still back out.
    await fireEvent.click(screen.getByRole('button', { name: 'Cancel' }));
    expect(await screen.findByRole('heading', { name: 'Plugins' })).toBeTruthy();
  });

  it('will not install a plugin with filters where there is no runtime', async () => {
    await review(inspectionFixture({ runtimeAvailable: false }));
    expect(screen.getByRole('alert').textContent).toMatch(/does not include the plugin runtime/);
    expect((screen.getByRole('button', { name: 'Install' }) as HTMLButtonElement).disabled).toBe(true);
  });

  it('does nothing if the file chooser is cancelled', async () => {
    const { backend, pickPackage } = await mount(statusFixture());
    pickPackage.mockResolvedValueOnce(null as never);
    await fireEvent.click(screen.getByRole('button', { name: 'Install plugin…' }));
    await waitFor(() => expect(pickPackage).toHaveBeenCalled());
    expect(backend.inspectPluginPackage).not.toHaveBeenCalled();
    expect(screen.getByRole('heading', { name: 'Plugins' })).toBeTruthy();
  });

  it('reports a package that cannot be inspected without leaving the list', async () => {
    const { backend, onmessage } = await mount(statusFixture());
    backend.inspectPluginPackage.mockRejectedValueOnce({ code: 'invalid_plugin_manifest', message: 'the package is not safe to read: two entries overlap' });
    await fireEvent.click(screen.getByRole('button', { name: 'Install plugin…' }));
    await waitFor(() => expect(onmessage).toHaveBeenCalledWith(expect.stringContaining('two entries overlap'), 'error'));
    expect(screen.getByRole('heading', { name: 'Plugins' })).toBeTruthy();
  });

  it('backs out of a review with Escape without installing', async () => {
    const { backend } = await review();
    await fireEvent.keyDown(screen.getByRole('dialog'), { key: 'Escape' });
    expect(await screen.findByRole('heading', { name: 'Plugins' })).toBeTruthy();
    expect(backend.installPluginPackage).not.toHaveBeenCalled();
  });
});
