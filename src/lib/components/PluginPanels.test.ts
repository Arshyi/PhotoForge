import { fireEvent, render, screen, waitFor, within } from '@testing-library/svelte';
import { describe, expect, it, vi } from 'vitest';
import PluginsPanel from './PluginsPanel.svelte';
import PluginFilterDialog from './PluginFilterDialog.svelte';
import PluginCommandDialog from './PluginCommandDialog.svelte';
import ParamForm from './ParamForm.svelte';
import { inspectorManifest, manifestFixture, pluginFixture } from '../plugins/testing';
import { createDocument, createPixelLayer } from '../layers/tree';
import type { RequirementStatus } from '../plugins/types';
import type { OperationSpec } from '../operations/types';

const document = (() => {
  const base = createDocument(320, 200, [createPixelLayer('Sky', 'px1', 320, 200)], 'linear_srgb_f32');
  return { ...base, activeLayerId: base.layers[0].id };
})();

describe('the plugins panel', () => {
  it('shows a plugin\'s declared panel with real facts, and runs its command button', async () => {
    const oncommand = vi.fn();
    const inspector = pluginFixture(inspectorManifest());
    render(PluginsPanel, { plugins: [inspector], document, oncommand });
    const panel = screen.getByRole('region', { name: /Document facts, from Document Inspector/ });
    expect(within(panel).getByText('Read from the open document.')).toBeTruthy();
    const fact = (label: string) => within(panel).getByText(label).closest('.fact')!.textContent!.replace(label, '').trim();
    expect(fact('Layers')).toBe('1');
    expect(fact('Canvas width')).toBe('320 px');
    expect(fact('Selected layer')).toBe('Sky');
    await fireEvent.click(within(panel).getByRole('button', { name: 'Add a group called Inspected' }));
    expect(oncommand).toHaveBeenCalledWith(expect.objectContaining({ id: 'photoforge.example.inspector' }), 'add_inspected_group');
  });

  it('shows no facts to a plugin that was not allowed to read them', () => {
    const restricted = pluginFixture(inspectorManifest(), { granted: ['ui.panel', 'document.operations'] });
    render(PluginsPanel, { plugins: [restricted], document });
    const panel = screen.getByRole('region', { name: /Document facts/ });
    expect(within(panel).getAllByText('Not allowed').length).toBeGreaterThan(0);
    expect(within(panel).queryByText('320 px')).toBeNull();
  });

  it('shows no panel at all for a plugin that was not allowed panels, or is not available', () => {
    const noPanel = pluginFixture(inspectorManifest(), { granted: ['document.read'] });
    const off = pluginFixture(manifestFixture({ id: 'com.example.off' }), { availability: { kind: 'disabled' } });
    render(PluginsPanel, { plugins: [noPanel, off], document });
    expect(screen.queryByRole('region', { name: /Document facts/ })).toBeNull();
  });

  it('lists tools, but only for a plugin allowed both the tools menu and changing the document', async () => {
    const oncommand = vi.fn();
    const { unmount } = render(PluginsPanel, { plugins: [pluginFixture()], document, oncommand });
    const tools = screen.getByRole('group', { name: 'Plugin tools' });
    await fireEvent.click(within(tools).getByRole('button', { name: 'Add shape' }));
    expect(oncommand).toHaveBeenCalledWith(expect.anything(), 'add_shape');
    unmount();
    render(PluginsPanel, { plugins: [pluginFixture(manifestFixture(), { granted: ['filter.pixels', 'ui.tool'] })], document });
    expect(screen.queryByRole('group', { name: 'Plugin tools' })).toBeNull();
  });

  it('disables commands and tools while something else owns the document, or when there is none', () => {
    const { unmount } = render(PluginsPanel, { plugins: [pluginFixture()], document, busy: true });
    expect((screen.getByRole('button', { name: 'Add shape' }) as HTMLButtonElement).disabled).toBe(true);
    unmount();
    render(PluginsPanel, { plugins: [pluginFixture()], document: null });
    expect((screen.getByRole('button', { name: 'Add shape' }) as HTMLButtonElement).disabled).toBe(true);
  });

  it('counts the filters that can run, and opens the filter dialog and the manager', async () => {
    const onfilter = vi.fn();
    const onmanage = vi.fn();
    render(PluginsPanel, { plugins: [pluginFixture()], document, onfilter, onmanage });
    const filters = screen.getByRole('button', { name: /Run a plugin filter/ }) as HTMLButtonElement;
    expect(filters.textContent).toContain('1');
    await fireEvent.click(filters);
    await fireEvent.click(screen.getByRole('button', { name: 'Manage plugins…' }));
    expect(onfilter).toHaveBeenCalled();
    expect(onmanage).toHaveBeenCalled();
  });

  it('disables the filter button when no filter can run', () => {
    render(PluginsPanel, { plugins: [], document });
    expect((screen.getByRole('button', { name: /Run a plugin filter/ }) as HTMLButtonElement).disabled).toBe(true);
    expect(screen.getByText('No plugins installed.')).toBeTruthy();
  });

  it('says what the document needs that cannot be had, and what that means for it', () => {
    const unavailable: RequirementStatus[] = [{
      plugin: 'photoforge.example.solarize', version: '1.0.0', sha256: 'a'.repeat(64), filters: ['solarize'],
      layerIds: ['l1'], availability: { kind: 'missing' }, message: 'photoforge.example.solarize 1.0.0 is not installed.'
    }];
    render(PluginsPanel, { plugins: [], document, unavailable });
    const alert = screen.getByTestId('needs-plugins');
    expect(alert.getAttribute('role')).toBe('alert');
    expect(alert.textContent).toContain('photoforge.example.solarize 1.0.0 is not installed.');
    expect(alert.textContent).toContain('left out of the preview and refuse to export');
    expect(alert.textContent).toContain('Nothing has been changed in the document');
  });
});

