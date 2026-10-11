import { invoke } from '@tauri-apps/api/core';
import type { EditOperation, MetadataPolicy, MetadataReport } from '../types/editor';
import type { LayerDocument, ProjectLoadResult } from './types';
import { summarise, type PsdReportEntry } from './psd';

/**
 * What an OpenEXR import did, in the shape the Photoshop import report has so one dialog shows
 * both. `bitDepth` is how the file stores its colour: 16 (half) or 32 (single precision).
 */
export interface ExrImportReport {
  format: 'OpenEXR';
  width: number;
  height: number;
  bitDepth: number;
  colorMode: string;
  layersImported: number;
  groupsImported: number;
  entries: PsdReportEntry[];
}

export interface ExrImportResult {
  project: ProjectLoadResult;
  report: ExrImportReport;
}

/** What an OpenEXR export did. `layersWritten` is always one: the composite. */
export interface ExrExportReport {
  format: 'OpenEXR';
  width: number;
  height: number;
  bitDepth: number;
  colorMode: string;
  layersWritten: number;
  groupsWritten: number;
  bytes: number;
  entries: PsdReportEntry[];
}

export interface ExrExportResult {
  outputPath: string;
  report: ExrExportReport;
  /** An OpenEXR file has no place for EXIF, XMP or IPTC; this is the account of that. */
  metadata?: MetadataReport;
  processingTimeMs: number;
}

export function isExrPath(path: string): boolean {
  return /\.exr$/i.test(path);
}

/**
 * Opens an OpenEXR file as a new, unsaved high-precision document. The file is admitted from its
 * header before any pixel is decoded; what the picture carries that PhotoForge has no place for is
 * named in the report.
 */
export function importExrDocument(path: string, requestId: number): Promise<ExrImportResult> {
  return invoke<ExrImportResult>('import_exr_document', { path, requestId });
}

/**
 * Writes the document, composited, as an OpenEXR file of scene-linear light. `bitDepth` 16 writes
 * half float; null (the default) writes single precision. The document is not changed.
 */
export function exportExrDocument(
  outputPath: string,
  document: LayerDocument,
  operations: EditOperation[],
  bitDepth: 16 | 32 | null = null,
  metadataPolicy?: MetadataPolicy
): Promise<ExrExportResult> {
  return invoke<ExrExportResult>('export_exr_document', {
    outputPath,
    document,
    operations,
    ...(bitDepth ? { bitDepth } : {}),
    ...(metadataPolicy ? { metadataPolicy } : {})
  });
}

/** One sentence for the notification after an OpenEXR import. */
export function exrImportSentence(fileName: string, report: ExrImportReport): string {
  const { approximated, notImported, clean } = summarise(report);
  const pictures = `${report.layersImported} ${report.layersImported === 1 ? 'layer' : 'layers'}`;
  if (clean) return `${fileName} opened with ${pictures}; nothing was approximated or left out.`;
  const parts: string[] = [];
  if (approximated) parts.push(`${approximated} approximated`);
  if (notImported) parts.push(`${notImported} left out`);
  return `${fileName} opened with ${pictures}; ${parts.join(' and ')}. See the import report.`;
}

/** One sentence for the notification after an OpenEXR export. */
export function exrExportSentence(fileName: string, report: ExrExportReport): string {
  const { approximated, notImported, clean } = summarise(report);
  const depth = report.bitDepth === 16 ? 'half float' : 'single precision';
  if (clean) return `${fileName} written as one ${depth} picture; nothing was approximated or left out.`;
  const parts: string[] = [];
  if (approximated) parts.push(`${approximated} approximated`);
  if (notImported) parts.push(`${notImported} left out`);
  return `${fileName} written as one ${depth} picture; ${parts.join(' and ')}. See the export report.`;
}
