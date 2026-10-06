import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { fireEvent, render, screen, waitFor } from '@testing-library/svelte';
import { invoke } from '@tauri-apps/api/core';
import { open, save } from '@tauri-apps/plugin-dialog';
import App from './App.svelte';
import { admissionFixture } from './lib/source/testing';
import type { SourceAdmission, SourceOrigin } from './lib/source/types';
import type { LayerDocument } from './lib/layers/types';
import type { ImageMetadata } from './lib/types/editor';

vi.mock('@tauri-apps/api/core', () => ({ invoke: vi.fn() }));
vi.mock('@tauri-apps/plugin-dialog', () => ({ open: vi.fn(), save: vi.fn() }));
vi.mock('@tauri-apps/api/webview', () => ({
  getCurrentWebview: () => ({ onDragDropEvent: async () => () => undefined })
}));
vi.mock('@tauri-apps/api/window', () => ({
  getCurrentWindow: () => ({ onCloseRequested: async () => () => undefined })
}));

const WALL = String.raw`C:\scans\wall.png`;
const SMALL = String.raw`C:\fixtures\fixture.png`;
const previewUrl = 'data:image/png;base64,b3JpZ2luYWw=';

const smallMetadata: ImageMetadata = {
  filename: 'fixture.png', width: 16, height: 12, format: 'PNG', fileSize: 128,
  colorSpace: 'sRGB', bitDepth: 8, hasAlpha: true, createdAt: null,
  modifiedAt: null, cameraModel: null, exifAvailable: false
};

const wallSource: SourceOrigin['source'] = {
  path: WALL, sha256: 'a'.repeat(64), bytes: 90_000_000, kind: 'png', width: 12_000, height: 9_000
};

function regionOrigin(rect: { x: number; y: number; width: number; height: number }): SourceOrigin {
  return { source: wallSource, view: { view: 'region', rect } };
}

const calls = (command: string) => vi.mocked(invoke).mock.calls.filter(([name]) => name === command);
const argsFor = (command: string) => calls(command).at(-1)?.[1] as Record<string, unknown>;

let admission: SourceAdmission | Error;

function opened(metadata: ImageMetadata, pixel = 'pxopened') {
  return {
    metadata, documentId: 9001, isCurrent: true, backgroundPixelId: pixel,
    originalPreviewDataUrl: previewUrl, previewDataUrl: previewUrl, processingTimeMs: 1
  };
}

