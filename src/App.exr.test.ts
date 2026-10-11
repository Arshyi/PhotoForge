/**
 * Opening and writing an OpenEXR file: the file goes to its own commands (not image admission, not
 * the Photoshop importer), comes back as an unsaved high-precision document, and what was
 * approximated or left out is shown in the same report dialog the other formats use.
 */
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { fireEvent, render, screen, waitFor, within } from '@testing-library/svelte';
import { invoke } from '@tauri-apps/api/core';
import { open, save } from '@tauri-apps/plugin-dialog';
import App from './App.svelte';
import registry from '../src-tauri/tests/fixtures/operation_registry.json';
import { createDocument, createPixelLayer } from './lib/layers/tree';
import type { OperationSpec } from './lib/operations/types';
import type { ExrExportResult, ExrImportResult } from './lib/layers/exr';
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

const EXR = String.raw`C:\renders\shot.exr`;
const previewUrl = 'data:image/png;base64,b3JpZ2luYWw=';
const calls = (command: string) => vi.mocked(invoke).mock.calls.filter(([name]) => name === command);

let entries: ExrImportResult['report']['entries'] = [];
let failImport: string | null = null;
let failExport: string | null = null;
let exportEntries: ExrExportResult['report']['entries'] = [];

function importResult(): ExrImportResult {
  const picture = createPixelLayer('Image', 'px1', 16, 12);
  const document = createDocument(16, 12, [picture], 'linear_srgb_f32');
  return {
    project: {
      documentId: 9001,
      isCurrent: true,
      metadata: {
        filename: 'shot.exr', width: 16, height: 12, format: 'OpenEXR', fileSize: 4096,
        colorSpace: 'Linear sRGB', bitDepth: 32, hasAlpha: true, createdAt: null, modifiedAt: null,
        cameraModel: null, exifAvailable: false
      },
      originalPreviewDataUrl: previewUrl,
      previewDataUrl: previewUrl,
      document,
      operations: [],
      canvasWidth: 16,
      canvasHeight: 12,
      applicationVersion: '0.14.0',
      createdAt: '2026-10-10T00:00:00.000Z',
      modifiedAt: '2026-10-10T00:00:00.000Z',
      processingTimeMs: 3,
      pluginRequirements: []
    } as unknown as ExrImportResult['project'],
    report: {
      format: 'OpenEXR', width: 16, height: 12, bitDepth: 32, colorMode: 'RGB',
      layersImported: 1, groupsImported: 0, entries
    }
  };
}

function exportResult(outputPath: string): ExrExportResult {
  return {
    outputPath,
    processingTimeMs: 9,
    report: {
      format: 'OpenEXR', width: 16, height: 12, bitDepth: 32, colorMode: 'RGBA',
      layersWritten: 1, groupsWritten: 0, bytes: 8192, entries: exportEntries
    },
    metadata: { policy: 'remove_private', notes: [] }
  } as ExrExportResult;
}

beforeEach(() => {
  localStorage.clear();
  entries = [];
  exportEntries = [];
  failImport = null;
  failExport = null;
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
      case 'list_plugins': return statusFixture([]);
      case 'document_plugin_status': return [];
      case 'list_operations': return registry as unknown as OperationSpec[];
      case 'import_exr_document':
        if (failImport) throw new Error(failImport);
        return importResult();
      case 'export_exr_document':
        if (failExport) throw new Error(failExport);
        return exportResult(args.outputPath);
      case 'export_layer_composite':
        return { outputPath: args.outputPath, width: 16, height: 12, processingTimeMs: 5 };
      case 'render_layer_composite':
      case 'render_preview':
        return { requestId: args.requestId, isCurrent: true, previewDataUrl: previewUrl, processingTime: 1, processingTimeMs: 1, operationCount: 0, missingPlugins: [] };
      case 'render_layer_thumbnail': return { layerId: args.layerId, previewDataUrl: previewUrl, width: 16, height: 12 };
      case 'list_recovery_snapshots': return { snapshots: [] };
      case 'analyze_image': return { isCurrent: false };
      case 'get_component_snapshot':
        return { configuration: { activePlanner: 'rule', ollamaSelectedModel: null, ollamaMaxOperations: 8 } };
      case 'get_ollama_diagnostics': return { connected: false };
      default: return {};
    }
  });
});

