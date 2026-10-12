/**
 * Opening and writing a TIFF: the file goes to its own command (not image admission, not another
 * importer), comes back as an unsaved document with the report the other formats use, and the
 * export dialog offers TIFF and writes it by the same path as the other flattened images.
 */
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { fireEvent, render, screen, waitFor, within } from '@testing-library/svelte';
import { invoke } from '@tauri-apps/api/core';
import { open, save } from '@tauri-apps/plugin-dialog';
import App from './App.svelte';
import registry from '../src-tauri/tests/fixtures/operation_registry.json';
import { createDocument, createPixelLayer } from './lib/layers/tree';
import type { OperationSpec } from './lib/operations/types';
import type { TiffImportResult } from './lib/layers/tiff';
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

const TIFF = String.raw`C:\scans\page.tif`;
const previewUrl = 'data:image/png;base64,b3JpZ2luYWw=';
const calls = (command: string) => vi.mocked(invoke).mock.calls.filter(([name]) => name === command);

let entries: TiffImportResult['report']['entries'] = [];
let failImport: string | null = null;

function importResult(): TiffImportResult {
  const picture = createPixelLayer('Background', 'px1', 16, 12);
  const document = createDocument(16, 12, [picture], 'linear_srgb_f32');
  return {
    project: {
      documentId: 9002,
      isCurrent: true,
      metadata: {
        filename: 'page.tif', width: 16, height: 12, format: 'TIFF', fileSize: 4096,
        colorSpace: 'Linear sRGB', bitDepth: 16, hasAlpha: false, createdAt: null, modifiedAt: null,
        cameraModel: null, exifAvailable: false
      },
      originalPreviewDataUrl: previewUrl,
      previewDataUrl: previewUrl,
      document,
      operations: [],
      canvasWidth: 16,
      canvasHeight: 12,
      applicationVersion: '0.14.0',
      createdAt: '2026-10-12T00:00:00.000Z',
      modifiedAt: '2026-10-12T00:00:00.000Z',
      processingTimeMs: 3,
      pluginRequirements: []
    } as unknown as TiffImportResult['project'],
    report: {
      format: 'TIFF', width: 16, height: 12, bitDepth: 16, colorMode: 'RGB',
      layersImported: 1, groupsImported: 0, entries
    }
  };
}

