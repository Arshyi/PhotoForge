import { invoke } from '@tauri-apps/api/core';
import type { ColorExportOptions, EditOperation, ExportProfile, ExportResult, PreviewResult } from '../types/editor';
import type { MaskSnapshot } from '../selections/types';
import type {
  LayerDocument,
  LayerMaskResult,
  LayerPixelsResult,
  LayerStoreReport,
  LayerThumbnailResult,
  ProjectLoadResult,
  ProjectSaveResult
} from './types';

export function renderLayerComposite(
  document: LayerDocument,
  operations: EditOperation[],
  documentId: number,
  requestId: number
): Promise<PreviewResult> {
  return invoke<PreviewResult>('render_layer_composite', {
    document,
    operations,
    documentId,
    requestId
  });
}

export function exportLayerComposite(
  outputPath: string,
  document: LayerDocument,
  operations: EditOperation[],
  profile: ExportProfile,
  color?: ColorExportOptions
): Promise<ExportResult> {
  return invoke<ExportResult>('export_layer_composite', {
    outputPath,
    document,
    operations,
    profile,
    ...(color ? { color } : {})
  });
}

export function importLayerImage(path: string): Promise<LayerPixelsResult> {
  return invoke<LayerPixelsResult>('import_layer_image', { path });
}

export function createLayerPixels(width: number, height: number): Promise<LayerPixelsResult> {
  return invoke<LayerPixelsResult>('create_layer_pixels', { width, height });
}

export function mergeLayerPixels(
  document: LayerDocument,
  layerIds: string[]
): Promise<LayerPixelsResult> {
  return invoke<LayerPixelsResult>('merge_layer_pixels', { document, layerIds });
}

export function flattenLayerDocument(document: LayerDocument): Promise<LayerPixelsResult> {
  return invoke<LayerPixelsResult>('flatten_layer_document', { document });
}

export function rasterizeLayerTransform(
  document: LayerDocument,
  layerId: string
): Promise<LayerPixelsResult> {
  return invoke<LayerPixelsResult>('rasterize_layer_transform', {
    request: { document, layerId }
  });
}

export function applyOperationsToLayer(
  document: LayerDocument,
  layerId: string,
  operations: EditOperation[]
): Promise<LayerPixelsResult> {
  return invoke<LayerPixelsResult>('apply_operations_to_layer', {
    document,
    layerId,
    operations
  });
}

export function renderLayerThumbnail(
  document: LayerDocument,
  layerId: string,
  maxEdge: number
): Promise<LayerThumbnailResult> {
  return invoke<LayerThumbnailResult>('render_layer_thumbnail', { document, layerId, maxEdge });
}

export function layerMaskFromSelection(
  document: LayerDocument,
  layerId: string,
  selection: MaskSnapshot
): Promise<LayerMaskResult> {
  return invoke<LayerMaskResult>('layer_mask_from_selection', { document, layerId, selection });
}

export function selectionFromLayerMask(
  document: LayerDocument,
  layerId: string
): Promise<LayerMaskResult> {
  return invoke<LayerMaskResult>('selection_from_layer_mask', { document, layerId });
}

export function createLayerMask(
  document: LayerDocument,
  layerId: string,
  filled: boolean
): Promise<LayerMaskResult> {
  return invoke<LayerMaskResult>('create_layer_mask', { document, layerId, filled });
}

export function validateLayerDocument(document: LayerDocument): Promise<number> {
  return invoke<number>('validate_layer_document', { document });
}

export function retainLayerPixels(pixelIds: string[], documentId: number): Promise<LayerStoreReport> {
  return invoke<LayerStoreReport>('retain_layer_pixels', { pixelIds, documentId });
}

export function layerStoreReport(): Promise<LayerStoreReport> {
  return invoke<LayerStoreReport>('layer_store_report');
}

export function saveLayerProject(
  outputPath: string,
  document: LayerDocument,
  operations: EditOperation[],
  createdAt: string,
  modifiedAt: string
): Promise<ProjectSaveResult> {
  return invoke<ProjectSaveResult>('save_layer_project', {
    outputPath,
    document,
    operations,
    createdAt,
    modifiedAt
  });
}

export function loadLayerProject(path: string, requestId: number): Promise<ProjectLoadResult> {
  return invoke<ProjectLoadResult>('load_layer_project', { path, requestId });
}

export interface RecoveryRecord {
  projectPath: string | null;
  documentName: string;
  savedAt: string;
  snapshotPath: string;
  bytes: number;
}

export function writeRecoverySnapshot(
  document: LayerDocument,
  operations: EditOperation[],
  projectPath: string | null,
  documentName: string,
  savedAt: string
): Promise<RecoveryRecord> {
  return invoke<RecoveryRecord>('write_recovery_snapshot', {
    document,
    operations,
    projectPath,
    documentName,
    savedAt
  });
}

export function listRecoverySnapshots(): Promise<{ snapshots: RecoveryRecord[] }> {
  return invoke<{ snapshots: RecoveryRecord[] }>('list_recovery_snapshots');
}

export function restoreRecoverySnapshot(path: string, requestId: number): Promise<ProjectLoadResult> {
  return invoke<ProjectLoadResult>('restore_recovery_snapshot', { path, requestId });
}

export function discardRecoverySnapshot(path: string | null): Promise<number> {
  return invoke<number>('discard_recovery_snapshot', { path });
}

export function planLayerWorkflowSteps(
  document: LayerDocument,
  steps: unknown[]
): Promise<{ targets: string[]; steps: number }> {
  return invoke<{ targets: string[]; steps: number }>('plan_layer_workflow', { document, steps });
}
