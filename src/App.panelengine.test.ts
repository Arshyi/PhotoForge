/**
 * The Layers panel and the engine, tested as what they promise since 0.15.0: the panel does not
 * edit the layer tree. It asks the engine, and what is on screen and in Undo is what came back.
 *
 * The backend here is a stand-in that applies a handful of operations. It is deliberately not the
 * TypeScript tree functions the panel used to call: if the panel were still editing the tree
 * itself, the "engine says otherwise" tests below could not pass.
 */
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { fireEvent, render, screen, waitFor } from '@testing-library/svelte';
import { readFileSync } from 'node:fs';
import { resolve } from 'node:path';
import { invoke } from '@tauri-apps/api/core';
import { open } from '@tauri-apps/plugin-dialog';
import App from './App.svelte';
import registry from '../src-tauri/tests/fixtures/operation_registry.json';
import type { Layer, LayerDocument } from './lib/layers/types';
import type { ImageMetadata } from './lib/types/editor';
import type { OperationSpec, TransactionRequest } from './lib/operations/types';
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
const requests = () => calls('apply_transaction').map(([, args]) => (args as { request: TransactionRequest }).request);

/** What the stand-in engine does for a test: wait, refuse, or answer with something only an engine would. */
let hold: Promise<unknown> | null;
let refuse: string | null;
let rename: string | null;

/** The operations these tests use, applied the plain way. */
function stand_in(request: TransactionRequest): LayerDocument {
  const document = structuredClone(request.document) as LayerDocument;
  const find = (id: string): Layer => {
    const found = document.layers.find((layer) => layer.id === id);
    if (!found) throw new Error(`no layer ${id}`);
    return found;
  };
  for (const step of request.steps) {
    const params = (step.params ?? {}) as Record<string, any>;
    if (step.op === 'core.layer.set_visible') find(params.selector.id).visible = params.visible;
    else if (step.op === 'core.layer.set_locked') find(params.selector.id).locked = params.locked;
    else if (step.op === 'core.layer.set_opacity') find(params.selector.id).opacity = params.opacity;
    else if (step.op === 'core.layer.rename') find(params.selector.id).name = params.name;
    else if (step.op === 'core.layer.delete') document.layers = document.layers.filter((layer) => layer.id !== params.selector.id);
    else if (step.op === 'core.layer.add_group') {
      const group = { ...structuredClone(document.layers[0]), id: 'lmadegroup', name: 'Group', content: { type: 'group', children: [], isolated: true } } as Layer;
      document.layers.push(group);
      document.activeLayerId = group.id;
    }
  }
  if (rename) for (const layer of document.layers) layer.name = rename;
  return document;
}