beforeEach(() => {
  localStorage.clear();
  entries = [];
  failImport = null;
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
      case 'import_tiff_document':
        if (failImport) throw new Error(failImport);
        return importResult();
      case 'export_layer_composite':
        return { outputPath: args.outputPath, width: 16, height: 12, processingTimeMs: 5, metadata: { policy: 'remove_private', notes: [] } };
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

describe('opening a TIFF', () => {
  it('offers TIFF in the Open dialog, among the images and on its own', async () => {
    render(App);
    await chooseFile(TIFF);
    await screen.findByRole('dialog', { name: /page\.tif/ });
    const options = vi.mocked(open).mock.calls[0][0] as { filters: { name: string; extensions: string[] }[] };
    expect(options.filters[0].extensions).toEqual(expect.arrayContaining(['tif', 'tiff']));
    expect(options.filters.some((filter) => filter.name.startsWith('TIFF'))).toBe(true);
  });

  it('sends the file to its own command, not image admission or the other importers', async () => {
    render(App);
    await chooseFile(TIFF);
    await screen.findByRole('dialog', { name: /page\.tif/ });
    expect(calls('import_tiff_document')).toHaveLength(1);
    expect(calls('import_tiff_document')[0][1]).toMatchObject({ path: TIFF });
    expect(calls('import_psd_document')).toHaveLength(0);
    expect(calls('import_exr_document')).toHaveLength(0);
    expect(calls('probe_image_source')).toHaveLength(0);
    expect(calls('open_image')).toHaveLength(0);
    expect(await screen.findByRole('button', { name: 'Select Background' })).toBeTruthy();
  });

  it('says it is a TIFF import, with its depth, and that nothing was left out when so', async () => {
    render(App);
    await chooseFile(TIFF);
    const dialog = await screen.findByRole('dialog', { name: /page\.tif/ });
    expect(within(dialog).getByText('TIFF import')).toBeTruthy();
    expect(within(dialog).getByTestId('psd-summary').textContent).toMatch(
      /TIFF,\s+16\s+×\s+12,\s+16-bit\s+RGB\.\s+1\s+layer\s+imported/
    );
    expect(within(dialog).getByTestId('psd-clean')).toBeTruthy();
    expect(await screen.findByText(/page\.tif opened; nothing was approximated or left out/)).toBeTruthy();
  });

  it('lists what was not imported, in the TIFF wording', async () => {
    entries = [
      { kind: 'not_imported', layer: null, detail: '2 other images in the file were not imported.' },
      { kind: 'approximated', layer: null, detail: 'The embedded profile could not be read, so the colour was taken as sRGB.' },
      { kind: 'note', layer: null, detail: '16-bit without a profile, read as sRGB, as a high-precision document' }
    ];
    render(App);
    await chooseFile(TIFF);
    const dialog = await screen.findByRole('dialog', { name: /page\.tif/ });
    const left = within(dialog).getByRole('region', { name: 'Left out' });
    expect(within(left).getByText(/2 other images/)).toBeTruthy();
    expect(within(dialog).getByRole('region', { name: 'Imported, but not exactly as in the TIFF file' })).toBeTruthy();
    expect(await screen.findByText(/1 approximated and 1 left out/)).toBeTruthy();
  });

  it('leaves the document unsaved: Save asks for a project name, not the TIFF', async () => {
    render(App);
    await chooseFile(TIFF);
    const dialog = await screen.findByRole('dialog', { name: /page\.tif/ });
    await fireEvent.click(within(dialog).getByRole('button', { name: 'Close' }));
    await waitFor(() => expect(screen.queryByRole('dialog', { name: /page\.tif/ })).toBeNull());
    vi.mocked(save).mockResolvedValueOnce(null);
    await fireEvent.click(screen.getByRole('button', { name: /^Save project/ }));
    await waitFor(() => expect(save).toHaveBeenCalledTimes(1));
    const options = vi.mocked(save).mock.calls[0][0] as { defaultPath: string; filters: { extensions: string[] }[] };
    expect(options.defaultPath).toBe('page.photoforge');
    expect(options.filters[0].extensions).toEqual(['photoforge']);
  });

  it('shows the reason when the file cannot be read, and opens nothing', async () => {
    failImport = 'This picture is stored as CMYK (separated colour), which is not read.';
    render(App);
    await chooseFile(TIFF);
    expect(await screen.findByText(/stored as CMYK/)).toBeTruthy();
    expect(screen.queryByRole('dialog', { name: /page\.tif/ })).toBeNull();
    expect(screen.queryByRole('button', { name: 'Select Background' })).toBeNull();
  });
});

describe('writing a TIFF', () => {
  async function openAndClose() {
    render(App);
    await chooseFile(TIFF);
    const dialog = await screen.findByRole('dialog', { name: /page\.tif/ });
    await fireEvent.click(within(dialog).getByRole('button', { name: 'Close' }));
    await waitFor(() => expect(screen.queryByRole('dialog', { name: /page\.tif/ })).toBeNull());
  }

  async function exportTo(path: string | null) {
    vi.mocked(save).mockResolvedValueOnce(path);
    await fireEvent.click(screen.getByRole('button', { name: 'Export' }));
  }

  it('offers TIFF in the Export dialog', async () => {
    await openAndClose();
    await exportTo(String.raw`C:\scans\flat.png`);
    await waitFor(() => expect(save).toHaveBeenCalledTimes(1));
    const options = vi.mocked(save).mock.calls[0][0] as { filters: { name: string; extensions: string[] }[] };
    const tiff = options.filters.find((filter) => filter.extensions.includes('tif'));
    expect(tiff?.name).toMatch(/TIFF/);
    expect(tiff?.extensions).toEqual(['tif', 'tiff']);
  });

  it('writes a TIFF by the same command, profile, colour and metadata choices as a PNG', async () => {
    await openAndClose();
    await exportTo(String.raw`C:\scans\out.tif`);
    await waitFor(() => expect(calls('export_layer_composite')).toHaveLength(1));
    const [, args] = calls('export_layer_composite')[0] as [string, Record<string, any>];
    expect(args.outputPath).toBe(String.raw`C:\scans\out.tif`);
    expect(args.document.layers).toHaveLength(1);
    expect(args.color).toMatchObject({ colorSpace: expect.any(String) });
    expect(args.metadataPolicy).toBe('remove_private');
    expect(calls('export_psd_document')).toHaveLength(0);
    expect(calls('export_exr_document')).toHaveLength(0);
    expect(await screen.findByText(/Exported 16 × 12 image/)).toBeTruthy();
  });
});
