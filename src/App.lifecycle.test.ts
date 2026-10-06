import { fireEvent, render, screen, waitFor } from '@testing-library/svelte';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { invoke } from '@tauri-apps/api/core';
import { open, save } from '@tauri-apps/plugin-dialog';
import App from './App.svelte';
import { createAdjustmentLayer, createDocument, createPixelLayer, insertLayer, updateLayer } from './lib/layers/tree';
import type { TransactionRequest } from './lib/operations/types';
import type { RecoveryRecord } from './lib/layers/commands';
import type { LayerDocument, ProjectLoadResult } from './lib/layers/types';
import type { ImageMetadata, Workflow } from './lib/types/editor';
import { createWorkflow, saveWorkflows, workflowDocument } from './lib/utils/workflows';
import { decodedCoverageChecksum } from './lib/selections/checksum';
import type { MaskSnapshot } from './lib/selections/types';

const nativeEvents = vi.hoisted(() => ({
  drop: undefined as undefined | ((event: { payload: { type: 'drop'; paths: string[] } }) => void)
}));

vi.mock('@tauri-apps/api/core', () => ({ invoke: vi.fn() }));
vi.mock('@tauri-apps/plugin-dialog', () => ({ open: vi.fn(), save: vi.fn() }));
vi.mock('@tauri-apps/api/webview', () => ({
  getCurrentWebview: () => ({
    onDragDropEvent: async (callback: typeof nativeEvents.drop) => {
      nativeEvents.drop = callback;
      return () => { nativeEvents.drop = undefined; };
    }
  })
}));
vi.mock('@tauri-apps/api/window', () => ({
  getCurrentWindow: () => ({ onCloseRequested: async () => () => undefined })
}));

const originalUrl = 'data:image/png;base64,b3JpZ2luYWw=';
const compositeUrl = 'data:image/png;base64,Y29tcG9zaXRl';
const metadata: ImageMetadata = {
  filename: 'fixture.png', width: 16, height: 12, format: 'PNG', fileSize: 128,
  colorSpace: 'sRGB', bitDepth: 8, hasAlpha: true, createdAt: null,
  modifiedAt: null, cameraModel: null, exifAvailable: false
};

function maskFixture(value: number): MaskSnapshot {
  const pixels = new Uint8Array(16 * 12).fill(value);
  const snapshot: MaskSnapshot = {
    version: 1, width: 16, height: 12, encoding: 'base64_u8',
    data: btoa(String.fromCharCode(...pixels)).replace(/=+$/, ''),
    checksum: ''
  };
  snapshot.checksum = decodedCoverageChecksum(snapshot) ?? '';
  return snapshot;
}

const originalMask = maskFixture(32);
const canvasStroke = maskFixture(64);
const localStroke = maskFixture(96);
const composedMask = maskFixture(128);
const invertedOriginalMask = maskFixture(223);
const invertedComposedMask = maskFixture(127);

function projectResult(documentId = 7001): ProjectLoadResult {
  return {
    documentId, isCurrent: true, metadata,
    document: createDocument(16, 12, [createPixelLayer('Background', 'pxproject', 16, 12)]),
    originalPreviewDataUrl: originalUrl, previewDataUrl: originalUrl,
    operations: [], canvasWidth: 16, canvasHeight: 12,
    applicationVersion: '0.8.2', createdAt: '2026-09-04T00:00:00Z',
    modifiedAt: '2026-09-04T00:00:00Z', processingTimeMs: 1
  };
}

function calls(command: string) {
  return vi.mocked(invoke).mock.calls.filter(([name]) => name === command);
}

function argsFor(command: string, index = -1): Record<string, unknown> {
  return calls(command).at(index)?.[1] as Record<string, unknown>;
}

async function openImage() {
  vi.mocked(open).mockResolvedValueOnce('C:\\fixtures\\fixture.png');
  await fireEvent.click(screen.getByRole('button', { name: 'Open' }));
  await waitFor(() => expect(screen.getByAltText('Edited preview of fixture.png')).toBeTruthy());
}

