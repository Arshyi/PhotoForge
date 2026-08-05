import { invoke } from '@tauri-apps/api/core';
import type { EditOperation } from '../types/editor';
import type {
  ColorRangeOptions,
  CompositionMode,
  MaskDiagnostics,
  MaskFile,
  MaskOperation,
  MaskResult,
  MaskSnapshot,
  Point,
  SelectionShape
} from './types';

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

export async function inspectSelectionMask(mask: MaskSnapshot): Promise<MaskDiagnostics> {
  return invoke<MaskDiagnostics>('inspect_selection_mask', { mask });
}

export async function importMaskFile(path: string): Promise<MaskFile> {
  return invoke<MaskFile>('import_mask_file', { path });
}

export async function exportMaskFile(path: string, document: MaskFile): Promise<string> {
  return invoke<string>('export_mask_file', { path, document });
}

export async function importMaskPng(path: string): Promise<MaskSnapshot> {
  return invoke<MaskSnapshot>('import_mask_png', { path });
}

export async function exportMaskPng(path: string, mask: MaskSnapshot): Promise<string> {
  return invoke<string>('export_mask_png', { path, mask });
}