describe('the parameter form', () => {
  const params = manifestFixture().commands![0].parameters!;

  it('draws one control per declared parameter, from four kinds only', async () => {
    const onchange = vi.fn();
    render(ParamForm, {
      params: [
        ...params,
        { id: 'width', title: 'Width', type: 'integer', min: 0, max: 10, default: 4 },
        { id: 'invert', title: 'Invert', type: 'bool', default: false }
      ],
      values: { shape: 0, size: 0.5, width: 4, invert: 0 },
      onchange
    });
    await fireEvent.change(screen.getByLabelText('Shape'), { target: { value: '2' } });
    expect(onchange).toHaveBeenLastCalledWith({ shape: 2, size: 0.5, width: 4, invert: 0 });
    await fireEvent.click(screen.getByRole('checkbox', { name: 'Invert' }));
    expect(onchange).toHaveBeenLastCalledWith(expect.objectContaining({ invert: 1 }));
    await fireEvent.input(screen.getByRole('slider', { name: 'Size slider' }), { target: { value: '0.75' } });
    expect(onchange).toHaveBeenLastCalledWith(expect.objectContaining({ size: 0.75 }));
  });

  it('does not take a typed value the plugin would refuse', async () => {
    const onchange = vi.fn();
    render(ParamForm, { params, values: { shape: 0, size: 0.5 }, onchange });
    const box = screen.getByLabelText('Size') as HTMLInputElement;
    await fireEvent.change(box, { target: { value: '7' } });
    await fireEvent.change(box, { target: { value: 'abc' } });
    await fireEvent.change(box, { target: { value: '' } });
    expect(onchange).not.toHaveBeenCalled();
    await fireEvent.change(box, { target: { value: '0.9' } });
    expect(onchange).toHaveBeenCalledWith({ shape: 0, size: 0.9 });
  });
});

