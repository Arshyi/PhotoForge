/**
 * Copying to and pasting from the system clipboard: the keys, the palette, what is sent to the
 * commands (the layer, the selection, the edits), and what the person is told.
 */
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { fireEvent, render, screen, waitFor, within } from '@testing-library/svelte';
import { invoke } from '@tauri-apps/api/core';
import { open, save } from '@tauri-apps/plugin-dialog';
import App from './App.svelte';
import registry from '../src-tauri/tests/fixtures/operation_registry.json';
import { createDocument, createPixelLayer } from './lib/layers/tree';
import type { OperationSpec } from './lib/operations/types';
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

let failCopy: string | null = null;
let failPaste: string | null = null;
let clippedHighlights = false;

function project(name: string, width: number, height: number) {
  const layer = createPixelLayer('Image', 'px1', width, height);
  const document = createDocument(width, height, [layer], 'linear_srgb_f32');
  return {
    documentId: 9100,
    isCurrent: true,
    metadata: {
      filename: name, width, height, format: 'OpenEXR', fileSize: 1024, colorSpace: 'Linear sRGB',
      bitDepth: 32, hasAlpha: true, createdAt: null, modifiedAt: null, cameraModel: null, exifAvailable: false
    },
    originalPreviewDataUrl: previewUrl,
    previewDataUrl: previewUrl,
    document,
    operations: [],
    canvasWidth: width,
    canvasHeight: height,
    applicationVersion: '0.14.0',
    createdAt: '2026-10-10T00:00:00.000Z',
    modifiedAt: '2026-10-10T00:00:00.000Z',
    processingTimeMs: 3,
    pluginRequirements: []
  };
}

