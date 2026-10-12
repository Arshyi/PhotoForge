import { beforeEach, describe, expect, it, vi } from 'vitest';
import { invoke } from '@tauri-apps/api/core';
import {
  importTiffDocument,
  isTiffPath,
  tiffImportSentence,
  type TiffImportReport
} from './tiff';

vi.mock('@tauri-apps/api/core', () => ({ invoke: vi.fn() }));

const imported = (entries: TiffImportReport['entries']): TiffImportReport => ({
  format: 'TIFF',
  width: 10,
  height: 10,
  bitDepth: 16,
  colorMode: 'RGB',
  layersImported: 1,
  groupsImported: 0,
  entries
});

describe('TIFF helpers', () => {
  beforeEach(() => vi.mocked(invoke).mockReset().mockResolvedValue({}));

  it('recognises a TIFF name by its extension, either spelling, in any case', () => {
    expect(isTiffPath('C:\\scans\\page.tif')).toBe(true);
    expect(isTiffPath('/home/a/PAGE.TIFF')).toBe(true);
    expect(isTiffPath('page.tif.png')).toBe(false);
    expect(isTiffPath('page.tiffx')).toBe(false);
    expect(isTiffPath('tif')).toBe(false);
  });

  it('says when nothing was approximated or left out', () => {
    expect(tiffImportSentence('a.tif', imported([]))).toBe(
      'a.tif opened; nothing was approximated or left out.'
    );
  });

  it('counts what an import approximated and left out, and ignores notes', () => {
    const entries: TiffImportReport['entries'] = [
      { kind: 'approximated', layer: null, detail: 'x' },
      { kind: 'not_imported', layer: null, detail: 'y' },
      { kind: 'not_imported', layer: null, detail: 'y2' },
      { kind: 'note', layer: null, detail: 'z' }
    ];
    expect(tiffImportSentence('a.tif', imported(entries))).toBe(
      'a.tif opened; 1 approximated and 2 left out. See the import report.'
    );
    expect(tiffImportSentence('a.tif', imported([{ kind: 'note', layer: null, detail: 'z' }]))).toContain(
      'nothing was approximated'
    );
  });

  it('opens by the import command, with the request id', async () => {
    await importTiffDocument('C:\\scans\\page.tif', 7);
    expect(invoke).toHaveBeenCalledWith('import_tiff_document', {
      path: 'C:\\scans\\page.tif',
      requestId: 7
    });
  });
});