afterEach(() => vi.restoreAllMocks());

async function chooseFile(path: string) {
  vi.mocked(open).mockResolvedValueOnce(path);
  await fireEvent.click(screen.getByRole('button', { name: 'Open' }));
}

describe('opening an OpenEXR file', () => {
  it('offers OpenEXR in the Open dialog', async () => {
    render(App);
    await chooseFile(EXR);
    await screen.findByRole('dialog', { name: /shot\.exr/ });
    const options = vi.mocked(open).mock.calls[0][0] as { filters: { name: string; extensions: string[] }[] };
    expect(options.filters.some((filter) => filter.extensions.includes('exr'))).toBe(true);
  });

  it('sends the file to its own command, not image admission or the other importers', async () => {
    render(App);
    await chooseFile(EXR);
    await screen.findByRole('dialog', { name: /shot\.exr/ });
    expect(calls('import_exr_document')).toHaveLength(1);
    expect(calls('import_exr_document')[0][1]).toMatchObject({ path: EXR });
    expect(calls('import_psd_document')).toHaveLength(0);
    expect(calls('import_svg_document')).toHaveLength(0);
    expect(calls('probe_image_source')).toHaveLength(0);
    expect(calls('open_image')).toHaveLength(0);
    expect(await screen.findByRole('button', { name: 'Select Image' })).toBeTruthy();
  });

  it('says it is an OpenEXR import, with its depth, and that nothing was left out when so', async () => {
    render(App);
    await chooseFile(EXR);
    const dialog = await screen.findByRole('dialog', { name: /shot\.exr/ });
    expect(within(dialog).getByText('OpenEXR import')).toBeTruthy();
    expect(within(dialog).getByTestId('psd-summary').textContent).toMatch(
      /OpenEXR,\s+16\s+×\s+12,\s+32-bit\s+RGB\.\s+1\s+layer\s+imported/
    );
    expect(within(dialog).getByTestId('psd-clean')).toBeTruthy();
    expect(within(dialog).getByText(/The original file is not changed/)).toBeTruthy();
    expect(await screen.findByText(/shot\.exr opened with 1 layer; nothing was approximated or left out/)).toBeTruthy();
  });

  it('lists what was not read, in the OpenEXR wording', async () => {
    entries = [
      { kind: 'not_imported', layer: null, detail: '3 other channels not read as colour: Z, id, mask.' },
      { kind: 'approximated', layer: null, detail: '4 pixels held a value that is not a number or is infinite.' },
      { kind: 'note', layer: null, detail: 'Read as scene-linear light with premultiplied alpha.' }
    ];
    render(App);
    await chooseFile(EXR);
    const dialog = await screen.findByRole('dialog', { name: /shot\.exr/ });
    const left = within(dialog).getByRole('region', { name: 'Left out' });
    expect(within(left).getByText(/3 other channels/)).toBeTruthy();
    expect(within(dialog).getByRole('region', { name: 'Imported, but not exactly as in the OpenEXR file' })).toBeTruthy();
    expect(within(dialog).getByRole('region', { name: 'Notes' })).toBeTruthy();
    expect(await screen.findByText(/1 approximated and 1 left out/)).toBeTruthy();
  });

  it('closes the report and leaves the document open and unsaved', async () => {
    render(App);
    await chooseFile(EXR);
    const dialog = await screen.findByRole('dialog', { name: /shot\.exr/ });
    await fireEvent.click(within(dialog).getByRole('button', { name: 'Close' }));
    await waitFor(() => expect(screen.queryByRole('dialog', { name: /shot\.exr/ })).toBeNull());
    vi.mocked(save).mockResolvedValueOnce(null);
    await fireEvent.click(screen.getByRole('button', { name: /^Save project/ }));
    await waitFor(() => expect(save).toHaveBeenCalledTimes(1));
    const options = vi.mocked(save).mock.calls[0][0] as { defaultPath: string; filters: { extensions: string[] }[] };
    expect(options.defaultPath).toBe('shot.photoforge');
    expect(options.filters[0].extensions).toEqual(['photoforge']);
  });

  it('shows the engine’s reason when the file cannot be read, and opens nothing', async () => {
    failImport = 'Part 1 is compressed with DWAA, which PhotoForge cannot read yet. Save the file again with ZIP, PIZ or no compression.';
    render(App);
    await chooseFile(EXR);
    expect(await screen.findByText(/compressed with DWAA/)).toBeTruthy();
    expect(screen.queryByRole('dialog', { name: /shot\.exr/ })).toBeNull();
    expect(screen.queryByRole('button', { name: 'Select Image' })).toBeNull();
  });
});

