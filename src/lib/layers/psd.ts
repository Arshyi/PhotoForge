import { invoke } from '@tauri-apps/api/core';
import type { ProjectLoadResult } from './types';

/** What an import did that is not "kept exactly". Nothing is dropped without an entry. */
export type PsdReportKind = 'approximated' | 'not_imported' | 'note';

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

export function isPhotoshopPath(path: string): boolean {
  return /\.(psd|psb)$/i.test(path);
}

/** Opens a Photoshop document as a new, unsaved layered document. */
export function importPsdDocument(path: string, requestId: number): Promise<PsdImportResult> {
  return invoke<PsdImportResult>('import_psd_document', { path, requestId });
}

/** The counts the toast and the report heading use. */
export function summarise(report: PsdImportReport): {
  approximated: number;
  notImported: number;
  clean: boolean;
} {
  const approximated = report.entries.filter((entry) => entry.kind === 'approximated').length;
  const notImported = report.entries.filter((entry) => entry.kind === 'not_imported').length;
  return { approximated, notImported, clean: approximated + notImported === 0 };
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
