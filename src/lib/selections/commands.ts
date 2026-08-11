import { invoke } from '@tauri-apps/api/core';
import type { EditOperation } from '../types/editor';
import type {
  ColorRangeOptions,
  CompositionMode,
  GeometryOperation,
  MaskDiagnostics,
  MaskFile,
  MaskOperation,
  MaskProgress,
  MaskResult,
  MaskSnapshot,
  Point,
  SelectionShape
} from './types';

export interface MaskRemapItem {
  key: string;
  mask: MaskSnapshot;
  oldStage: number;
  newStage: number;
}

export interface RemappedMaskItem {
  key: string;
  mask: MaskSnapshot;
  diagnostics: MaskDiagnostics;
}

export interface MaskRemapResult {
  masks: RemappedMaskItem[];
  finalWidth: number;
  finalHeight: number;
  documentId: number;
  requestId: number;
  processingTimeMs: number;
  isCurrent: boolean;
}

export interface MaskIoRequestScope {
  documentId: number;
  requestId: number;
}

export interface ImportMaskInput extends MaskIoRequestScope {
  path: string;
}

export interface ExportMaskFileInput extends ImportMaskInput {
  document: MaskFile;
}

export interface ExportMaskPngInput extends ImportMaskInput {
  mask: MaskSnapshot;
}

export interface ImportedMaskFileResult {
  document: MaskFile;
  diagnostics: MaskDiagnostics;
}

export interface ImportedMaskPngResult {
  mask: MaskSnapshot;
  diagnostics: MaskDiagnostics;
}

export async function rasterizeSelection(input: {
  width: number;
  height: number;
  shape: SelectionShape;
  mode: CompositionMode;
  base: MaskSnapshot | null;
  documentId: number;
  requestId: number;
}): Promise<MaskResult> {
  return invoke<MaskResult>('rasterize_selection', input);
}

export async function transformSelection(input: {
  mask: MaskSnapshot;
  operation: MaskOperation;
  documentId: number;
  requestId: number;
}): Promise<MaskResult> {
  return invoke<MaskResult>('transform_selection_mask', input);
}

export async function remapSelectionMasks(input: {
  oldGeometry: GeometryOperation[];
  newGeometry: GeometryOperation[];
  items: MaskRemapItem[];
  documentId: number;
  requestId: number;
}): Promise<MaskRemapResult> {
  return invoke<MaskRemapResult>('remap_selection_masks', {
    ...input,
    oldGeometry: input.oldGeometry.map(geometryStepForBackend),
    newGeometry: input.newGeometry.map(geometryStepForBackend)
  });
}

export async function composeSelectionMasks(input: {
  base: MaskSnapshot;
  incoming: MaskSnapshot;
  mode: CompositionMode;
  documentId: number;
  requestId: number;
}): Promise<MaskResult> {
  return invoke<MaskResult>('compose_selection_masks', input);
}

export async function refineSelection(input: {
  mask: MaskSnapshot;
  operation: MaskOperation;
  edgeStrength: number;
  sampleMerged: boolean;
  operations: EditOperation[];
  documentId: number;
  requestId: number;
}): Promise<MaskResult> {
  return invoke<MaskResult>('refine_selection_mask', input);
}

export async function magicWandSelection(input: {
  point: Point;
  options: {
    tolerance: number;
    connectivity: 'four' | 'eight';
    antiAlias: boolean;
    contiguous: boolean;
  };
  mode: CompositionMode;
  base: MaskSnapshot | null;
  sampleMerged: boolean;
  operations: EditOperation[];
  documentId: number;
  requestId: number;
}): Promise<MaskResult> {
  return invoke<MaskResult>('magic_wand_selection', input);
}

export async function colorRangeSelection(input: {
  samples: Point[];
  options: ColorRangeOptions;
  mode: CompositionMode;
  base: MaskSnapshot | null;
  sampleMerged: boolean;
  operations: EditOperation[];
  documentId: number;
  requestId: number;
}): Promise<MaskResult> {
  return invoke<MaskResult>('color_range_selection', input);
}

export async function cancelMaskOperation(requestId: number): Promise<boolean> {
  return invoke<boolean>('cancel_mask_operation', { requestId });
}

export async function getMaskProgress(
  documentId: number,
  requestId: number
): Promise<MaskProgress | null> {
  return invoke<MaskProgress | null>('get_mask_progress', { documentId, requestId });
}

export async function inspectSelectionMask(mask: MaskSnapshot): Promise<MaskDiagnostics> {
  return invoke<MaskDiagnostics>('inspect_selection_mask', { mask });
}

export async function validateMaskSnapshot(mask: MaskSnapshot): Promise<MaskSnapshot> {
  return invoke<MaskSnapshot>('validate_mask_snapshot', { mask });
}

export async function importMaskFile(input: ImportMaskInput): Promise<ImportedMaskFileResult> {
  return invoke<ImportedMaskFileResult>('import_mask_file', { ...input });
}

export async function exportMaskFile(input: ExportMaskFileInput): Promise<string> {
  return invoke<string>('export_mask_file', { ...input });
}

export async function importMaskPng(input: ImportMaskInput): Promise<ImportedMaskPngResult> {
  return invoke<ImportedMaskPngResult>('import_mask_png', { ...input });
}

export async function exportMaskPng(input: ExportMaskPngInput): Promise<string> {
  return invoke<string>('export_mask_png', { ...input });
}

function geometryStepForBackend(operation: GeometryOperation): Record<string, unknown> {
  if (operation.type === 'crop') {
    return {
      type: operation.type,
      x: operation.x,
      y: operation.y,
      width: operation.width,
      height: operation.height
    };
  }
  return structuredClone(operation) as unknown as Record<string, unknown>;
}
