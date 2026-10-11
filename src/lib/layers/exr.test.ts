import { beforeEach, describe, expect, it, vi } from 'vitest';
import { invoke } from '@tauri-apps/api/core';
import {
  exportExrDocument,
  exrExportSentence,
  exrImportSentence,
  importExrDocument,
  isExrPath,
  type ExrExportReport,
  type ExrImportReport
} from './exr';
import { createDocument } from './tree';

vi.mock('@tauri-apps/api/core', () => ({ invoke: vi.fn() }));

const imported = (entries: ExrImportReport['entries'], layers = 1): ExrImportReport => ({
  format: 'OpenEXR',
  width: 10,
  height: 10,
  bitDepth: 16,
  colorMode: 'RGB',
  layersImported: layers,
  groupsImported: 0,
  entries
});

const written = (entries: ExrExportReport['entries'], bitDepth = 32): ExrExportReport => ({
  format: 'OpenEXR',
  width: 10,
  height: 10,
  bitDepth,
  colorMode: 'RGBA',
  layersWritten: 1,
  groupsWritten: 0,
  bytes: 100,
  entries
});

describe('OpenEXR helpers', () => {
  beforeEach(() => vi.mocked(invoke).mockReset().mockResolvedValue({}));

  it('recognises an EXR name by its extension, in any case', () => {
    expect(isExrPath('C:\\renders\\shot.exr')).toBe(true);
    expect(isExrPath('/home/a/SHOT.EXR')).toBe(true);
    expect(isExrPath('shot.exr.png')).toBe(false);
    expect(isExrPath('shot.exrx')).toBe(false);
  });

  it('says when nothing was approximated or left out of an import', () => {
    expect(exrImportSentence('a.exr', imported([]))).toBe(
      'a.exr opened with 1 layer; nothing was approximated or left out.'
    );
    expect(exrImportSentence('a.exr', imported([], 3))).toContain('3 layers;');
  });

  it('counts what an import approximated and left out', () => {
    const entries: ExrImportReport['entries'] = [
      { kind: 'approximated', layer: null, detail: 'x' },
      { kind: 'not_imported', layer: null, detail: 'y' },
      { kind: 'note', layer: null, detail: 'z' }
    ];
    expect(exrImportSentence('a.exr', imported(entries))).toBe(
      'a.exr opened with 1 layer; 1 approximated and 1 left out. See the import report.'
    );
  });

  it('names the precision an export used', () => {
    expect(exrExportSentence('a.exr', written([]))).toBe(
      'a.exr written as one single precision picture; nothing was approximated or left out.'
    );
    expect(exrExportSentence('a.exr', written([], 16))).toContain('half float');
    const entries: ExrExportReport['entries'] = [
      { kind: 'not_exported', layer: null, detail: 'composited' }
    ];
    expect(exrExportSentence('a.exr', written(entries))).toBe(
      'a.exr written as one single precision picture; 1 left out. See the export report.'
    );
  });

  it('calls the commands with the arguments the backend takes', async () => {
    await importExrDocument('C:\\a.exr', 7);
    expect(invoke).toHaveBeenCalledWith('import_exr_document', { path: 'C:\\a.exr', requestId: 7 });
    const document = createDocument(4, 4, [], 'linear_srgb_f32');
    await exportExrDocument('C:\\b.exr', document, []);
    expect(invoke).toHaveBeenLastCalledWith('export_exr_document', {
      outputPath: 'C:\\b.exr',
      document,
      operations: []
    });
    await exportExrDocument('C:\\b.exr', document, [], 16, 'keep_all');
    expect(invoke).toHaveBeenLastCalledWith('export_exr_document', {
      outputPath: 'C:\\b.exr',
      document,
      operations: [],
      bitDepth: 16,
      metadataPolicy: 'keep_all'
    });
  });
});
