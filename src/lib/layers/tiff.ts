import { invoke } from '@tauri-apps/api/core';
import type { ProjectLoadResult } from './types';
import { summarise, type PsdReportEntry } from './psd';

/**
 * What a TIFF import did, in the shape the Photoshop import report has so one dialog shows both.
 * `bitDepth` is how the file stores a sample: 1 to 64.
 */
export interface TiffImportReport {
  format: 'TIFF';
  width: number;
  height: number;
  bitDepth: number;
  colorMode: string;
  layersImported: number;
  groupsImported: number;
  entries: PsdReportEntry[];
}

export interface TiffImportResult {
  project: ProjectLoadResult;
  report: TiffImportReport;
}

/** True for a name that ends `.tif` or `.tiff`, in any case. */
export function isTiffPath(path: string): boolean {
  return /\.tiff?$/i.test(path);
}

/**
 * Opens a TIFF file as a new, unsaved document. The file is admitted from its directory before
 * any pixel is decoded; what the picture carries that PhotoForge has no place for (further
 * pages, extra channels, Photoshop layers) is named in the report.
 */
export function importTiffDocument(path: string, requestId: number): Promise<TiffImportResult> {
  return invoke<TiffImportResult>('import_tiff_document', { path, requestId });
}

/** One sentence for the notification after a TIFF import. */
export function tiffImportSentence(fileName: string, report: TiffImportReport): string {
  const { approximated, notImported, clean } = summarise(report);
  if (clean) return `${fileName} opened; nothing was approximated or left out.`;
  const parts: string[] = [];
  if (approximated) parts.push(`${approximated} approximated`);
  if (notImported) parts.push(`${notImported} left out`);
  return `${fileName} opened; ${parts.join(' and ')}. See the import report.`;
}
