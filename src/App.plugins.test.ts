import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { fireEvent, render, screen, waitFor, within } from '@testing-library/svelte';
import { invoke } from '@tauri-apps/api/core';
import { open, save } from '@tauri-apps/plugin-dialog';
import App from './App.svelte';
import { createAdjustmentLayer, createDocument, createPixelLayer, insertLayer } from './lib/layers/tree';
import type { LayerDocument } from './lib/layers/types';
import type { ImageMetadata, EditOperation } from './lib/types/editor';
import type { TransactionRequest } from './lib/operations/types';
import type { PluginSummary, RequirementStatus } from './lib/plugins/types';
import { inspectorManifest, manifestFixture, pluginFixture, statusFixture } from './lib/plugins/testing';

vi.mock('@tauri-apps/api/core', () => ({ invoke: vi.fn() }));
vi.mock('@tauri-apps/plugin-dialog', () => ({ open: vi.fn(), save: vi.fn() }));
vi.mock('@tauri-apps/api/webview', () => ({
  getCurrentWebview: () => ({ onDragDropEvent: async () => () => undefined })
}));
vi.mock('@tauri-apps/api/window', () => ({
  getCurrentWindow: () => ({ onCloseRequested: async () => () => undefined })
}));

const FIXTURE = String.raw`C:\fixtures\fixture.png`;
const previewUrl = 'data:image/png;base64,b3JpZ2luYWw=';
const metadata: ImageMetadata = {
  filename: 'fixture.png', width: 16, height: 12, format: 'PNG', fileSize: 128,
  colorSpace: 'sRGB', bitDepth: 8, hasAlpha: true, createdAt: null,
  modifiedAt: null, cameraModel: null, exifAvailable: false
};

const calls = (command: string) => vi.mocked(invoke).mock.calls.filter(([name]) => name === command);
const argsFor = (command: string) => calls(command).at(-1)?.[1] as Record<string, any>;

let plugins: PluginSummary[];
let requirements: RequirementStatus[];
let transactionResult: (request: TransactionRequest) => unknown;

function pluginOperation(sha = 'f'.repeat(64)): EditOperation {
  return {
    type: 'plugin_filter', plugin: 'photoforge.example.solarize', version: '1.0.0', sha256: sha,
    filter: 'solarize', locality: { kind: 'pointwise' }, parameters: [0.5]
  };
}

beforeEach(() => {
  localStorage.clear();
  plugins = [];
  requirements = [];
  transactionResult = (request) => ({
    document: request.document, revision: 'r'.repeat(64), label: request.label, steps: [],
    createdPixelIds: [], lastCreated: null
  });
  vi.mocked(open).mockReset();
  vi.mocked(save).mockReset();
  vi.spyOn(window, 'confirm').mockReturnValue(true);
  vi.spyOn(HTMLCanvasElement.prototype, 'getContext').mockImplementation(
    () =>
      ({
        createImageData: (width: number, height: number) => ({ data: new Uint8ClampedArray(width * height * 4), width, height }),
        putImageData: vi.fn(),
        clearRect: vi.fn()
      }) as unknown as CanvasRenderingContext2D
  );
  vi.mocked(invoke).mockReset().mockImplementation(async (command, options) => {
    const args = (options ?? {}) as Record<string, any>;
    switch (command) {
      case 'list_plugins':
        return statusFixture(plugins);
      case 'document_plugin_status':
        return requirements;
      case 'list_operations':
        return [
          { id: 'core.layer.add_group', title: 'New group' },
          { id: 'core.layer.add_pixel', title: 'New pixel layer' }
        ];
      case 'plugin_remembered_values':
        return {};
      case 'remember_plugin_values':
        return undefined;
      case 'probe_image_source':
        return {
          path: FIXTURE, filename: 'fixture.png', kind: 'png', width: 16, height: 12, fileBytes: 128, hasIcc: false,
          bitDepth: 8,
          report: {
            verdict: { kind: 'fullResolution' }, options: [{ kind: 'openFull', peakBytes: 1 }], fullPeakBytes: 1,
            budgetBytes: 1e9, availableBytes: 1e9, regionCost: null, outOfCore: 'n/a'
          }
        };
      case 'open_image':
        return {
          metadata, documentId: 9001, isCurrent: true, backgroundPixelId: 'pxopened',
          originalPreviewDataUrl: previewUrl, previewDataUrl: previewUrl, processingTimeMs: 1
        };
      case 'apply_transaction':
        return transactionResult(args.request as TransactionRequest);
      case 'run_plugin_command':
        return transactionResult({
          document: args.request.document, label: 'Add a group called Inspected', steps: [], origin: 'plugin'
        } as unknown as TransactionRequest);
      case 'render_layer_composite':
      case 'render_preview':
        return {
          requestId: args.requestId, isCurrent: true, previewDataUrl: previewUrl, processingTimeMs: 1,
          operationCount: 0, missingPlugins: []
        };
      case 'render_layer_thumbnail':
        return { layerId: args.layerId, previewDataUrl: previewUrl, width: 16, height: 12 };
      case 'list_recovery_snapshots':
        return { snapshots: [] };
      case 'analyze_image':
        return { isCurrent: false };
      case 'get_component_snapshot':
        return { configuration: { activePlanner: 'rule', ollamaSelectedModel: null, ollamaMaxOperations: 8 } };
      case 'get_ollama_diagnostics':
        return { connected: false };
      default:
        return {};
    }
  });
});

