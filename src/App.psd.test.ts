/**
 * Opening a Photoshop document: the file goes to its own command (not image admission), comes
 * back as an unsaved layered document, and what was approximated or left out is shown.
 */
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { fireEvent, render, screen, waitFor, within } from '@testing-library/svelte';
import { invoke } from '@tauri-apps/api/core';
import { open, save } from '@tauri-apps/plugin-dialog';
import App from './App.svelte';
import registry from '../src-tauri/tests/fixtures/operation_registry.json';
import { createDocument, createPixelLayer } from './lib/layers/tree';
import type { OperationSpec } from './lib/operations/types';
import type { PsdExportResult, PsdImportResult } from './lib/layers/psd';
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

const PSD = String.raw`C:\work\poster.psd`;
const previewUrl = 'data:image/png;base64,b3JpZ2luYWw=';
const calls = (command: string) => vi.mocked(invoke).mock.calls.filter(([name]) => name === command);

let importResult: () => PsdImportResult;
let failImport: string | null;
let exportResult: () => PsdExportResult;
let failExport: string | null;

function result(entries: PsdImportResult['report']['entries']): PsdImportResult {
  const sky = createPixelLayer('Sky', 'px1', 16, 12);
  const sun = createPixelLayer('Sun', 'px2', 6, 4);
  const document = createDocument(16, 12, [sky, sun], 'legacy_srgb8');
  return {
    project: {
      documentId: 7001,
      isCurrent: true,
      metadata: {
        filename: 'poster.psd', width: 16, height: 12, format: 'PSD', fileSize: 1024,
        colorSpace: 'sRGB', bitDepth: 8, hasAlpha: true, createdAt: null, modifiedAt: null,
        cameraModel: null, exifAvailable: false
      },
      originalPreviewDataUrl: previewUrl,
      previewDataUrl: previewUrl,
      document,
      operations: [],
      canvasWidth: 16,
      canvasHeight: 12,
      applicationVersion: '0.14.0',
      createdAt: '2026-10-09T00:00:00.000Z',
      modifiedAt: '2026-10-09T00:00:00.000Z',
      processingTimeMs: 3,
      pluginRequirements: []
    } as unknown as PsdImportResult['project'],
    report: {
      format: 'PSD', width: 16, height: 12, bitDepth: 8, colorMode: 'RGB',
      layersImported: 2, groupsImported: 0, entries
    }
  };
}

