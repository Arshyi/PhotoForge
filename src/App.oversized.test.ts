import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { fireEvent, render, screen, waitFor } from '@testing-library/svelte';
import { invoke } from '@tauri-apps/api/core';
import { open, save } from '@tauri-apps/plugin-dialog';
import App from './App.svelte';
import { admissionFixture } from './lib/source/testing';
import type { SourceAdmission, SourceOrigin } from './lib/source/types';
import type { LayerDocument } from './lib/layers/types';
import type { ImageMetadata, RawLayerSource } from './lib/types/editor';

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
const SENSOR = String.raw`C:\raw\sensor.dng`;
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

function rawSource(rect: { x: number; y: number; width: number; height: number } | null): RawLayerSource {
  return {
    reference: { filename: 'sensor.dng', format: 'DNG', fileSize: 180_000_000, sha256: 'b'.repeat(64), width: 12_000, height: 9_000 },
    mode: { mode: 'linked', path: SENSOR },
    parameters: { whiteBalance: { mode: 'custom', multipliers: [2, 1, 1.5] }, exposureEv: 0, contrast: 0, highlights: 0, shadows: 0, whites: 0, blacks: 0 },
    decoder: 'photoforge-dng',
    decoderVersion: '2',
    capture: {
      manufacturer: null, model: null, lens: null, focalLengthMm: null, aperture: null, shutterSpeedSeconds: null,
      iso: null, captureTime: null, orientation: null, exposureCompensation: null, whiteBalanceMultipliers: null
    },
    ...(rect ? { view: rect } : {})
  };
}

const calls = (command: string) => vi.mocked(invoke).mock.calls.filter(([name]) => name === command);
const argsFor = (command: string) => calls(command).at(-1)?.[1] as Record<string, unknown>;

let admission: SourceAdmission | Error;
/** What the file-identity checks answer, so a test can say the source went missing or changed. */
let originState: 'available' | 'missing' | 'changed' = 'available';

function opened(metadata: ImageMetadata, pixel = 'pxopened') {
  return {
    metadata, documentId: 9001, isCurrent: true, backgroundPixelId: pixel,
    originalPreviewDataUrl: previewUrl, previewDataUrl: previewUrl, processingTimeMs: 1
  };
}

