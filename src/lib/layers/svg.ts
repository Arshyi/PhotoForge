import { invoke } from '@tauri-apps/api/core';
import type { EditOperation } from '../types/editor';
import type { LayerDocument } from './types';
import { summarise, type PsdReportEntry } from './psd';

/**
 * What an SVG export did, in the same shape as a Photoshop export's report so one dialog shows
 * both. `layersWritten` counts shape layers; an SVG holds no bit depth or colour mode.
 */
export interface SvgExportReport {
  format: 'SVG';
  width: number;
  height: number;
  layersWritten: number;
  groupsWritten: number;
  bytes: number;
  entries: PsdReportEntry[];
}

export interface SvgExportResult {
  outputPath: string;
  report: SvgExportReport;
  processingTimeMs: number;
}

export function isSvgPath(path: string): boolean {
  return /\.svg$/i.test(path);
}

/**
 * Writes the shape layers of the document as an SVG. Pixels, text, masks and adjustments are not
 * rasterised into it; they are named in the report. The document is not changed.
 */
export function exportSvgDocument(
  outputPath: string,
  document: LayerDocument,
  operations: EditOperation[]
): Promise<SvgExportResult> {
  return invoke<SvgExportResult>('export_svg_document', { outputPath, document, operations });
}

/** One sentence for the notification after an SVG export. */
export function svgExportSentence(fileName: string, report: SvgExportReport): string {
  const { approximated, notImported, clean } = summarise(report);
  const shapes = `${report.layersWritten} ${report.layersWritten === 1 ? 'shape' : 'shapes'}`;
  if (clean) return `${fileName} written with ${shapes}; nothing was approximated or left out.`;
  const parts: string[] = [];
  if (approximated) parts.push(`${approximated} approximated`);
  if (notImported) parts.push(`${notImported} left out`);
  return `${fileName} written with ${shapes}; ${parts.join(' and ')}. See the export report.`;
}