async function saveCurrentProject(expectedCount: number) {
  await waitFor(() => expect(screen.getByRole('button', { name: /^Save project/ }).hasAttribute('disabled')).toBe(false));
  await fireEvent.click(screen.getByRole('button', { name: /^Save project/ }));
  await waitFor(() => expect(calls('save_layer_project')).toHaveLength(expectedCount));
  await waitFor(() => expect(screen.getByRole('button', { name: /^Save project/ }).hasAttribute('disabled')).toBe(false));
  return argsFor('save_layer_project');
}

// These mount the whole App against its full mock set and legitimately take
// 3-10 s each, which is most of vitest's 5 s default. Five of them used to fail
// whenever the machine was busy and pass when it was idle, so the suite total
// depended on load rather than on the code. The bound is raised for this block
// only: a global timeout would hide a genuinely slow unit test elsewhere.
describe('App document lifecycle', { timeout: 30_000 }, () => {
  let snapshots: RecoveryRecord[];
  let failSave: boolean;
  let failPixelWorker: boolean;
  let importedWorkflow: Workflow | null;
  let pendingCreate: Promise<unknown> | null;
  let loadedProject: ProjectLoadResult | null;

  beforeEach(() => {
    localStorage.clear();
    snapshots = [];
    failSave = false;
    failPixelWorker = false;
    importedWorkflow = null;
    pendingCreate = null;
    loadedProject = null;
    nativeEvents.drop = undefined;
    vi.mocked(open).mockReset();
    vi.mocked(save).mockReset();
    vi.spyOn(window, 'confirm').mockReturnValue(true);
    vi.spyOn(HTMLCanvasElement.prototype, 'getContext').mockImplementation(() => ({
      createImageData: (width: number, height: number) => ({
        data: new Uint8ClampedArray(width * height * 4), width, height
      }),
      putImageData: vi.fn(), clearRect: vi.fn()
    }) as unknown as CanvasRenderingContext2D);
    vi.mocked(invoke).mockReset().mockImplementation(async (command, options) => {
      const args = options as Record<string, unknown>;
      switch (command) {
        case 'list_recovery_snapshots': return { snapshots };
        case 'import_workflow': return workflowDocument(importedWorkflow!);
        case 'plan_layer_workflow': return { targets: [], steps: (args.steps as unknown[]).length };
        case 'load_layer_project': return loadedProject ?? projectResult();
        case 'restore_recovery_snapshot': return projectResult(8001);
        case 'open_image': return {
          metadata, documentId: 6001, isCurrent: true, backgroundPixelId: 'pxoriginal',
          originalPreviewDataUrl: originalUrl, previewDataUrl: originalUrl, processingTimeMs: 1
        };
        case 'render_layer_composite':
        case 'render_preview': return {
          requestId: args.requestId, isCurrent: true, previewDataUrl: compositeUrl,
          processingTimeMs: 1, operationCount: (args.operations as unknown[]).length
        };
        case 'render_layer_thumbnail': return {
          layerId: args.layerId, previewDataUrl: originalUrl, width: 16, height: 12
        };
        case 'create_layer_pixels': return pendingCreate ?? {
          pixelId: 'pxcreated', width: 16, height: 12, bytes: 768
        };
        case 'apply_operations_to_layer':
          if (failPixelWorker) throw new Error('Pixel worker failed');
          return { pixelId: 'pxedited', width: 16, height: 12, bytes: 768 };
        // A stand-in for the backend's transaction engine, which is tested for real
        // in src-tauri/tests/operations_engine.rs. It performs the two operations
        // these tests replay on a copy and, like the engine, hands back a document
        // without touching the one it was given.
        case 'apply_transaction': {
          const request = args.request as TransactionRequest;
          let next = structuredClone(request.document) as LayerDocument;
          let lastCreated: string | null = null;
          for (const step of request.steps) {
            const params = (step.params ?? {}) as Record<string, any>;
            if (step.op === 'core.layer.set_opacity') {
              next = updateLayer(next, next.activeLayerId!, (layer) => ({ ...layer, opacity: params.opacity }));
            } else if (step.op === 'core.layer.add_adjustment') {
              const layer = createAdjustmentLayer(params.name ?? 'Adjustment', params.operation);
              next = insertLayer(next, layer, null, next.layers.length);
              lastCreated = layer.id;
            } else if (step.op === 'core.document.flatten') {
              next = createDocument(next.canvasWidth, next.canvasHeight, [
                createPixelLayer('Background', 'pxflattened', 16, 12)
              ], next.precision ?? 'legacy_srgb8');
            } else if (step.op === 'core.layer.apply_edit') {
              if (failPixelWorker) throw new Error('Step 2 (core.layer.apply_edit) failed: Pixel worker failed Nothing was changed.');
            }
          }
          return {
            document: next, revision: 'r'.repeat(64), label: request.label, steps: [],
            createdPixelIds: [], lastCreated
          };
        }
        case 'rasterize_selection': return {
          documentId: args.documentId, requestId: args.requestId, isCurrent: true,
          mask: canvasStroke, diagnostics: null, processingTimeMs: 1
        };
        case 'layer_mask_from_selection': return { snapshot: localStroke, width: 16, height: 12 };
        case 'compose_selection_masks': return {
          documentId: args.documentId, requestId: args.requestId, isCurrent: true,
          mask: composedMask, diagnostics: null, processingTimeMs: 1
        };
        case 'transform_selection_mask': return {
          documentId: args.documentId, requestId: args.requestId, isCurrent: true,
          mask: (args.mask as MaskSnapshot).checksum === originalMask.checksum ? invertedOriginalMask : invertedComposedMask,
          diagnostics: null, processingTimeMs: 1
        };
        case 'analyze_image': return { isCurrent: false };
        case 'save_layer_project':
          if (failSave) throw new Error('Disk is unavailable');
          return { outputPath: args.outputPath, bytes: 1234, processingTimeMs: 1 };
        case 'discard_recovery_snapshot': return 1;
        case 'get_component_snapshot': return {
          configuration: { activePlanner: 'rule', ollamaSelectedModel: null, ollamaMaxOperations: 8 }
        };
        case 'get_ollama_diagnostics': return { connected: false };
        default: return {};
      }
    });
  });

  afterEach(() => vi.restoreAllMocks());

  it('uses the backend project identity and compositor even for one plain layer with zero operations', async () => {
    render(App);
    vi.mocked(open).mockResolvedValueOnce('C:\\fixtures\\saved.photoforge');
    await fireEvent.click(screen.getByRole('button', { name: 'Open project' }));

    await waitFor(() => expect(calls('render_layer_composite')).toHaveLength(1));
    expect(argsFor('load_layer_project').requestId).toEqual(expect.any(Number));
    expect(argsFor('render_layer_composite')).toMatchObject({ documentId: 7001, operations: [] });
    expect(argsFor('analyze_image').documentId).toBe(7001);
    expect(calls('render_preview')).toHaveLength(0);
    await waitFor(() => expect(screen.getByAltText('Edited preview of fixture.png').getAttribute('src')).toBe(compositeUrl));
  });

  it('renders a flattened replacement buffer instead of reverting to the original photo', async () => {
    render(App);
    await openImage();
    expect(calls('render_layer_composite')).toHaveLength(0);
    await fireEvent.click(screen.getByRole('button', { name: 'Flatten' }));

    await waitFor(() => expect(calls('render_layer_composite')).toHaveLength(1));
    const request = argsFor('render_layer_composite');
    expect(request).toMatchObject({ documentId: 6001, operations: [] });
    const document = request.document as LayerDocument;
    expect(document.layers).toHaveLength(1);
    expect(document.layers[0].content).toMatchObject({ type: 'pixel', pixelId: 'pxflattened' });
    expect(calls('render_preview')).toHaveLength(0);
    await waitFor(() => expect(screen.getByAltText('Edited preview of fixture.png').getAttribute('src')).toBe(compositeUrl));
  });

  it('holds the document stable while the Save As dialog is pending', async () => {
    render(App);
    await openImage();
    let chooseDestination!: (path: string) => void;
    vi.mocked(save).mockReturnValueOnce(new Promise((resolve) => { chooseDestination = resolve; }));
    await fireEvent.click(screen.getByRole('button', { name: /^Save project/ }));
    await waitFor(() => expect(save).toHaveBeenCalledTimes(1));

    // The guards must protect callbacks as well as disabled native controls.
    await fireEvent.click(screen.getByRole('button', { name: '◐ Grayscale' }));
    await fireEvent.click(screen.getByRole('button', { name: 'Open' }));
    await fireEvent.click(screen.getByRole('button', { name: 'Open project' }));
    expect(open).toHaveBeenCalledTimes(1);
    expect(calls('open_image')).toHaveLength(1);
    expect(calls('load_layer_project')).toHaveLength(0);
    expect(calls('save_layer_project')).toHaveLength(0);

    chooseDestination('C:\\fixtures\\new.photoforge');
    await waitFor(() => expect(calls('save_layer_project')).toHaveLength(1));
    expect(argsFor('save_layer_project').operations).toEqual([]);
    expect((argsFor('save_layer_project').document as LayerDocument).layers[0].content)
      .toMatchObject({ pixelId: 'pxoriginal' });
    await waitFor(() => expect(screen.getByRole('button', { name: /^Save project/ }).hasAttribute('disabled')).toBe(false));
    await fireEvent.click(screen.getByRole('button', { name: '◐ Grayscale' }));
    await waitFor(() => expect(calls('render_layer_composite')).toHaveLength(1));
    expect(argsFor('render_layer_composite').operations).toEqual([{ type: 'grayscale' }]);
    expect((argsFor('render_layer_composite').document as LayerDocument).precision).toBe('linear_srgb_f32');
    expect(calls('render_preview')).toHaveLength(0);
  });

  it('keeps recovered work until a successful save and deletes only its own snapshot', async () => {
    const unrelated = 'C:\\recovery\\unrelated.pfrecovery';
    const recovered = 'C:\\recovery\\current.pfrecovery';
    snapshots = [
      { projectPath: null, documentName: 'Other work', savedAt: '2026-09-03T00:00:00Z', snapshotPath: unrelated, bytes: 10 },
      { projectPath: null, documentName: 'Current work', savedAt: '2026-09-04T00:00:00Z', snapshotPath: recovered, bytes: 10 }
    ];
    render(App);
    await fireEvent.click(await screen.findByRole('button', { name: 'Recover' }));
    await waitFor(() => expect(argsFor('render_layer_composite')?.documentId).toBe(8001));
    expect(argsFor('restore_recovery_snapshot').path).toBe(recovered);
    expect(calls('discard_recovery_snapshot')).toHaveLength(0);

    failSave = true;
    vi.mocked(save).mockResolvedValueOnce('C:\\fixtures\\recovered.photoforge');
    await fireEvent.click(screen.getByRole('button', { name: /^Save project/ }));
    await screen.findByText('Disk is unavailable');
    expect(calls('discard_recovery_snapshot')).toHaveLength(0);

    failSave = false;
    vi.mocked(save).mockResolvedValueOnce('C:\\fixtures\\recovered.photoforge');
    await fireEvent.click(screen.getByRole('button', { name: /^Save project/ }));
    await waitFor(() => expect(calls('discard_recovery_snapshot')).toHaveLength(1));
    expect(argsFor('discard_recovery_snapshot')).toEqual({ path: recovered });
    expect(calls('discard_recovery_snapshot').some(([, args]) => {
      const path = (args as Record<string, unknown>).path;
      return path === unrelated || path === null;
    })).toBe(false);
  });

  it('imports and replays a mixed schema-v2 workflow as one undoable document edit', async () => {
    importedWorkflow = createWorkflow('Layered workflow', [{ type: 'grayscale' }], '', new Date(), [
      { type: 'set_opacity', selector: { type: 'active' }, opacity: 0.35 },
      { type: 'create_adjustment_layer', name: 'Workflow light', operation: { type: 'brightness', amount: 0.15 } }
    ]);
    render(App);
    await openImage();
    await fireEvent.click(screen.getByRole('tab', { name: 'Flows' }));
    vi.mocked(open).mockResolvedValueOnce('C:\\fixtures\\layered-workflow.json');
    await fireEvent.click(screen.getByRole('button', { name: 'Import JSON' }));
    await screen.findByText('Layered workflow');
    expect(argsFor('import_workflow').path).toBe('C:\\fixtures\\layered-workflow.json');
    expect(localStorage.getItem('photoforge.workflows.v1')).toContain('set_opacity');
    await fireEvent.click(screen.getByRole('button', { name: 'Replay' }));
    await screen.findByRole('button', { name: 'Select Workflow light' });
    await waitFor(() => expect(screen.getByRole('button', { name: /^Save project/ }).hasAttribute('disabled')).toBe(false));

    vi.mocked(save).mockResolvedValueOnce('C:\\fixtures\\workflow.photoforge');
    const replayed = await saveCurrentProject(1);
    expect(replayed.operations).toEqual([{ type: 'grayscale' }]);
    const layers = (replayed.document as LayerDocument).layers;
    expect(layers).toHaveLength(2);
    expect(layers[0]).toMatchObject({ opacity: 0.35, content: { pixelId: 'pxoriginal' } });
    expect(layers[1]).toMatchObject({ name: 'Workflow light', content: {
      type: 'adjustment', operation: { type: 'brightness', amount: 0.15 }
    } });

    await fireEvent.click(screen.getByRole('button', { name: 'Undo' }));
    const undone = await saveCurrentProject(2);
    expect(undone.operations).toEqual([]);
    expect((undone.document as LayerDocument).layers).toHaveLength(1);
    expect((undone.document as LayerDocument).layers[0]).toMatchObject({ opacity: 1, content: { pixelId: 'pxoriginal' } });

    await fireEvent.click(screen.getByRole('button', { name: 'Redo' }));
    const redone = await saveCurrentProject(3);
    expect(redone.operations).toEqual(replayed.operations);
    expect(redone.document).toEqual(replayed.document);
  });

  it('does not publish staged layer or global edits when a workflow pixel worker fails', async () => {
    saveWorkflows([createWorkflow('Failing layer workflow', [{ type: 'brightness', amount: 0.2 }], '', new Date(), [
      { type: 'set_opacity', selector: { type: 'active' }, opacity: 0.3 },
      { type: 'apply_to_layer', selector: { type: 'active' }, operations: [{ type: 'grayscale' }] }
    ])]);
    failPixelWorker = true;
    render(App);
    await openImage();
    await fireEvent.click(screen.getByRole('tab', { name: 'Flows' }));
    await fireEvent.click(screen.getByRole('button', { name: 'Replay' }));
    await screen.findByText(/Pixel worker failed/);
    // One transaction carried both steps. The document it was given is unchanged:
    // the engine works on a copy, and the copy is what was thrown away.
    expect(calls('apply_transaction')).toHaveLength(1);
    const request = argsFor('apply_transaction').request as TransactionRequest;
    expect(request.steps.map((step) => step.op)).toEqual(['core.layer.set_opacity', 'core.layer.apply_edit']);
    expect(request.origin).toBe('automation');
    expect((request.document as LayerDocument).layers[0].opacity).toBe(1);
    expect(screen.getByRole('button', { name: 'Undo' }).hasAttribute('disabled')).toBe(true);
    expect(calls('render_preview')).toHaveLength(0);
    expect(calls('render_layer_composite')).toHaveLength(0);

    vi.mocked(save).mockResolvedValueOnce('C:\\fixtures\\unchanged.photoforge');
    const unchanged = await saveCurrentProject(1);
    expect(unchanged.operations).toEqual([]);
    expect((unchanged.document as LayerDocument).layers).toHaveLength(1);
    expect((unchanged.document as LayerDocument).layers[0]).toMatchObject({ opacity: 1, content: { pixelId: 'pxoriginal' } });
    expect(screen.getByAltText('Edited preview of fixture.png').getAttribute('src')).toBe(originalUrl);
  });

  it('blocks Open and native drag-drop while an asynchronous layer creation owns the document', async () => {
    render(App);
    await openImage();
    let finishCreate!: (value: unknown) => void;
    pendingCreate = new Promise((resolve) => { finishCreate = resolve; });
    await fireEvent.click(screen.getByRole('button', { name: 'New pixel layer' }));
    await waitFor(() => expect(calls('create_layer_pixels')).toHaveLength(1));
    await fireEvent.click(screen.getByRole('button', { name: 'Open' }));
    expect(nativeEvents.drop).toBeTypeOf('function');
    nativeEvents.drop?.({ payload: { type: 'drop', paths: ['C:\\fixtures\\other.png'] } });
    await fireEvent.click(screen.getByRole('button', { name: 'Open project' }));
    expect(open).toHaveBeenCalledTimes(1);
    expect(calls('open_image')).toHaveLength(1);
    expect(calls('load_layer_project')).toHaveLength(0);

    finishCreate({ pixelId: 'pxcreated', width: 16, height: 12, bytes: 768 });
    await screen.findByRole('button', { name: 'Select Layer' });
    await waitFor(() => expect(calls('render_layer_composite')).toHaveLength(1));
    const rendered = argsFor('render_layer_composite');
    expect(rendered.documentId).toBe(6001);
    expect((rendered.document as LayerDocument).layers.map((layer) => layer.content)).toEqual([
      expect.objectContaining({ pixelId: 'pxoriginal' }),
      expect.objectContaining({ pixelId: 'pxcreated' })
    ]);
  });

  it.each([
    { tool: 'brush', label: 'Selection brush', inverted: false, mode: 'add' },
    { tool: 'eraser', label: 'Selection eraser', inverted: true, mode: 'subtract' }
  ])('routes a $tool gesture to the layer mask with inverted=$inverted and one-step Undo', async ({ tool, label, inverted, mode }) => {
    loadedProject = projectResult();
    loadedProject.document.layers[0].mask = { snapshot: originalMask, enabled: true, inverted };
    loadedProject.document.layers[0].transform.translateX = 2;
    const layerId = loadedProject.document.layers[0].id;
    render(App);
    vi.mocked(open).mockResolvedValueOnce('C:\\fixtures\\masked.photoforge');
    await fireEvent.click(screen.getByRole('button', { name: 'Open project' }));
    await waitFor(() => expect(calls('render_layer_composite')).toHaveLength(1));
    await fireEvent.click(screen.getByRole('button', { name: 'Mask' }));
    await fireEvent.click(screen.getByRole('button', { name: label }));
    const canvas = screen.getByRole('button', { name: `${tool} selection canvas` });
    Object.defineProperty(canvas, 'getBoundingClientRect', { value: () => ({
      left: 0, top: 0, width: 160, height: 120, right: 160, bottom: 120,
      x: 0, y: 0, toJSON: () => ({})
    }) });
    Object.defineProperty(canvas, 'setPointerCapture', { value: vi.fn() });
    Object.defineProperty(canvas, 'releasePointerCapture', { value: vi.fn() });
    const down = new MouseEvent('pointerdown', { bubbles: true, clientX: 40, clientY: 30 });
    const up = new MouseEvent('pointerup', { bubbles: true, clientX: 80, clientY: 60 });
    Object.defineProperty(down, 'pointerId', { value: 1 });
    Object.defineProperty(up, 'pointerId', { value: 1 });
    await fireEvent(canvas, down);
    await fireEvent(canvas, up);
    await waitFor(() => expect(calls('compose_selection_masks')).toHaveLength(1));

    expect(argsFor('rasterize_selection')).toMatchObject({
      width: 16, height: 12, mode: 'replace', base: null, documentId: 7001,
      shape: { type: 'resolved_brush', samples: expect.arrayContaining([expect.objectContaining({ x: 4, y: 3 })]) }
    });
    expect(argsFor('layer_mask_from_selection')).toMatchObject({
      layerId, selection: canvasStroke, document: { layers: [expect.objectContaining({
        transform: expect.objectContaining({ translateX: 2 }), mask: { snapshot: originalMask, enabled: true, inverted }
      })] }
    });
    expect(argsFor('compose_selection_masks')).toMatchObject({
      base: inverted ? invertedOriginalMask : originalMask, incoming: localStroke, mode, documentId: 7001
    });
    expect(calls('selection_from_layer_mask')).toHaveLength(0);
    expect(calls('transform_selection_mask')).toHaveLength(inverted ? 2 : 0);
    expect(screen.getByText('No active selection. Global adjustments remain unchanged.')).toBeTruthy();

    const painted = await saveCurrentProject(1);
    expect(painted.operations).toEqual([]);
    expect((painted.document as LayerDocument).layers[0].mask).toEqual({
      snapshot: inverted ? invertedComposedMask : composedMask, enabled: true, inverted
    });
    await fireEvent.click(screen.getByRole('button', { name: 'Undo' }));
    const undone = await saveCurrentProject(2);
    expect((undone.document as LayerDocument).layers[0].mask).toEqual({ snapshot: originalMask, enabled: true, inverted });
    expect(screen.getByText('No active selection. Global adjustments remain unchanged.')).toBeTruthy();
  });
});
