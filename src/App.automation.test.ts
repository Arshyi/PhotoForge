import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { fireEvent, render, screen, waitFor, within } from '@testing-library/svelte';
import { invoke } from '@tauri-apps/api/core';
import { open, save } from '@tauri-apps/plugin-dialog';
import App from './App.svelte';
import registry from '../src-tauri/tests/fixtures/operation_registry.json';
import type { LayerDocument } from './lib/layers/types';
import type { ImageMetadata } from './lib/types/editor';
import type { OperationSpec, StepReport, TransactionRequest } from './lib/operations/types';
import type { PluginSummary } from './lib/plugins/types';
import { MACRO_STORAGE_KEY, insertStep, macroDocument, newMacro, newStep, type Macro } from './lib/automation/macros';
import { statusFixture } from './lib/plugins/testing';

vi.setConfig({ testTimeout: 30_000 });
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
const requestOf = (command: string) => argsFor(command).request as TransactionRequest;

let plugins: PluginSummary[];
let transactionResult: (request: TransactionRequest) => unknown;
let planResult: (request: TransactionRequest) => unknown;
let failWith: string | null;
let macroText: string;

/** A transaction result in which the first layer has been hidden, as a real run would return. */
function hidden(request: TransactionRequest, skipped = false) {
  const document: LayerDocument = {
    ...request.document,
    layers: request.document.layers.map((layer, index) => (index === 0 ? { ...layer, visible: false } : layer))
  };
  const steps: StepReport[] = request.steps.map((step, index) => ({
    index, op: step.op, skipped, createdLayers: [], createdPixels: []
  }));
  return { document, revision: 'r'.repeat(64), label: request.label, steps, createdPixelIds: [], lastCreated: null };
}

function seed(...macros: Macro[]) {
  localStorage.setItem(MACRO_STORAGE_KEY, JSON.stringify(macros));
}

function hideBackgroundMacro(name = 'Hide it'): Macro {
  const macro = insertStep(newMacro(name), newStep('core.layer.set_visible', { selector: { type: 'active' }, visible: false }));
  return { ...macro, name };
}