describe('writing an OpenEXR file', () => {
  async function openAndClose() {
    render(App);
    await chooseFile(EXR);
    const dialog = await screen.findByRole('dialog', { name: /shot\.exr/ });
    await fireEvent.click(within(dialog).getByRole('button', { name: 'Close' }));
    await waitFor(() => expect(screen.queryByRole('dialog', { name: /shot\.exr/ })).toBeNull());
  }

  async function exportTo(path: string | null) {
    vi.mocked(save).mockResolvedValueOnce(path);
    await fireEvent.click(screen.getByRole('button', { name: 'Export' }));
  }

  it('offers OpenEXR in the Export dialog', async () => {
    await openAndClose();
    await exportTo(String.raw`C:\renders\flat.png`);
    await waitFor(() => expect(save).toHaveBeenCalledTimes(1));
    const options = vi.mocked(save).mock.calls[0][0] as { filters: { name: string; extensions: string[] }[] };
    const exr = options.filters.find((filter) => filter.extensions.includes('exr'));
    expect(exr?.name).toMatch(/OpenEXR/);
  });

  it('writes the composite to an EXR name through its own command and report', async () => {
    await openAndClose();
    await exportTo(String.raw`C:\renders\out.exr`);
    const dialog = await screen.findByRole('dialog', { name: /out\.exr/ });
    expect(calls('export_exr_document')).toHaveLength(1);
    const [, args] = calls('export_exr_document')[0] as [string, Record<string, any>];
    expect(args.outputPath).toBe(String.raw`C:\renders\out.exr`);
    expect(args.document.layers).toHaveLength(1);
    expect(args.operations).toEqual([]);
    // Single precision unless asked otherwise; the profile and colour choices do not apply.
    expect(args.bitDepth).toBeUndefined();
    expect(args.metadataPolicy).toBe('remove_private');
    expect(calls('export_layer_composite')).toHaveLength(0);
    expect(calls('export_psd_document')).toHaveLength(0);
    expect(within(dialog).getByText('OpenEXR export')).toBeTruthy();
    expect(within(dialog).getByTestId('psd-summary').textContent).toMatch(
      /OpenEXR,\s+16\s+×\s+12,\s+32-bit\s+RGBA\.\s+1\s+layer\s+written/
    );
    expect(within(dialog).getByText(/one picture of scene-linear/)).toBeTruthy();
    expect(await screen.findByText(/out\.exr written as one single precision picture; nothing was approximated/)).toBeTruthy();
  });

  it('says that the layers were composited', async () => {
    exportEntries = [
      { kind: 'not_exported', layer: null, detail: 'The document’s 3 layers were composited into one picture.' }
    ];
    await openAndClose();
    await exportTo(String.raw`C:\renders\out.exr`);
    const dialog = await screen.findByRole('dialog', { name: /out\.exr/ });
    const left = within(dialog).getByRole('region', { name: 'Left out of the file' });
    expect(within(left).getByText(/composited into one picture/)).toBeTruthy();
    expect(await screen.findByText(/1 left out/)).toBeTruthy();
  });

  it('shows the reason when the file cannot be written, and leaves the document alone', async () => {
    failExport = 'The output path is not an OpenEXR file name.';
    await openAndClose();
    await exportTo(String.raw`C:\renders\out.exr`);
    expect(await screen.findByText(/not an OpenEXR file name/)).toBeTruthy();
    expect(screen.queryByRole('dialog', { name: /out\.exr/ })).toBeNull();
    expect(screen.getByRole('button', { name: 'Select Image' })).toBeTruthy();
  });

  it('still writes flattened images through the image path', async () => {
    await openAndClose();
    await exportTo(String.raw`C:\renders\flat.png`);
    await waitFor(() => expect(calls('export_layer_composite')).toHaveLength(1));
    expect(calls('export_exr_document')).toHaveLength(0);
  });
});