describe('the plugin filter dialog', () => {
  const plugin = pluginFixture();
  const layer = { name: 'Sky', pixel: true, locked: false };

  async function mount(extra: Record<string, unknown> = {}) {
    const onrun = vi.fn();
    const oncancel = vi.fn();
    render(PluginFilterDialog, { plugins: [plugin], layer, onrun, oncancel, ...extra });
    await screen.findByRole('dialog');
    return { onrun, oncancel };
  }

  it('offers each filter by plugin and title, with how far it reaches', async () => {
    await mount();
    expect(screen.getByRole('option', { name: 'Shape Generator — Draw shape' })).toBeTruthy();
    expect(screen.getByTestId('locality').textContent).toMatch(/Each pixel on its own/);
  });

  it('starts from what the person used last, and runs it as a live adjustment layer', async () => {
    const { onrun } = await mount({ remembered: async () => ({ shape: 1, size: 0.25 }) });
    await waitFor(() => expect((screen.getByLabelText('Shape') as HTMLSelectElement).value).toBe('1'));
    await fireEvent.click(screen.getByRole('button', { name: 'Add as adjustment layer' }));
    expect(onrun).toHaveBeenCalledWith({
      mode: 'adjustment', plugin: 'photoforge.example.shapes', filter: 'shape', values: { shape: 1, size: 0.25 }
    });
  });

  it('can bake the filter into the selected layer\'s pixels instead', async () => {
    const { onrun } = await mount();
    // The buttons wake once the values the dialog starts from have loaded.
    const bake = screen.getByRole('button', { name: 'Apply to Sky' }) as HTMLButtonElement;
    await waitFor(() => expect(bake.disabled).toBe(false));
    await fireEvent.click(bake);
    expect(onrun).toHaveBeenCalledWith(expect.objectContaining({ mode: 'pixels', filter: 'shape' }));
  });

  it('says why baking is not possible for a locked layer, a non-pixel layer, or no layer', async () => {
    for (const [shape, reason] of [
      [{ name: 'Sky', pixel: true, locked: true }, /Sky is locked/],
      [{ name: 'Group', pixel: false, locked: false }, /Group is not a pixel layer/],
      [null, /Select a layer first/]
    ] as const) {
      const { unmount } = render(PluginFilterDialog, { plugins: [plugin], layer: shape });
      await screen.findByRole('dialog');
      expect(screen.getByRole('button', { name: /^Apply to/ }).hasAttribute('disabled')).toBe(true);
      expect(screen.getAllByText(reason).length).toBeGreaterThan(0);
      unmount();
    }
  });

  it('does not offer a live adjustment layer in a document that is not linear float', async () => {
    await mount({ linearDocument: false });
    expect((screen.getByRole('button', { name: 'Add as adjustment layer' }) as HTMLButtonElement).disabled).toBe(true);
    expect(screen.getByText(/need a linear float document/)).toBeTruthy();
  });

  it('warns that a whole-image filter may be refused on a large image', async () => {
    const global = pluginFixture(manifestFixture({
      filters: [{ id: 'g', title: 'Global', locality: { kind: 'global' } }]
    }));
    await mount({ plugins: [global] });
    expect(screen.getByTestId('locality').textContent).toMatch(/whole image at once/);
    expect(screen.getByTestId('locality').textContent).toMatch(/Very large images may be refused/);
  });

  it('says so when no filter can run, and cancels on Escape', async () => {
    const { oncancel } = await mount({ plugins: [] });
    expect(screen.getByText(/No installed plugin has a filter that can run/)).toBeTruthy();
    expect((screen.getByRole('button', { name: 'Add as adjustment layer' }) as HTMLButtonElement).disabled).toBe(true);
    await fireEvent.keyDown(screen.getByRole('dialog'), { key: 'Escape' });
    expect(oncancel).toHaveBeenCalled();
  });
});

describe('the plugin command dialog', () => {
  const plugin = pluginFixture();
  const command = plugin.manifest!.commands![0];
  const operations: OperationSpec[] = [
    { id: 'core.layer.add_pixel', title: 'New pixel layer' } as OperationSpec,
    { id: 'core.plugin.apply_filter', title: 'Apply plugin filter' } as OperationSpec
  ];

  it('shows the steps it will take, by name, as one undoable step, and runs with the chosen values', async () => {
    const onrun = vi.fn();
    render(PluginCommandDialog, { plugin, command, operations, remembered: { size: 0.8 }, onrun });
    const steps = screen.getByTestId('steps');
    expect(steps.textContent).toContain('one undoable step');
    expect(within(steps).getAllByRole('listitem').map((item) => item.textContent)).toEqual([
      'New pixel layer', 'Apply plugin filter'
    ]);
    await fireEvent.change(screen.getByLabelText('Shape'), { target: { value: '1' } });
    await fireEvent.click(screen.getByRole('button', { name: 'Run' }));
    expect(onrun).toHaveBeenCalledWith({ shape: 1, size: 0.8 });
  });

  it('falls back to the operation id when the registry has not loaded', () => {
    render(PluginCommandDialog, { plugin, command, operations: [] });
    expect(screen.getByText('core.layer.add_pixel')).toBeTruthy();
  });

  it('cancels on Escape without running', async () => {
    const onrun = vi.fn();
    const oncancel = vi.fn();
    render(PluginCommandDialog, { plugin, command, operations, onrun, oncancel });
    await fireEvent.keyDown(screen.getByRole('dialog'), { key: 'Escape' });
    expect(oncancel).toHaveBeenCalled();
    expect(onrun).not.toHaveBeenCalled();
  });
});