afterEach(() => vi.restoreAllMocks());

async function openImage() {
  vi.mocked(open).mockResolvedValueOnce(FIXTURE);
  await fireEvent.click(screen.getByRole('button', { name: 'Open' }));
  await waitFor(() => expect(screen.getByAltText('Edited preview of fixture.png')).toBeTruthy());
}

async function openPalette() {
  await fireEvent.keyDown(window, { key: 'P', ctrlKey: true, shiftKey: true });
  return screen.findByRole('combobox', { name: 'Search commands' });
}

describe('the command palette in the application', () => {
  it('opens on Ctrl+Shift+P, even with nothing open, and from the header button', async () => {
    render(App);
    const input = await openPalette();
    expect(document.activeElement).toBe(input);
    expect(screen.getByRole('listbox', { name: 'Commands' })).toBeTruthy();
    await fireEvent.keyDown(input, { key: 'Escape' });
    expect(screen.queryByRole('combobox', { name: 'Search commands' })).toBeNull();
    await fireEvent.click(screen.getByRole('button', { name: 'Commands' }));
    expect(await screen.findByRole('combobox', { name: 'Search commands' })).toBeTruthy();
  });

  it('runs a built-in command, which is the same call the interface makes', async () => {
    render(App);
    const input = await openPalette();
    vi.mocked(open).mockResolvedValueOnce(null);
    await fireEvent.input(input, { target: { value: 'open image' } });
    await fireEvent.keyDown(input, { key: 'Enter' });
    await waitFor(() => expect(open).toHaveBeenCalled());
    expect(screen.queryByRole('combobox', { name: 'Search commands' })).toBeNull();
  });

  it('says why a command cannot run when there is no layered document', async () => {
    render(App);
    const input = await openPalette();
    await fireEvent.input(input, { target: { value: 'merge down' } });
    const row = await screen.findByRole('option', { name: /Merge down/ });
    expect(row.getAttribute('aria-disabled')).toBe('true');
    expect(row.textContent).toMatch(/Select a layer|layered document/);
  });

  it('opens the plugin manager from the palette and puts the application behind it out of reach', async () => {
    render(App);
    const input = await openPalette();
    await fireEvent.input(input, { target: { value: 'manage plugins' } });
    await fireEvent.keyDown(input, { key: 'Enter' });
    expect(await screen.findByRole('heading', { name: 'Plugins' })).toBeTruthy();
    // `inert` and `aria-hidden` are set together; jsdom reflects only the latter.
    expect(document.querySelector('.app-shell')?.getAttribute('aria-hidden')).toBe('true');
    // The keyboard belongs to the dialog while it is open.
    await fireEvent.keyDown(window, { key: 'P', ctrlKey: true, shiftKey: true });
    expect(screen.queryByRole('combobox', { name: 'Search commands' })).toBeNull();
    await fireEvent.click(screen.getByRole('button', { name: 'Close plugins' }));
    await waitFor(() => expect(screen.queryByRole('heading', { name: 'Plugins' })).toBeNull());
  });

  it('lists a plugin\'s commands beside the built-in ones, and runs one as a plugin transaction', async () => {
    plugins = [pluginFixture(inspectorManifest())];
    render(App);
    await openImage();
    const input = await openPalette();
    await fireEvent.input(input, { target: { value: 'inspected' } });
    const row = await screen.findByRole('option', { name: /Document Inspector: Add a group called Inspected/ });
    await fireEvent.click(row);
    await waitFor(() => expect(calls('run_plugin_command')).toHaveLength(1));
    expect(argsFor('run_plugin_command').request).toMatchObject({
      plugin: 'photoforge.example.inspector', command: 'add_inspected_group', values: {}
    });
    expect((argsFor('run_plugin_command').request.document as LayerDocument).canvasWidth).toBe(16);
  });
});