beforeEach(() => {
  localStorage.clear();
  failCopy = null;
  failPaste = null;
  clippedHighlights = false;
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
        return {
          project: project('shot.exr', 16, 12),
          report: { format: 'OpenEXR', width: 16, height: 12, bitDepth: 32, colorMode: 'RGB', layersImported: 1, groupsImported: 0, entries: [] }
        };
      case 'copy_to_clipboard':
        if (failCopy) throw new Error(failCopy);
        return {
          width: 16, height: 12, scope: args.layerId ? 'layer' : 'merged', selection: Boolean(args.selection),
          formats: ['PNG', 'CF_DIBV5'], pngBytes: 900, clippedHighlights, processingTimeMs: 3
        };
      case 'paste_clipboard_image':
        if (failPaste) throw new Error(failPaste);
        return { pixelId: 'px-pasted', width: 8, height: 6, filename: 'Pasted image', format: 'png' };
      case 'open_clipboard_image':
        if (failPaste) throw new Error(failPaste);
        return project('Pasted image', 20, 10);
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

async function openDocument() {
  render(App);
  vi.mocked(open).mockResolvedValueOnce(EXR);
  await fireEvent.click(screen.getByRole('button', { name: 'Open' }));
  const dialog = await screen.findByRole('dialog', { name: /shot\.exr/ });
  await fireEvent.click(within(dialog).getByRole('button', { name: 'Close' }));
  await waitFor(() => expect(screen.queryByRole('dialog', { name: /shot\.exr/ })).toBeNull());
  await screen.findByRole('button', { name: 'Select Image' });
}

async function palette(query: string) {
  await fireEvent.keyDown(window, { key: 'P', ctrlKey: true, shiftKey: true });
  const input = await screen.findByRole('combobox', { name: 'Search commands' });
  await fireEvent.input(input, { target: { value: query } });
  await fireEvent.keyDown(input, { key: 'Enter' });
}

describe('copying', () => {
  it('copies the active layer with Ctrl+C, and says so', async () => {
    await openDocument();
    await fireEvent.keyDown(window, { key: 'c', ctrlKey: true });
    await waitFor(() => expect(calls('copy_to_clipboard')).toHaveLength(1));
    const [, args] = calls('copy_to_clipboard')[0] as [string, Record<string, any>];
    expect(args.layerId).toBeTruthy();
    expect(args.selection).toBeNull();
    expect(args.document.layers).toHaveLength(1);
    expect(args.operations).toEqual([]);
    expect(await screen.findByText(/Copied the layer \(16 × 12\) to the clipboard/)).toBeTruthy();
  });

  it('copies the merged picture with Ctrl+Shift+C, with no layer named', async () => {
    await openDocument();
    await fireEvent.keyDown(window, { key: 'C', ctrlKey: true, shiftKey: true });
    await waitFor(() => expect(calls('copy_to_clipboard')).toHaveLength(1));
    const [, args] = calls('copy_to_clipboard')[0] as [string, Record<string, any>];
    expect(args.layerId).toBeNull();
    expect(await screen.findByText(/Copied the picture \(16 × 12\)/)).toBeTruthy();
  });

  it('warns when highlights were clipped for the 8-bit clipboard', async () => {
    clippedHighlights = true;
    await openDocument();
    await fireEvent.keyDown(window, { key: 'C', ctrlKey: true, shiftKey: true });
    expect(await screen.findByText(/Highlights brighter than white were clipped/)).toBeTruthy();
  });

  it('shows the engine’s reason when the copy fails, and leaves the document alone', async () => {
    failCopy = 'The clipboard is in use by another program. Try again in a moment.';
    await openDocument();
    await fireEvent.keyDown(window, { key: 'c', ctrlKey: true });
    expect(await screen.findByText(/in use by another program/)).toBeTruthy();
    expect(screen.getByRole('button', { name: 'Select Image' })).toBeTruthy();
  });

  it('is in the command palette, with its keys', async () => {
    await openDocument();
    await palette('copy merged');
    await waitFor(() => expect(calls('copy_to_clipboard')).toHaveLength(1));
    expect((calls('copy_to_clipboard')[0][1] as Record<string, any>).layerId).toBeNull();
  });
});

describe('pasting', () => {
  it('places the clipboard picture as a new layer with Ctrl+V', async () => {
    await openDocument();
    await fireEvent.keyDown(window, { key: 'v', ctrlKey: true });
    await waitFor(() => expect(calls('paste_clipboard_image')).toHaveLength(1));
    expect(await screen.findByText(/Pasted a 8 × 6 picture from the clipboard as a new layer/)).toBeTruthy();
    // It went through the engine as a placed image, not as a file import.
    expect(calls('import_layer_image')).toHaveLength(0);
    expect(calls('open_clipboard_image')).toHaveLength(0);
  });

  it('opens the picture as a new document when none is open', async () => {
    render(App);
    await fireEvent.keyDown(window, { key: 'v', ctrlKey: true });
    await waitFor(() => expect(calls('open_clipboard_image')).toHaveLength(1));
    expect(await screen.findByText(/Opened the 20 × 10 picture from the clipboard as a new document/)).toBeTruthy();
    expect(await screen.findByRole('button', { name: 'Select Image' })).toBeTruthy();
    expect(calls('paste_clipboard_image')).toHaveLength(0);
  });

  it('offers “New document from clipboard” even with a document open', async () => {
    await openDocument();
    await palette('new document from clipboard');
    await waitFor(() => expect(calls('open_clipboard_image')).toHaveLength(1));
  });

  it('says when there is no picture on the clipboard, and changes nothing', async () => {
    failPaste = 'The clipboard does not hold a picture.';
    await openDocument();
    await fireEvent.keyDown(window, { key: 'v', ctrlKey: true });
    expect(await screen.findByText(/does not hold a picture/)).toBeTruthy();
    expect(screen.getByRole('button', { name: 'Select Image' })).toBeTruthy();
  });

  it('leaves Ctrl+V to a text field', async () => {
    await openDocument();
    const field = document.createElement('input');
    field.type = 'text';
    document.body.appendChild(field);
    field.focus();
    await fireEvent.keyDown(field, { key: 'v', ctrlKey: true });
    expect(calls('paste_clipboard_image')).toHaveLength(0);
    field.remove();
  });
});