beforeEach(() => {
  localStorage.clear();
  plugins = [];
  failWith = null;
  macroText = '';
  transactionResult = (request) => hidden(request);
  planResult = (request) => hidden(request, false).steps;
  vi.mocked(open).mockReset();
  vi.mocked(save).mockReset();
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
        return [];
      case 'list_operations':
        return registry as unknown as OperationSpec[];
      case 'apply_transaction':
        if (failWith) throw new Error(failWith);
        return transactionResult(args.request as TransactionRequest);
      case 'plan_transaction':
        if (failWith) throw new Error(failWith);
        return planResult(args.request as TransactionRequest);
      case 'import_macro':
        return macroText;
      case 'export_macro':
        return args.path;
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

async function runPaletteCommand(query: string) {
  await fireEvent.keyDown(window, { key: 'P', ctrlKey: true, shiftKey: true });
  const input = await screen.findByRole('combobox', { name: 'Search commands' });
  await fireEvent.input(input, { target: { value: query } });
  await fireEvent.keyDown(input, { key: 'Enter' });
}

async function openEditor() {
  await runPaletteCommand('automation');
  return screen.findByRole('dialog', { name: 'Macros' });
}

const closeEditor = async () => {
  await fireEvent.click(screen.getByRole('button', { name: 'Close automation' }));
  await waitFor(() => expect(screen.queryByRole('dialog', { name: 'Macros' })).toBeNull());
};

describe('the automation editor in the application', () => {
  it('opens from the palette over the specs the backend published, and puts the application out of reach', async () => {
    render(App);
    const dialog = await openEditor();
    expect(calls('list_operations').length).toBeGreaterThan(0);
    expect(within(dialog).getByText('No macros yet.')).toBeTruthy();
    expect(document.querySelector('.app-shell')?.getAttribute('aria-hidden')).toBe('true');
    // The keyboard belongs to the dialog: the palette does not open over it.
    await fireEvent.keyDown(window, { key: 'P', ctrlKey: true, shiftKey: true });
    expect(screen.queryByRole('combobox', { name: 'Search commands' })).toBeNull();
    await fireEvent.keyDown(dialog, { key: 'Escape' });
    await waitFor(() => expect(screen.queryByRole('dialog', { name: 'Macros' })).toBeNull());
  });

  it('builds a macro from the registry, saves it, and runs it as one automation transaction and one undo', async () => {
    render(App);
    await openImage();
    const dialog = await openEditor();
    await fireEvent.click(within(dialog).getByRole('button', { name: 'New macro' }));
    await fireEvent.change(within(dialog).getByLabelText('Add a step'), { target: { value: 'core.layer.set_visible' } });
    await fireEvent.click(within(dialog).getByRole('button', { name: 'Add step' }));
    // The step opens with the parameters the registry names, already valid.
    expect(within(dialog).getByLabelText('visible')).toBeTruthy();
    expect(within(dialog).queryByRole('alert', { name: 'Problems with this macro' })).toBeNull();
    expect(JSON.parse(localStorage.getItem(MACRO_STORAGE_KEY)!)[0].steps[0]).toMatchObject({
      op: 'core.layer.set_visible', params: { selector: { type: 'active' }, visible: false }, enabled: true
    });

    const run = within(dialog).getByRole('button', { name: 'Run' }) as HTMLButtonElement;
    await waitFor(() => expect(run.disabled).toBe(false));
    await fireEvent.click(run);
    await waitFor(() => expect(calls('apply_transaction')).toHaveLength(1));

    const request = requestOf('apply_transaction');
    expect(request.origin).toBe('automation');
    expect(request.label).toBe('New macro 1');
    expect(request.steps).toEqual([{ op: 'core.layer.set_visible', params: { selector: { type: 'active' }, visible: false } }]);
    expect(await within(dialog).findByText(/Ran “New macro 1”: 1 step\. One Undo reverses all of it\./)).toBeTruthy();

    await closeEditor();
    const undo = screen.getByRole('button', { name: 'Undo' }) as HTMLButtonElement;
    expect(undo.disabled).toBe(false);
    expect(screen.getByRole('button', { name: 'Show Background' })).toBeTruthy();
    await fireEvent.click(undo);
    await waitFor(() => expect(screen.getByRole('button', { name: 'Hide Background' })).toBeTruthy());
  });

  it('checks without changing anything, saying which steps would run and which would be skipped', async () => {
    const two = insertStep(hideBackgroundMacro('Two steps'), newStep('core.layer.add_group', {}, { kind: 'layer_exists', selector: { type: 'name', name: 'Nope' } }));
    seed(two);
    planResult = (request) =>
      request.steps.map((step, index) => ({ index, op: step.op, skipped: index === 1, createdLayers: [], createdPixels: [] }));
    render(App);
    await openImage();
    const dialog = await openEditor();
    await fireEvent.click(within(dialog).getByRole('button', { name: 'Check' }));
    expect(await within(dialog).findByText('Checked: 1 of 2 steps would run; 1 would be skipped. Nothing was changed.')).toBeTruthy();
    expect(within(dialog).getByText('would run')).toBeTruthy();
    expect(within(dialog).getByText('would be skipped')).toBeTruthy();
    expect(calls('plan_transaction')).toHaveLength(1);
    expect(calls('apply_transaction')).toHaveLength(0);
    expect(requestOf('plan_transaction').steps[1].when).toEqual({ kind: 'layer_exists', selector: { type: 'name', name: 'Nope' } });
    // Changing the macro makes the old answer stale rather than leaving it up.
    await fireEvent.input(within(dialog).getByLabelText('Name'), { target: { value: 'Renamed' } });
    await waitFor(() => expect(within(dialog).queryByText('would be skipped')).toBeNull());
  });

  it('commits nothing, and says so, when every step was skipped', async () => {
    seed(hideBackgroundMacro());
    transactionResult = (request) => hidden(request, true);
    render(App);
    await openImage();
    const dialog = await openEditor();
    await fireEvent.click(within(dialog).getByRole('button', { name: 'Run' }));
    expect(await within(dialog).findByText(/Nothing was changed: the step was skipped because its condition did not hold\./)).toBeTruthy();
    await closeEditor();
    expect((screen.getByRole('button', { name: 'Undo' }) as HTMLButtonElement).disabled).toBe(true);
    expect(screen.getByRole('button', { name: 'Hide Background' })).toBeTruthy();
  });

  it('shows the backend\'s reason when a macro cannot be done, and leaves the document alone', async () => {
    seed(hideBackgroundMacro());
    failWith = 'step 1: no layer is selected';
    render(App);
    await openImage();
    const dialog = await openEditor();
    await fireEvent.click(within(dialog).getByRole('button', { name: 'Run' }));
    expect((await within(dialog).findByRole('alert')).textContent).toContain('step 1: no layer is selected');
    await closeEditor();
    expect((screen.getByRole('button', { name: 'Undo' }) as HTMLButtonElement).disabled).toBe(true);
    expect(screen.getByRole('button', { name: 'Hide Background' })).toBeTruthy();
  });

  it('will not run a macro whose plugin is not installed, and says which', async () => {
    const withPlugin = insertStep(newMacro('Needs a plugin'), newStep('core.plugin.add_adjustment', { plugin: 'com.example.gone', filter: 'f' }));
    seed({ ...withPlugin, name: 'Needs a plugin' });
    render(App);
    await openImage();
    const dialog = await openEditor();
    expect(within(dialog).getByText(/com\.example\.gone — It is not installed\./)).toBeTruthy();
    const run = within(dialog).getByRole('button', { name: 'Run' }) as HTMLButtonElement;
    expect(run.disabled).toBe(true);
    expect(within(dialog).getByText(/Run is unavailable: com\.example\.gone: It is not installed\./)).toBeTruthy();
    expect(calls('apply_transaction')).toHaveLength(0);
  });

  it('exports a macro as a document and imports it back as a new macro without running anything', async () => {
    seed(hideBackgroundMacro('Original'));
    render(App);
    await openImage();
    const dialog = await openEditor();
    vi.mocked(save).mockResolvedValueOnce(String.raw`C:\out\original.json`);
    await fireEvent.click(within(dialog).getByRole('button', { name: 'Export…' }));
    await waitFor(() => expect(calls('export_macro')).toHaveLength(1));
    const exported = argsFor('export_macro');
    expect(exported.path).toBe(String.raw`C:\out\original.json`);
    expect(JSON.parse(exported.text)).toMatchObject({ kind: 'photoforge-macro', schemaVersion: 1, macro: { name: 'Original' } });

    macroText = exported.text;
    vi.mocked(open).mockResolvedValueOnce(String.raw`C:\out\original.json`);
    await fireEvent.click(within(dialog).getByRole('button', { name: 'Import…' }));
    expect(await within(dialog).findByText(/Imported “Original”\. It is a new macro; nothing it contains has run\./)).toBeTruthy();
    expect(within(dialog).getAllByRole('button', { name: /Original/ })).toHaveLength(2);
    expect(JSON.parse(localStorage.getItem(MACRO_STORAGE_KEY)!)).toHaveLength(2);
    expect(calls('apply_transaction')).toHaveLength(0);
  });

  it('refuses a file that is not a macro, with the reason', async () => {
    render(App);
    const dialog = await openEditor();
    macroText = JSON.stringify({ kind: 'photoforge-macro', schemaVersion: 1, macro: { name: 'x', steps: [{ op: 'core.layer.add_group', params: {}, when: { kind: 'eval', code: '1' } }] } });
    vi.mocked(open).mockResolvedValueOnce(String.raw`C:\hostile.json`);
    await fireEvent.click(within(dialog).getByRole('button', { name: 'Import…' }));
    expect((await within(dialog).findByRole('alert')).textContent).toMatch(/malformed/);
    expect(localStorage.getItem(MACRO_STORAGE_KEY)).toBeNull();
  });
});

describe('recording a macro in the application', () => {
  it('writes down what the Layers panel does as registered operations, and says what it could not', async () => {
    render(App);
    await openImage();
    await runPaletteCommand('record a macro');
    expect(await screen.findByText('Recording a macro.')).toBeTruthy();

    await fireEvent.click(screen.getByRole('button', { name: 'Hide Background' }));
    await waitFor(() => expect(screen.getByRole('button', { name: 'Show Background' })).toBeTruthy());
    // Undo changes what a replay would mean, so it is reported, not recorded.
    await fireEvent.click(screen.getByRole('button', { name: 'Undo' }));
    await waitFor(() => expect(screen.getByRole('button', { name: 'Hide Background' })).toBeTruthy());
    await fireEvent.click(screen.getByRole('button', { name: 'Hide Background' }));
    await waitFor(() => expect(screen.getByRole('button', { name: 'Show Background' })).toBeTruthy());

    await fireEvent.click(screen.getByRole('button', { name: 'Stop recording' }));
    const dialog = await screen.findByRole('dialog', { name: 'Macros' });
    expect(within(dialog).getByLabelText('Name')).toHaveProperty('value', 'Recorded macro 1');
    expect(within(dialog).getByText('Undo cannot be recorded, so it is not in this macro.')).toBeTruthy();
    const [saved] = JSON.parse(localStorage.getItem(MACRO_STORAGE_KEY)!) as Macro[];
    expect(saved.steps.map((step) => [step.op, step.params])).toEqual([
      ['core.layer.set_visible', { selector: { type: 'name', name: 'Background' }, visible: false }],
      ['core.layer.set_visible', { selector: { type: 'name', name: 'Background' }, visible: false }]
    ]);
    expect(within(dialog).queryByRole('alert', { name: 'Problems with this macro' })).toBeNull();
    // Recording does not touch the backend: nothing was run to make it.
    expect(calls('apply_transaction')).toHaveLength(0);
  });

  it('records an adjustment layer as the registered operation that makes it', async () => {
    render(App);
    await openImage();
    await runPaletteCommand('record a macro');
    await screen.findByText('Recording a macro.');
    await fireEvent.click(screen.getByRole('button', { name: 'New adjustment layer' }));
    await fireEvent.click(await screen.findByRole('button', { name: 'Create layer' }));
    await fireEvent.click(await screen.findByRole('button', { name: 'Stop recording' }));
    const dialog = await screen.findByRole('dialog', { name: 'Macros' });
    const [saved] = JSON.parse(localStorage.getItem(MACRO_STORAGE_KEY)!) as Macro[];
    expect(saved.steps).toHaveLength(1);
    expect(saved.steps[0].op).toBe('core.layer.add_adjustment');
    expect(saved.steps[0].params.operation).toMatchObject({ type: expect.any(String) });
    expect(typeof saved.steps[0].params.name).toBe('string');
    // What was recorded is something the registry accepts, so it can be run.
    expect(within(dialog).queryByRole('alert', { name: 'Problems with this macro' })).toBeNull();
    expect(within(dialog).getByRole('button', { name: 'Run' })).toBeTruthy();
  });

  it('can be discarded, leaving no macro behind', async () => {
    render(App);
    await openImage();
    await runPaletteCommand('record a macro');
    await fireEvent.click(await screen.findByRole('button', { name: 'Hide Background' }));
    await fireEvent.click(screen.getByRole('button', { name: 'Discard' }));
    await waitFor(() => expect(screen.queryByText('Recording a macro.')).toBeNull());
    expect(localStorage.getItem(MACRO_STORAGE_KEY)).toBeNull();
  });

  it('offers a run command for each saved macro in the palette, and it runs as an automation transaction', async () => {
    seed(hideBackgroundMacro('Hide it'));
    render(App);
    await openImage();
    await fireEvent.keyDown(window, { key: 'P', ctrlKey: true, shiftKey: true });
    const input = await screen.findByRole('combobox', { name: 'Search commands' });
    await fireEvent.input(input, { target: { value: 'Hide it' } });
    await fireEvent.click(await screen.findByRole('option', { name: /Run macro: Hide it/ }));
    await waitFor(() => expect(calls('apply_transaction')).toHaveLength(1));
    expect(requestOf('apply_transaction').origin).toBe('automation');
    await waitFor(() => expect(screen.getByRole('button', { name: 'Show Background' })).toBeTruthy());
  });

  it('does not offer to run a macro while recording', async () => {
    seed(hideBackgroundMacro('Hide it'));
    render(App);
    await openImage();
    await runPaletteCommand('record a macro');
    await screen.findByText('Recording a macro.');
    await fireEvent.keyDown(window, { key: 'P', ctrlKey: true, shiftKey: true });
    const input = await screen.findByRole('combobox', { name: 'Search commands' });
    await fireEvent.input(input, { target: { value: 'Hide it' } });
    const row = await screen.findByRole('option', { name: /Run macro: Hide it/ });
    expect(row.getAttribute('aria-disabled')).toBe('true');
    expect(row.textContent).toMatch(/Stop recording first/);
  });
});

describe('what a macro file looks like', () => {
  it('is the document the editor exports', () => {
    const macro = hideBackgroundMacro();
    expect(macroDocument(macro)).toEqual({ schemaVersion: 1, kind: 'photoforge-macro', macro });
  });
});
