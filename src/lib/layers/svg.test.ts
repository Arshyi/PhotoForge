import { describe, expect, it } from 'vitest';
import { isSvgPath, svgExportSentence, svgImportSentence, type SvgExportReport, type SvgImportReport } from './svg';

const report = (entries: SvgExportReport['entries'], layers = 3): SvgExportReport => ({
  format: 'SVG',
  width: 100,
  height: 50,
  layersWritten: layers,
  groupsWritten: 0,
  bytes: 1000,
  entries
});

describe('SVG export helpers', () => {
  it('recognises an SVG name by its extension, in any case', () => {
    expect(isSvgPath('C:\\art\\poster.svg')).toBe(true);
    expect(isSvgPath('/home/a/poster.SVG')).toBe(true);
    expect(isSvgPath('poster.svg.png')).toBe(false);
    expect(isSvgPath('poster.svgz')).toBe(false);
  });

  it('says when nothing was approximated or left out', () => {
    expect(svgExportSentence('a.svg', report([]))).toBe(
      'a.svg written with 3 shapes; nothing was approximated or left out.'
    );
    expect(svgExportSentence('a.svg', report([], 1))).toContain('1 shape;');
  });

  it('counts what was approximated and what was left out', () => {
    const entries: SvgExportReport['entries'] = [
      { kind: 'approximated', layer: 'A', detail: 'x' },
      { kind: 'not_exported', layer: 'B', detail: 'y' },
      { kind: 'not_exported', layer: 'C', detail: 'z' }
    ];
    expect(svgExportSentence('a.svg', report(entries))).toBe(
      'a.svg written with 3 shapes; 1 approximated and 2 left out. See the export report.'
    );
  });
});

describe('SVG import helpers', () => {
  const imported = (entries: SvgImportReport['entries'], layers = 2): SvgImportReport => ({
    format: 'SVG',
    width: 10,
    height: 10,
    layersImported: layers,
    groupsImported: 0,
    entries
  });

  it('says when nothing was approximated or left out', () => {
    expect(svgImportSentence('a.svg', imported([]))).toBe(
      'a.svg opened with 2 shapes; nothing was approximated or left out.'
    );
    expect(svgImportSentence('a.svg', imported([], 1))).toContain('1 shape;');
  });

  it('counts what was approximated and what was left out', () => {
    const entries: SvgImportReport['entries'] = [
      { kind: 'approximated', layer: 'A', detail: 'x' },
      { kind: 'not_imported', layer: 'B', detail: 'y' }
    ];
    expect(svgImportSentence('a.svg', imported(entries))).toBe(
      'a.svg opened with 2 shapes; 1 approximated and 1 left out. See the import report.'
    );
  });
});