beforeEach(() => {
  localStorage.clear();
  failImport = null;
  failExport = null;
  importResult = () => result([]);
  exportResult = () => ({
    outputPath: String.raw`C:\work\out.psd`,
    processingTimeMs: 12,
    report: {
      format: 'PSD', width: 16, height: 12, bitDepth: 8, colorMode: 'RGB',
      layersWritten: 2, groupsWritten: 0, bytes: 2048, entries: []
    }
  });
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
      case 'import_psd_document':
        if (failImport) throw new Error(failImport);
        return importResult();
      case 'export_layer_composite':
        return { outputPath: args.outputPath, width: 16, height: 12, processingTimeMs: 5 };
      case 'export_psd_document':
        if (failExport) throw new Error(failExport);
        return exportResult();
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

describe('opening a Photoshop document', () => {
  it('offers Photoshop files in the Open dialog', async () => {
    render(App);
    await chooseFile(PSD);
    await screen.findByRole('dialog', { name: /poster\.psd/ });
    const options = vi.mocked(open).mock.calls[0][0] as { filters: { name: string; extensions: string[] }[] };
    const photoshop = options.filters.find((filter) => filter.extensions.includes('psd'));
    expect(photoshop?.extensions).toEqual(['psd', 'psb']);
  });

  it('sends the file to the import command, not to image admission, and shows its layers', async () => {
    render(App);
    await chooseFile(PSD);
    await screen.findByRole('dialog', { name: /poster\.psd/ });
    expect(calls('import_psd_document')).toHaveLength(1);
    expect(calls('import_psd_document')[0][1]).toMatchObject({ path: PSD });
    expect(calls('probe_image_source')).toHaveLength(0);
    expect(calls('open_image')).toHaveLength(0);
    expect(await screen.findByRole('button', { name: 'Select Sky' })).toBeTruthy();
    expect(screen.getByRole('button', { name: 'Select Sun' })).toBeTruthy();
  });

  it('says plainly when nothing was approximated or left out', async () => {
    render(App);
    await chooseFile(PSD);
    const dialog = await screen.findByRole('dialog', { name: /poster\.psd/ });
    expect(within(dialog).getByTestId('psd-clean').textContent).toMatch(/Nothing was approximated or left out/);
    expect(within(dialog).queryByRole('region', { name: 'Left out' })).toBeNull();
  });

  it('lists what was left out and what was approximated, each with its layer', async () => {
    importResult = () =>
      result([
        { kind: 'not_imported', layer: 'Glow', detail: 'Layer styles were not imported.' },
        { kind: 'approximated', layer: 'Title', detail: 'Text layer imported as pixels.' },
        { kind: 'note', layer: null, detail: '8-bit sRGB' }
      ]);
    render(App);
    await chooseFile(PSD);
    const dialog = await screen.findByRole('dialog', { name: /poster\.psd/ });
    const left = within(dialog).getByRole('region', { name: 'Left out' });
    expect(within(left).getByText('Glow')).toBeTruthy();
    expect(within(left).getByText('Layer styles were not imported.')).toBeTruthy();
    const approximated = within(dialog).getByRole('region', {
      name: 'Imported, but not exactly as in Photoshop'
    });
    expect(within(approximated).getByText('Title')).toBeTruthy();
    expect(within(dialog).queryByTestId('psd-clean')).toBeNull();
    // The notification says it too, in one sentence.
    expect(await screen.findByText(/1 approximated and 1 left out/)).toBeTruthy();
  });

  it('says a repeated finding once, with how many layers it covers', async () => {
    importResult = () =>
      result([
        { kind: 'approximated', layer: 'Rectangle 1', detail: 'Fill layer imported as its rendered pixels.' },
        { kind: 'approximated', layer: 'Rectangle 1', detail: 'Fill layer imported as its rendered pixels.' },
        { kind: 'approximated', layer: 'Rectangle 1', detail: 'Fill layer imported as its rendered pixels.' },
        { kind: 'approximated', layer: 'Logo', detail: 'Fill layer imported as its rendered pixels.' }
      ]);
    render(App);
    await chooseFile(PSD);
    const dialog = await screen.findByRole('dialog', { name: /poster\.psd/ });
    const region = within(dialog).getByRole('region', { name: 'Imported, but not exactly as in Photoshop' });
    // Four findings, three lines: the count in the heading is of findings.
    expect(within(region).getAllByRole('listitem')).toHaveLength(2);
    expect(within(region).getByText('on 3 layers with this name')).toBeTruthy();
    expect(within(region).getByRole('heading', { level: 2 }).textContent).toMatch(/4/);
  });

  it('closes the report and leaves the document open and unsaved', async () => {
    render(App);
    await chooseFile(PSD);
    const dialog = await screen.findByRole('dialog', { name: /poster\.psd/ });
    await fireEvent.click(within(dialog).getByRole('button', { name: 'Close' }));
    await waitFor(() => expect(screen.queryByRole('dialog', { name: /poster\.psd/ })).toBeNull());
    expect(screen.getByRole('button', { name: 'Select Sky' })).toBeTruthy();

    // Saving asks where the PhotoForge project goes; it does not offer the Photoshop file.
    vi.mocked(save).mockResolvedValueOnce(null);
    await fireEvent.click(screen.getByRole('button', { name: /^Save project/ }));
    await waitFor(() => expect(save).toHaveBeenCalledTimes(1));
    const options = vi.mocked(save).mock.calls[0][0] as { defaultPath: string; filters: { extensions: string[] }[] };
    expect(options.defaultPath).toBe('poster.photoforge');
    expect(options.filters[0].extensions).toEqual(['photoforge']);
  });

  it('shows the engine’s reason when the file cannot be imported, and opens nothing', async () => {
    failImport =
      'This file is in CMYK colour mode, which PhotoForge cannot import yet. In Photoshop, choose Image > Mode > RGB Color and save it again.';
    render(App);
    await chooseFile(PSD);
    expect(await screen.findByText(/CMYK colour mode/)).toBeTruthy();
    expect(screen.queryByRole('dialog', { name: /poster\.psd/ })).toBeNull();
    expect(screen.queryByRole('button', { name: 'Select Sky' })).toBeNull();
  });
});

describe('writing a Photoshop document', () => {
  async function openAndClose() {
    render(App);
    await chooseFile(PSD);
    const dialog = await screen.findByRole('dialog', { name: /poster\.psd/ });
    await fireEvent.click(within(dialog).getByRole('button', { name: 'Close' }));
    await waitFor(() => expect(screen.queryByRole('dialog', { name: /poster\.psd/ })).toBeNull());
  }

  async function exportTo(path: string | null) {
    vi.mocked(save).mockResolvedValueOnce(path);
    await fireEvent.click(screen.getByRole('button', { name: 'Export' }));
  }

  it('offers Photoshop files in the Export dialog', async () => {
    await openAndClose();
    await exportTo(null);
    await waitFor(() => expect(save).toHaveBeenCalledTimes(1));
    const options = vi.mocked(save).mock.calls[0][0] as { filters: { name: string; extensions: string[] }[] };
    const photoshop = options.filters.find((filter) => filter.extensions.includes('psd'));
    expect(photoshop?.extensions).toEqual(['psd', 'psb']);
    expect(calls('export_psd_document')).toHaveLength(0);
  });

  it('writes the layers to a Photoshop name, and shows what the file holds', async () => {
    await openAndClose();
    await exportTo(String.raw`C:\work\out.psd`);
    const dialog = await screen.findByRole('dialog', { name: /out\.psd/ });
    expect(calls('export_psd_document')).toHaveLength(1);
    const [, args] = calls('export_psd_document')[0] as [string, Record<string, any>];
    expect(args.outputPath).toBe(String.raw`C:\work\out.psd`);
    expect(args.document.layers).toHaveLength(2);
    expect(args.operations).toEqual([]);
    expect(args.bitDepth).toBeNull();
    // Not the flattened-image path.
    expect(calls('export_layer_composite')).toHaveLength(0);
    expect(within(dialog).getByText('Photoshop export')).toBeTruthy();
    expect(within(dialog).getByTestId('psd-summary').textContent).toMatch(/2\s+layers\s+written/);
    expect(within(dialog).getByTestId('psd-clean').textContent).toMatch(/Nothing was approximated or left out/);
    expect(within(dialog).getByText(/Your document in PhotoForge is not changed/)).toBeTruthy();
  });

  it('lists what was left out of the file and what was written approximately', async () => {
    exportResult = () => ({
      outputPath: String.raw`C:\work\out.psd`,
      processingTimeMs: 12,
      report: {
        format: 'PSD', width: 16, height: 12, bitDepth: 16, colorMode: 'RGB',
        layersWritten: 1, groupsWritten: 1, bytes: 4096,
        entries: [
          { kind: 'not_exported', layer: 'Soften', detail: 'A gaussian blur adjustment layer has no Photoshop equivalent.' },
          { kind: 'approximated', layer: 'Title', detail: 'Text layer written as the pixels it draws.' }
        ]
      }
    });
    await openAndClose();
    await exportTo(String.raw`C:\work\out.psd`);
    const dialog = await screen.findByRole('dialog', { name: /out\.psd/ });
    const left = within(dialog).getByRole('region', { name: 'Left out of the file' });
    expect(within(left).getByText('Soften')).toBeTruthy();
    expect(within(dialog).getByRole('region', { name: 'Imported, but not exactly as in Photoshop' })).toBeTruthy();
    expect(within(dialog).getByTestId('psd-summary').textContent).toMatch(/1\s+layer\s+and\s+1\s+group\s+written/);
    expect(await screen.findByText(/1 approximated and 1 left out/)).toBeTruthy();
  });

  it('names the file the engine wrote, which may not be the one asked for', async () => {
    exportResult = () => ({
      outputPath: String.raw`C:\work\huge.psb`,
      processingTimeMs: 12,
      report: {
        format: 'PSB', width: 16, height: 12, bitDepth: 8, colorMode: 'RGB',
        layersWritten: 2, groupsWritten: 0, bytes: 2048, entries: []
      }
    });
    await openAndClose();
    await exportTo(String.raw`C:\work\huge.psd`);
    expect(await screen.findByRole('dialog', { name: /huge\.psb/ })).toBeTruthy();
  });

  it('shows the reason when the file cannot be written, and leaves the document alone', async () => {
    failExport = 'The output path is not a Photoshop file name.';
    await openAndClose();
    await exportTo(String.raw`C:\work\out.psd`);
    expect(await screen.findByText(/not a Photoshop file name/)).toBeTruthy();
    expect(screen.queryByRole('dialog', { name: /out\.psd/ })).toBeNull();
    expect(screen.getByRole('button', { name: 'Select Sky' })).toBeTruthy();
  });

  it('still writes flattened images through the image path', async () => {
    await openAndClose();
    await exportTo(String.raw`C:\work\flat.png`);
    await waitFor(() => expect(calls('export_layer_composite')).toHaveLength(1));
    expect(calls('export_psd_document')).toHaveLength(0);
  });

  it('sends the metadata policy with both kinds of export, private by default', async () => {
    await openAndClose();
    await exportTo(String.raw`C:\work\out.psd`);
    await screen.findByRole('dialog', { name: /out\.psd/ });
    const [, psd] = calls('export_psd_document')[0] as [string, Record<string, any>];
    expect(psd.metadataPolicy).toBe('remove_private');
    await exportTo(String.raw`C:\work\flat.png`);
    await waitFor(() => expect(calls('export_layer_composite')).toHaveLength(1));
    const [, flat] = calls('export_layer_composite')[0] as [string, Record<string, any>];
    expect(flat.metadataPolicy).toBe('remove_private');
  });

  it('remembers a chosen policy and sends it', async () => {
    await openAndClose();
    const select = screen.getByLabelText('Metadata on export') as HTMLSelectElement;
    await fireEvent.change(select, { target: { value: 'keep_all' } });
    expect(localStorage.getItem('photoforge.metadataPolicy')).toBe('keep_all');
    await exportTo(String.raw`C:\work\flat.png`);
    await waitFor(() => expect(calls('export_layer_composite')).toHaveLength(1));
    const [, flat] = calls('export_layer_composite')[0] as [string, Record<string, any>];
    expect(flat.metadataPolicy).toBe('keep_all');
  });

  it('says in the notice what an export removed', async () => {
    exportResult = () => ({
      outputPath: String.raw`C:\work\out.psd`,
      processingTimeMs: 12,
      report: {
        format: 'PSD', width: 16, height: 12, bitDepth: 8, colorMode: 'RGB',
        layersWritten: 2, groupsWritten: 0, bytes: 2048, entries: []
      },
      metadata: {
        policy: 'remove_private',
        notes: [
          { kind: 'kept', detail: 'EXIF, Acme Cam-One' },
          { kind: 'removed', detail: 'the place the picture was taken (GPS)' },
          { kind: 'removed', detail: 'the camera owner name and serial numbers' },
          { kind: 'removed', detail: 'the small embedded preview' }
        ]
      }
    });
    await openAndClose();
    await exportTo(String.raw`C:\work\out.psd`);
    expect(await screen.findByText(/Metadata removed: the place the picture was taken \(GPS\); the camera owner name and serial numbers and 1 more\./)).toBeTruthy();
  });
});