beforeEach(() => {
  localStorage.clear();
  hold = null;
  refuse = null;
  rename = null;
  vi.mocked(open).mockReset();
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
      case 'apply_transaction': {
        if (hold) await hold;
        if (refuse) throw new Error(refuse);
        const request = args.request as TransactionRequest;
        return {
          document: stand_in(request), revision: 'r'.repeat(64), label: request.label, steps: [],
          createdPixelIds: [], lastCreated: null
        };
      }
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

async function openImage() {
  vi.mocked(open).mockResolvedValueOnce(FIXTURE);
  await fireEvent.click(screen.getByRole('button', { name: 'Open' }));
  await waitFor(() => expect(screen.getByAltText('Edited preview of fixture.png')).toBeTruthy());
}

const undoButton = () => screen.getByRole('button', { name: 'Undo' }) as HTMLButtonElement;
/** The percentage the opacity slider's own read-out shows. */
const opacityShown = () =>
  document.getElementById('layer-opacity-range')?.parentElement?.querySelector('output')?.textContent?.trim();

describe('the Layers panel asks the engine and shows what it answers', () => {
  it('shows the document the engine returned, not one the panel worked out', async () => {
    render(App);
    await openImage();
    rename = 'Named by the engine';
    await fireEvent.click(screen.getByRole('button', { name: 'Lock Background' }));
    // The panel can only have known this name from the engine's answer.
    await screen.findByRole('button', { name: 'Select Named by the engine' });
    expect(requests()).toHaveLength(1);
    expect(requests()[0]).toMatchObject({
      origin: 'user',
      label: 'Lock layer',
      steps: [{ op: 'core.layer.set_locked', params: { locked: true } }]
    });
    expect(screen.getByRole('button', { name: 'Unlock Named by the engine' })).toBeTruthy();
    await waitFor(() => expect(undoButton().disabled).toBe(false));
  });

  it('changes nothing, and says why, when the engine refuses', async () => {
    render(App);
    await openImage();
    refuse = 'Step 1 (core.layer.set_locked) failed: refused for this test. Nothing was changed.';
    await fireEvent.click(screen.getByRole('button', { name: 'Lock Background' }));
    expect(await screen.findByText(/refused for this test/)).toBeTruthy();
    expect(screen.getByRole('button', { name: 'Lock Background' })).toBeTruthy();
    expect(undoButton().disabled).toBe(true);
  });

  it('queues a quick second edit behind the first, which sees the first one’s result', async () => {
    render(App);
    await openImage();
    let release!: () => void;
    hold = new Promise<void>((resolve) => { release = resolve; });
    await fireEvent.click(screen.getByRole('button', { name: 'Hide Background' }));
    await waitFor(() => expect(requests()).toHaveLength(1));
    // Still with the engine: a second click is accepted, not refused with "wait".
    await fireEvent.click(screen.getByRole('button', { name: 'Lock Background' }));
    expect(screen.queryByText(/Wait for the current/)).toBeNull();
    expect(requests()).toHaveLength(1);

    hold = null;
    release();
    await waitFor(() => expect(requests()).toHaveLength(2));
    // The second request was built from the document the first returned.
    expect(requests()[1].document.layers[0].visible).toBe(false);
    expect(requests()[1].steps[0].op).toBe('core.layer.set_locked');
    await screen.findByRole('button', { name: 'Show Background' });
    await screen.findByRole('button', { name: 'Unlock Background' });
  });

  it('lets a layer be chosen while an edit is queued, and applies the choice after it', async () => {
    render(App);
    await openImage();
    await fireEvent.click(screen.getByRole('button', { name: 'New group' }));
    await screen.findByRole('button', { name: 'Select Group' });
    const activeRow = () => document.querySelector('li.active strong')?.textContent;
    expect(activeRow()).toBe('Group');

    let release!: () => void;
    hold = new Promise<void>((resolve) => { release = resolve; });
    await fireEvent.click(screen.getByRole('button', { name: 'Hide Background' }));
    await waitFor(() => expect(requests()).toHaveLength(2));
    await fireEvent.click(screen.getByRole('button', { name: 'Select Background' }));
    // Not refused, and not applied yet either: the edit ahead of it has not come back.
    expect(screen.queryByText(/Wait for the current/)).toBeNull();
    expect(activeRow()).toBe('Group');

    hold = null;
    release();
    await screen.findByRole('button', { name: 'Show Background' });
    await waitFor(() => expect(activeRow()).toBe('Background'));
  });

  it('queues a new group behind an edit that is still with the engine', async () => {
    render(App);
    await openImage();
    let release!: () => void;
    hold = new Promise<void>((resolve) => { release = resolve; });
    await fireEvent.click(screen.getByRole('button', { name: 'Hide Background' }));
    await waitFor(() => expect(requests()).toHaveLength(1));
    await fireEvent.click(screen.getByRole('button', { name: 'New group' }));
    expect(screen.queryByText(/Wait for the current/)).toBeNull();
    hold = null;
    release();
    await waitFor(() => expect(requests()).toHaveLength(2));
    expect(requests()[1].steps[0].op).toBe('core.layer.add_group');
    // Built from the document the hide returned.
    expect(requests()[1].document.layers[0].visible).toBe(false);
    await screen.findByRole('button', { name: 'Select Group' });
  });

  it('keeps other changes out while the engine is busy', async () => {
    render(App);
    await openImage();
    let release!: () => void;
    hold = new Promise<void>((resolve) => { release = resolve; });
    await fireEvent.click(screen.getByRole('button', { name: 'Hide Background' }));
    await waitFor(() => expect(requests()).toHaveLength(1));
    await fireEvent.click(screen.getByRole('button', { name: 'Open' }));
    expect(open).toHaveBeenCalledTimes(1);
    hold = null;
    release();
    await screen.findByRole('button', { name: 'Show Background' });
  });

  it('draws an opacity drag as it goes and has the engine make it once, when the slider is released', async () => {
    render(App);
    await openImage();
    const slider = await waitFor(() => {
      const found = document.getElementById('layer-opacity-range');
      if (!found) throw new Error('the opacity slider is not there yet');
      return found as HTMLInputElement;
    });
    for (const value of ['80', '60', '40']) await fireEvent.input(slider, { target: { value } });
    // Drawn at once, committed to nothing yet.
    expect(opacityShown()).toBe('40%');
    expect(requests()).toHaveLength(0);
    expect(undoButton().disabled).toBe(true);

    await fireEvent.change(slider);
    await waitFor(() => expect(requests()).toHaveLength(1));
    expect(requests()[0]).toMatchObject({
      label: 'Layer opacity',
      steps: [{ op: 'core.layer.set_opacity', params: { opacity: 0.4 } }]
    });
    await waitFor(() => expect(undoButton().disabled).toBe(false));
    expect(opacityShown()).toBe('40%');
  });

  it('goes back to the committed value if the engine refuses a dragged change', async () => {
    render(App);
    await openImage();
    refuse = 'Step 1 (core.layer.set_opacity) failed: refused for this test. Nothing was changed.';
    const slider = await waitFor(() => {
      const found = document.getElementById('layer-opacity-range');
      if (!found) throw new Error('the opacity slider is not there yet');
      return found as HTMLInputElement;
    });
    await fireEvent.input(slider, { target: { value: '30' } });
    expect(opacityShown()).toBe('30%');
    await fireEvent.change(slider);
    expect(await screen.findByText(/refused for this test/)).toBeTruthy();
    await waitFor(() => expect(opacityShown()).toBe('100%'));
    expect(undoButton().disabled).toBe(true);
  });
});

describe('the application does not edit the layer tree itself for the panel', () => {
  const source = readFileSync(resolve(__dirname, 'App.svelte'), 'utf8');

  it('no longer imports the tree functions the panel used to call', () => {
    const imported = [...source.matchAll(/import\s*\{([^}]*)\}\s*from\s*'\.\/lib\/layers\/tree'/g)]
      .flatMap((match) => match[1].split(',').map((name) => name.trim().split(/\s+as\s+/)[0]))
      .filter(Boolean);
    expect(imported.length).toBeGreaterThan(0);
    for (const name of [
      'moveLayer', 'removeLayer', 'duplicateLayer', 'groupLayers', 'ungroupLayer',
      'createGroupLayer', 'createAdjustmentLayer'
    ]) {
      expect(imported, `${name} is an edit the engine makes`).not.toContain(name);
    }
  });

  it('commits a panel edit only from an engine result', () => {
    // Every call that commits a document the panel produced says so, in one place.
    const fromEngine = source.match(/commitLayers\(next\.document, plan\.label, plan\.coalesceKey, recorded, true\)/g) ?? [];
    expect(fromEngine).toHaveLength(1);
  });
});