beforeEach(() => {
  localStorage.clear();
  admission = admissionFixture();
  originState = 'available';
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
      case 'inspect_source_origin':
        return originState;
      case 'verify_raw_source':
        return { status: originState, expectedSha256: 'b'.repeat(64) };
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
      case 'open_raw_image': {
        const rect = (args.region ?? null) as { x: number; y: number; width: number; height: number } | null;
        const width = rect ? rect.width : 12_000;
        const height = rect ? rect.height : 9_000;
        return {
          ...opened({ ...smallMetadata, filename: 'sensor.dng', format: 'DNG', width, height }, 'pxraw'),
          source: rawSource(rect), multipliers: [2, 1, 1.5], colorManaged: true, cfaPattern: 'RGGB', whiteLevel: 4095
        };
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

  describe('a camera RAW', () => {
    beforeEach(() => {
      admission = {
        ...admissionFixture({
          options: [
            {
              kind: 'openRegion',
              maxRegionPixels: 20_000_000,
              decode: { kind: 'segments', fixedBytes: 5_000_000, bytesPerPixel: 34 },
              peakBytesAtMax: 1_000_000_000
            }
          ]
        }),
        path: SENSOR,
        filename: 'sensor.dng',
        kind: 'dng',
        bitDepth: 14
      };
    });

    it('is probed like any other file, and offered a region and no reduced copy', async () => {
      render(App);
      await chooseFile(SENSOR);
      await waitFor(() => expect(screen.getByRole('dialog')).toBeTruthy());
      expect(argsFor('probe_image_source')).toEqual({ path: SENSOR });
      expect(screen.getByRole('button', { name: /Choose Region/ })).toBeTruthy();
      expect(screen.queryByRole('button', { name: /Open Reduced Copy/ })).toBeNull();
      // The decoder's real behaviour is stated, not implied.
      expect(screen.getByText(/compressed file is still read\s+whole/)).toBeTruthy();
      expect(calls('open_raw_image')).toHaveLength(0);
    });

    it('opens the region through the RAW command and keeps the view on the layer', async () => {
      render(App);
      await chooseFile(SENSOR);
      await fireEvent.click(await screen.findByRole('button', { name: /Choose Region/ }));
      await waitFor(() => expect(screen.getByAltText(/Reduced preview/)).toBeTruthy());
      await fireEvent.click(screen.getByRole('button', { name: 'Open region' }));

      await waitFor(() => expect(calls('open_raw_image')).toHaveLength(1));
      const request = argsFor('open_raw_image');
      expect(request.path).toBe(SENSOR);
      expect(request.region).toEqual(expect.objectContaining({ width: expect.any(Number), height: expect.any(Number) }));
      // Neither of the raster commands ran.
      expect(calls('open_image')).toHaveLength(0);
      expect(calls('open_image_selection')).toHaveLength(0);
      await waitFor(() => expect(screen.getByText(/sensor\.dng: region .* of the 12000 × 9000 sensor developed locally/)).toBeTruthy());

      vi.mocked(save).mockResolvedValueOnce(String.raw`C:\out\sensor.photoforge`);
      await waitFor(() =>
        expect(screen.getByRole('button', { name: /^Save project/ }).hasAttribute('disabled')).toBe(false)
      );
      await fireEvent.click(screen.getByRole('button', { name: /^Save project/ }));
      await waitFor(() => expect(calls('save_layer_project')).toHaveLength(1));
      const document = argsFor('save_layer_project').document as LayerDocument;
      const chosen = request.region as { x: number; y: number; width: number; height: number };
      // The project records which part of which sensor this is, and the layer is
      // named for what it is.
      expect(document.layers[0].raw?.view).toEqual(chosen);
      expect(document.layers[0].raw?.reference.width).toBe(12_000);
      expect(document.layers[0].name).toBe('RAW region');
      expect(document.canvasWidth).toBe(chosen.width);
    });

    it('does not send a region when the sensor opens whole', async () => {
      const probed = admission as SourceAdmission;
      admission = {
        ...probed,
        report: { ...probed.report, verdict: { kind: 'fullResolution' }, options: [{ kind: 'openFull', peakBytes: 1 }] }
      };
      render(App);
      await chooseFile(SENSOR);
      await waitFor(() => expect(calls('open_raw_image')).toHaveLength(1));
      expect(argsFor('open_raw_image').region).toBeNull();
      expect(screen.queryByRole('dialog')).toBeNull();
    });
  });

  describe('Change Source Region', () => {
    async function openRegionOfWall() {
      render(App);
      await chooseFile(WALL);
      await fireEvent.click(await screen.findByRole('button', { name: /Choose Region/ }));
      await waitFor(() => expect(screen.getByAltText(/Reduced preview/)).toBeTruthy());
      await fireEvent.click(screen.getByRole('button', { name: 'Open region' }));
      await waitFor(() => expect(screen.getByText(/opened locally/)).toBeTruthy());
      return (argsFor('open_image_selection').selection as { rect: { x: number; y: number; width: number; height: number } }).rect;
    }

    it('is offered only for a document that is a region of a file', async () => {
      admission = admissionFixture({ verdict: { kind: 'fullResolution' }, options: [{ kind: 'openFull', peakBytes: 1 }] });
      render(App);
      await chooseFile(SMALL);
      await waitFor(() => expect(screen.getByAltText('Edited preview of fixture.png')).toBeTruthy());
      expect(screen.queryByRole('button', { name: /Change Source Region/ })).toBeNull();
    });

    it('is not offered for a reduced copy, which is the whole file at lower resolution', async () => {
      render(App);
      await chooseFile(WALL);
      await fireEvent.click(await screen.findByRole('button', { name: /Open Reduced Copy/ }));
      await fireEvent.click(screen.getByRole('button', { name: 'Open reduced copy' }));
      await waitFor(() => expect(screen.getByText(/reduced copy/)).toBeTruthy());
      expect(screen.queryByRole('button', { name: /Change Source Region/ })).toBeNull();
    });

    it('reopens the chooser at the current region and says what replacing the document means', async () => {
      const current = await openRegionOfWall();
      await fireEvent.click(await screen.findByRole('button', { name: /Change Source Region/ }));

      // It went straight to choosing, after the file was verified, and nothing was opened.
      await waitFor(() => expect(screen.getByAltText(/Reduced preview/)).toBeTruthy());
      expect(calls('inspect_source_origin')).toHaveLength(1);
      expect((argsFor('inspect_source_origin').origin as SourceOrigin).source.sha256).toBe(wallSource.sha256);
      expect(screen.getByTestId('change-note').textContent).toMatch(/not carried across/);
      expect(calls('open_image_selection')).toHaveLength(1);

      // Confirming without touching the rectangle reopens exactly the current one.
      await fireEvent.click(screen.getByRole('button', { name: 'Open region' }));
      await waitFor(() => expect(calls('open_image_selection')).toHaveLength(2));
      expect((argsFor('open_image_selection').selection as { rect: unknown }).rect).toEqual(current);
    });

    it('offers nothing when the source file has gone missing, and leaves the document alone', async () => {
      await openRegionOfWall();
      originState = 'missing';
      await fireEvent.click(await screen.findByRole('button', { name: /Change Source Region/ }));
      await waitFor(() => expect(screen.getByText(/is no longer at .*untouched/)).toBeTruthy());
      expect(screen.queryByRole('dialog')).toBeNull();
      expect(calls('probe_image_source')).toHaveLength(1); // the original open's only
      expect(calls('open_image_selection')).toHaveLength(1);
    });

    it('refuses to rebind a region to a different file under the same name', async () => {
      await openRegionOfWall();
      originState = 'changed';
      await fireEvent.click(await screen.findByRole('button', { name: /Change Source Region/ }));
      await waitFor(() => expect(screen.getByText(/is not the one this region was taken from/)).toBeTruthy());
      expect(screen.queryByRole('dialog')).toBeNull();
      expect(calls('open_image_selection')).toHaveLength(1);
    });

    it('works for a camera RAW region, verifying the file by its hash', async () => {
      admission = {
        ...admissionFixture({
          options: [
            {
              kind: 'openRegion',
              maxRegionPixels: 20_000_000,
              decode: { kind: 'segments', fixedBytes: 5_000_000, bytesPerPixel: 34 },
              peakBytesAtMax: 1_000_000_000
            }
          ]
        }),
        path: SENSOR,
        filename: 'sensor.dng',
        kind: 'dng'
      };
      render(App);
      await chooseFile(SENSOR);
      await fireEvent.click(await screen.findByRole('button', { name: /Choose Region/ }));
      await waitFor(() => expect(screen.getByAltText(/Reduced preview/)).toBeTruthy());
      await fireEvent.click(screen.getByRole('button', { name: 'Open region' }));
      await waitFor(() => expect(calls('open_raw_image')).toHaveLength(1));
      const current = argsFor('open_raw_image').region;

      await fireEvent.click(await screen.findByRole('button', { name: /Change Source Region/ }));
      await waitFor(() => expect(screen.getByTestId('change-note')).toBeTruthy());
      expect(argsFor('verify_raw_source')).toMatchObject({ path: SENSOR });
      await waitFor(() => expect(screen.getByAltText(/Reduced preview/)).toBeTruthy());
      await fireEvent.click(screen.getByRole('button', { name: 'Open region' }));
      await waitFor(() => expect(calls('open_raw_image')).toHaveLength(2));
      expect(argsFor('open_raw_image').region).toEqual(current);
    });
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