describe('plugins in the editing interface', () => {
  it('shows the panel a plugin declares, with facts from the open document', async () => {
    plugins = [pluginFixture(inspectorManifest())];
    render(App);
    await openImage();
    const panel = await screen.findByRole('region', { name: /Document facts, from Document Inspector/ });
    const fact = (label: string) => within(panel).getByText(label).closest('.fact')!.textContent!.replace(label, '').trim();
    await waitFor(() => expect(fact('Canvas width')).toBe('16 px'));
    expect(fact('Layers')).toBe('1');
    expect(fact('Selected layer')).toBe('Background');
  });

  it('adds a plugin filter as a live adjustment layer through the transaction engine', async () => {
    plugins = [pluginFixture(manifestFixture({ commands: [], tools: [] }))];
    transactionResult = (request) => ({
      document: insertLayer(
        request.document,
        createAdjustmentLayer('Draw shape', {
          type: 'plugin_filter', plugin: 'photoforge.example.shapes', version: '1.0.0', sha256: 'a'.repeat(64),
          filter: 'shape', locality: { kind: 'pointwise' }, parameters: [0, 0.5]
        } as never),
        null,
        request.document.layers.length
      ),
      revision: 'r'.repeat(64), label: request.label, steps: [], createdPixelIds: [], lastCreated: null
    });
    render(App);
    await openImage();
    await fireEvent.click(await screen.findByRole('button', { name: /Run a plugin filter/ }));
    const dialog = await screen.findByRole('dialog', { name: 'Run a plugin filter' });
    const add = within(dialog).getByRole('button', { name: 'Add as adjustment layer' }) as HTMLButtonElement;
    await waitFor(() => expect(add.disabled).toBe(false));
    await fireEvent.change(within(dialog).getByLabelText('Shape'), { target: { value: '2' } });
    await fireEvent.click(add);

    await waitFor(() => expect(calls('apply_transaction')).toHaveLength(1));
    const request = argsFor('apply_transaction').request as TransactionRequest;
    expect(request.origin).toBe('user');
    expect(request.steps).toEqual([
      { op: 'core.plugin.add_adjustment', params: {
        plugin: 'photoforge.example.shapes', filter: 'shape', parameters: { shape: 2, size: 0.5 } } }
    ]);
    // The new layer is in the Layers panel, and the person's settings are remembered.
    await waitFor(() => expect(screen.getAllByText('Draw shape').length).toBeGreaterThan(0));
    expect(argsFor('remember_plugin_values')).toMatchObject({
      plugin: 'photoforge.example.shapes', key: 'filter:shape', values: { shape: 2, size: 0.5 }
    });
  });

  it('bakes a filter into the selected layer when asked, naming that layer as the target', async () => {
    plugins = [pluginFixture(manifestFixture({ commands: [], tools: [] }))];
    render(App);
    await openImage();
    await fireEvent.click(await screen.findByRole('button', { name: /Run a plugin filter/ }));
    const dialog = await screen.findByRole('dialog', { name: 'Run a plugin filter' });
    const bake = within(dialog).getByRole('button', { name: 'Apply to Background' }) as HTMLButtonElement;
    await waitFor(() => expect(bake.disabled).toBe(false));
    await fireEvent.click(bake);
    await waitFor(() => expect(calls('apply_transaction')).toHaveLength(1));
    expect((argsFor('apply_transaction').request as TransactionRequest).steps[0]).toMatchObject({
      op: 'core.plugin.apply_filter',
      params: { selector: { type: 'active' }, plugin: 'photoforge.example.shapes', filter: 'shape' }
    });
  });

  it('shows the reason, and changes nothing, when the transaction is refused', async () => {
    plugins = [pluginFixture(manifestFixture({ commands: [], tools: [] }))];
    render(App);
    await openImage();
    await fireEvent.click(await screen.findByRole('button', { name: /Run a plugin filter/ }));
    const dialog = await screen.findByRole('dialog', { name: 'Run a plugin filter' });
    const add = within(dialog).getByRole('button', { name: 'Add as adjustment layer' }) as HTMLButtonElement;
    await waitFor(() => expect(add.disabled).toBe(false));
    transactionResult = () => {
      throw { code: 'transaction_failed', message: 'Step 1 (core.plugin.add_adjustment) failed: the plugin stopped with an error. Nothing was changed.' };
    };
    await fireEvent.click(add);
    expect(await screen.findByText(/Nothing was changed/)).toBeTruthy();
    // Nothing was committed, so there is still nothing to undo.
    expect((screen.getByRole('button', { name: 'Undo' }) as HTMLButtonElement).disabled).toBe(true);
  });

  it('runs a plugin tool through the plugin command path, asking for its parameters first', async () => {
    plugins = [pluginFixture()];
    render(App);
    await openImage();
    await fireEvent.click(await screen.findByRole('button', { name: 'Add shape' }));
    const dialog = await screen.findByRole('dialog', { name: 'Add shape layer' });
    expect(within(dialog).getByTestId('steps').textContent).toContain('New pixel layer');
    await fireEvent.change(within(dialog).getByLabelText('Shape'), { target: { value: '1' } });
    await fireEvent.click(within(dialog).getByRole('button', { name: 'Run' }));
    await waitFor(() => expect(calls('run_plugin_command')).toHaveLength(1));
    expect(argsFor('run_plugin_command').request).toMatchObject({
      plugin: 'photoforge.example.shapes', command: 'add_shape', values: { shape: 1, size: 0.5 }
    });
  });

  it('says which plugin a document needs when it cannot be had, and marks the layer', async () => {
    plugins = [];
    requirements = [];
    render(App);
    await openImage();
    // Open a project whose layer uses a plugin that is not installed.
    const document = insertLayer(
      createDocument(16, 12, [createPixelLayer('Background', 'pxopened', 16, 12)], 'linear_srgb_f32'),
      createAdjustmentLayer('Solarize', pluginOperation() as never), null, 1
    );
    const layerId = document.layers[1].id;
    requirements = [{
      plugin: 'photoforge.example.solarize', version: '1.0.0', sha256: 'f'.repeat(64), filters: ['solarize'],
      layerIds: [layerId], availability: { kind: 'missing' }, message: 'photoforge.example.solarize 1.0.0 is not installed.'
    }];
    vi.mocked(open).mockResolvedValueOnce(String.raw`C:\fixtures\needs-plugin.photoforge`);
    const original = vi.mocked(invoke).getMockImplementation()!;
    vi.mocked(invoke).mockImplementation(async (command, options) => {
      if (command === 'load_layer_project') {
        return {
          documentId: 9100, isCurrent: true, metadata: { ...metadata, filename: 'needs-plugin.photoforge' },
          originalPreviewDataUrl: previewUrl, previewDataUrl: previewUrl, document, operations: [],
          canvasWidth: 16, canvasHeight: 12, applicationVersion: '0.14.0', createdAt: '', modifiedAt: '',
          processingTimeMs: 1, pluginRequirements: requirements
        };
      }
      return original(command, options);
    });
    await fireEvent.click(screen.getByRole('button', { name: 'Open project' }));
    const alert = await screen.findByTestId('needs-plugins');
    expect(alert.textContent).toContain('photoforge.example.solarize 1.0.0 is not installed.');
    expect(alert.textContent).toContain('refuse to export');
    // The layer row says so, in words, and the layer is still there.
    expect(await screen.findByText(/Plugin unavailable — left out of the preview/)).toBeTruthy();
    expect(screen.getAllByText('Solarize').length).toBeGreaterThan(0);
    expect(argsFor('document_plugin_status').operations).toEqual([]);
  });

  it('does not ask the backend about plugins for a document that uses none', async () => {
    render(App);
    await openImage();
    await waitFor(() => expect(calls('list_plugins').length).toBeGreaterThan(0));
    expect(calls('document_plugin_status')).toHaveLength(0);
    expect(screen.queryByTestId('needs-plugins')).toBeNull();
  });
});