beforeEach(() => {
  localStorage.clear();
  admission = admissionFixture();
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
    const args = (options ?? {}) as Record<string, unknown>;
    switch (command) {
      case 'probe_image_source':
        if (admission instanceof Error) throw admission;
        return admission;
      case 'source_preview_image':
        return { dataUrl: previewUrl, width: 600, height: 450 };
      case 'cancel_source_preview':
        return undefined;
      case 'open_image':
        return opened(smallMetadata, 'pxsmall');
      case 'open_image_selection': {
        const selection = args.selection as {
          kind: string;
          rect?: { x: number; y: number; width: number; height: number };
          width?: number;
          height?: number;
        };
        if (selection.kind === 'region') {
          const rect = selection.rect!;
          return opened(
            { ...smallMetadata, filename: 'wall.png', width: rect.width, height: rect.height, origin: regionOrigin(rect) },
            'pxregion'
          );
        }
        return opened(
          {
            ...smallMetadata,
            filename: 'wall.png',
            width: selection.width!,
            height: selection.height!,
            origin: {
              source: wallSource,
              view: { view: 'reduced', width: selection.width!, height: selection.height! }
            }
          },
          'pxreduced'
        );
      }
      case 'save_layer_project':
        return { outputPath: args.outputPath, bytes: 1, processingTimeMs: 1 };
      case 'render_layer_composite':
      case 'render_preview':
        return {
          requestId: args.requestId, isCurrent: true, previewDataUrl: previewUrl,
          processingTimeMs: 1, operationCount: 0
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

async function chooseFile(path: string) {
  vi.mocked(open).mockResolvedValueOnce(path);
  await fireEvent.click(screen.getByRole('button', { name: 'Open' }));
}

describe('opening an image too large to open whole', { timeout: 30_000 }, () => {
  it('asks what to do and opens nothing until the user decides', async () => {
    render(App);
    await chooseFile(WALL);
    await waitFor(() => expect(screen.getByRole('dialog')).toBeTruthy());
    expect(screen.getByText('This image is too large to open whole')).toBeTruthy();
    expect(screen.getByRole('button', { name: /Choose Region/ })).toBeTruthy();
    // Nothing was decoded: neither open command ran.
    expect(calls('open_image')).toHaveLength(0);
    expect(calls('open_image_selection')).toHaveLength(0);
    expect(argsFor('probe_image_source')).toEqual({ path: WALL });
  });

  it('opens the chosen region through the selection command, and records where it came from', async () => {
    render(App);
    await chooseFile(WALL);
    await fireEvent.click(await screen.findByRole('button', { name: /Choose Region/ }));
    await waitFor(() => expect(screen.getByAltText(/Reduced preview/)).toBeTruthy());
    await fireEvent.click(screen.getByRole('button', { name: 'Open region' }));

    await waitFor(() => expect(calls('open_image_selection')).toHaveLength(1));
    const request = argsFor('open_image_selection');
    expect(request.path).toBe(WALL);
    expect(request.selection).toMatchObject({ kind: 'region', rect: expect.objectContaining({ width: expect.any(Number) }) });
    expect(request.requestId).toEqual(expect.any(Number));
    // The whole-file command never ran.
    expect(calls('open_image')).toHaveLength(0);
    // The dialog is gone and the region is the document.
    await waitFor(() => expect(screen.queryByRole('dialog')).toBeNull());
    await waitFor(() => expect(screen.getByText(/wall\.png: region .* of 12000 × 9000 opened locally/)).toBeTruthy());
  });

  /** The relationship is persisted: it must reach the saved project, on the layer. */
  it('carries the source relationship into the saved project', async () => {
    render(App);
    await chooseFile(WALL);
    await fireEvent.click(await screen.findByRole('button', { name: /Choose Region/ }));
    await waitFor(() => expect(screen.getByAltText(/Reduced preview/)).toBeTruthy());
    await fireEvent.click(screen.getByRole('button', { name: 'Open region' }));
    await waitFor(() => expect(screen.getByText(/opened locally/)).toBeTruthy());

    vi.mocked(save).mockResolvedValueOnce(String.raw`C:\out\wall.photoforge`);
    await waitFor(() =>
      expect(screen.getByRole('button', { name: /^Save project/ }).hasAttribute('disabled')).toBe(false)
    );
    await fireEvent.click(screen.getByRole('button', { name: /^Save project/ }));
    await waitFor(() => expect(calls('save_layer_project')).toHaveLength(1));
    const document = argsFor('save_layer_project').document as LayerDocument;
    expect(document.layers).toHaveLength(1);
    // Whatever region was chosen is exactly what the project records, and the
    // document is that region's size, not the file's.
    const chosen = (argsFor('open_image_selection').selection as { rect: { x: number; y: number; width: number; height: number } }).rect;
    expect(document.layers[0].origin).toEqual(regionOrigin(chosen));
    expect(document.layers[0].name).toBe('Region');
    expect(document.canvasWidth).toBe(chosen.width);
    expect(document.canvasHeight).toBe(chosen.height);
    expect(document.canvasWidth * document.canvasHeight).toBeLessThanOrEqual(20_000_000);
    expect(document.canvasWidth).toBeLessThan(12_000);
  });

  it('opens a reduced copy, labelled as one', async () => {
    render(App);
    await chooseFile(WALL);
    await fireEvent.click(await screen.findByRole('button', { name: /Open Reduced Copy/ }));
    await fireEvent.click(screen.getByLabelText('25%'));
    await fireEvent.click(screen.getByRole('button', { name: 'Open reduced copy' }));

    await waitFor(() => expect(calls('open_image_selection')).toHaveLength(1));
    expect(argsFor('open_image_selection').selection).toEqual({ kind: 'reduced', width: 3_000, height: 2_250 });
    await waitFor(() => expect(screen.getByText(/reduced copy 3000 × 2250 \(25% of the file\)/)).toBeTruthy());
  });

  /** Cancelling must leave the document that is open exactly as it was. */
  it('leaves an open document untouched when the dialog is cancelled', async () => {
    // The small file's probe says it fits, so it goes straight through.
    admission = admissionFixture({ verdict: { kind: 'fullResolution' }, options: [{ kind: 'openFull', peakBytes: 1 }] });
    render(App);
    await chooseFile(SMALL);
    await waitFor(() => expect(screen.getByAltText('Edited preview of fixture.png')).toBeTruthy());
    expect(calls('open_image')).toHaveLength(1);

    // The second file is too large.
    admission = admissionFixture();
    await chooseFile(WALL);
    await waitFor(() => expect(screen.getByRole('dialog')).toBeTruthy());
    await fireEvent.click(screen.getByRole('button', { name: 'Cancel opening this image' }));
    await waitFor(() => expect(screen.queryByRole('dialog')).toBeNull());

    expect(calls('open_image')).toHaveLength(1);
    expect(calls('open_image_selection')).toHaveLength(0);
    expect(screen.getByAltText('Edited preview of fixture.png')).toBeTruthy();
    // And the user was not asked to discard anything, since nothing was replaced.
    expect(window.confirm).not.toHaveBeenCalled();
  });

  it('does not interrupt an image that opens whole', async () => {
    admission = admissionFixture({ verdict: { kind: 'fullResolution' }, options: [{ kind: 'openFull', peakBytes: 1 }] });
    render(App);
    await chooseFile(SMALL);
    await waitFor(() => expect(screen.getByAltText('Edited preview of fixture.png')).toBeTruthy());
    expect(screen.queryByRole('dialog')).toBeNull();
    expect(calls('open_image')).toHaveLength(1);
  });

  it('mentions, after opening, that a large image takes much of the budget', async () => {
    admission = admissionFixture({
      verdict: { kind: 'fullResolutionWithWarning', peakPercentOfBudget: 68 },
      options: [{ kind: 'openFull', peakBytes: 1 }],
      report: { fullPeakBytes: 3_000_000_000 }
    });
    render(App);
    await chooseFile(SMALL);
    await waitFor(() => expect(screen.getByText(/High-memory document: about 2\.\d+ GB while editing, 68% of the memory budget/)).toBeTruthy());
    expect(screen.queryByRole('dialog')).toBeNull();
  });

  /** A file the probe cannot read is the ordinary open's to report, in its own words. */
  it('falls through to the ordinary open when the probe fails', async () => {
    admission = new Error('unsupported');
    render(App);
    await chooseFile(SMALL);
    await waitFor(() => expect(calls('open_image')).toHaveLength(1));
    expect(screen.queryByRole('dialog')).toBeNull();
  });

  it('shows a file refused as unsafe without offering to open any of it', async () => {
    admission = admissionFixture({
      verdict: { kind: 'unsafe', refusal: { kind: 'impossibleExpansion', claimedBytes: 4e10, fileBytes: 60 } },
      options: []
    });
    render(App);
    await chooseFile(WALL);
    await waitFor(() => expect(screen.getByRole('dialog')).toBeTruthy());
    expect(screen.getByText(/not a valid image/)).toBeTruthy();
    expect(screen.queryByRole('button', { name: /Choose Region|Reduced Copy/ })).toBeNull();
  });

  it('keeps the interface behind the dialog inert while it is open', async () => {
    const view = render(App);
    await chooseFile(WALL);
    await waitFor(() => expect(screen.getByRole('dialog')).toBeTruthy());
    const shell = view.container.querySelector('.app-shell') as HTMLElement;
    // `inert` and `aria-hidden` are set together; jsdom reflects only the latter
    // as an attribute, and it is what assistive technology reads.
    expect(shell.getAttribute('aria-hidden')).toBe('true');
    await fireEvent.click(screen.getByRole('button', { name: 'Cancel opening this image' }));
    await waitFor(() => expect(shell.getAttribute('aria-hidden')).not.toBe('true'));
  });
});
