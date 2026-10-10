import { invoke } from '@tauri-apps/api/core';
import type { EditOperation } from '../types/editor';
import type { LayerDocument, ProjectLoadResult } from './types';

/**
 * What an import or an export did that is not "kept exactly". Nothing is dropped without an
 * entry. `not_imported` is for a file being opened, `not_exported` for one being written.
 */
export type PsdReportKind = 'approximated' | 'not_imported' | 'not_exported' | 'note';

export interface PsdReportEntry {
  kind: PsdReportKind;
  /** The layer it is about, when it is about one. */
  layer: string | null;
  detail: string;
}

export interface PsdImportReport {
  /** "PSD" or "PSB". */
  format: string;
  width: number;
  height: number;
  bitDepth: number;
  colorMode: string;
  layersImported: number;
  groupsImported: number;
  entries: PsdReportEntry[];
}

export interface PsdImportResult {
  project: ProjectLoadResult;
  report: PsdImportReport;
}

export interface PsdExportReport {
  /** "PSD" or "PSB". */
  format: string;
  width: number;
  height: number;
  bitDepth: number;
  colorMode: string;
  layersWritten: number;
  groupsWritten: number;
  bytes: number;
  entries: PsdReportEntry[];
}

export interface PsdExportResult {
  /** Where the file went: a document too large for a PSD is written as a PSB. */
  outputPath: string;
  report: PsdExportReport;
  processingTimeMs: number;
}

export function isPhotoshopPath(path: string): boolean {
  return /\.(psd|psb)$/i.test(path);
}

/** Opens a Photoshop document as a new, unsaved layered document. */
export function importPsdDocument(path: string, requestId: number): Promise<PsdImportResult> {
  return invoke<PsdImportResult>('import_psd_document', { path, requestId });
}

/**
 * Writes the layered document as a Photoshop file. The document is not changed. `bitDepth` null
 * is the document's own: 8 for 8-bit sRGB, 16 for high precision, 32 when it holds highlights.
 */
export function exportPsdDocument(
  outputPath: string,
  document: LayerDocument,
  operations: EditOperation[],
  bitDepth: number | null = null
): Promise<PsdExportResult> {
  return invoke<PsdExportResult>('export_psd_document', { outputPath, document, operations, bitDepth });
}

/** The counts the toast and the report heading use, for either direction. */
export function summarise(report: { entries: PsdReportEntry[] }): {
  approximated: number;
  notImported: number;
  clean: boolean;
} {
  const approximated = report.entries.filter((entry) => entry.kind === 'approximated').length;
  const notImported = report.entries.filter(
    (entry) => entry.kind === 'not_imported' || entry.kind === 'not_exported'
  ).length;
  return { approximated, notImported, clean: approximated + notImported === 0 };
}

/** One sentence for the notification after an export. */
export function exportSentence(fileName: string, report: PsdExportReport): string {
  const { approximated, notImported, clean } = summarise(report);
  const layers = `${report.layersWritten} ${report.layersWritten === 1 ? 'layer' : 'layers'}`;
  if (clean) return `${fileName} written with ${layers}; nothing was approximated or left out.`;
  const parts: string[] = [];
  if (approximated) parts.push(`${approximated} approximated`);
  if (notImported) parts.push(`${notImported} left out`);
  return `${fileName} written with ${layers}; ${parts.join(' and ')}. See the export report.`;
}

/** One sentence for the notification after an import. */
export function importSentence(fileName: string, report: PsdImportReport): string {
  const { approximated, notImported, clean } = summarise(report);
  const layers = `${report.layersImported} ${report.layersImported === 1 ? 'layer' : 'layers'}`;
  if (clean) return `${fileName} opened with ${layers}; nothing was approximated or left out.`;
  const parts: string[] = [];
  if (approximated) parts.push(`${approximated} approximated`);
  if (notImported) parts.push(`${notImported} left out`);
  return `${fileName} opened with ${layers}; ${parts.join(' and ')}. See the import report.`;
}
