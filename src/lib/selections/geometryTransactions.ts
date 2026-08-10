import type { EditOperation } from '../types/editor';
import { cloneOperations } from '../utils/operations';
import type {
  MaskRemapResult,
  MaskRemapItem,
  RemappedMaskItem
} from './commands';
import {
  computeStageDimensions,
  extractGeometryOperations,
  geometryFingerprint
} from './geometry';
import { cloneSelectionState } from './state';
import type { GeometryOperation, MaskSnapshot, SelectionState } from './types';

const MAX_REMAP_ITEMS = 256;

export interface NewEmbeddedMask {
  operationIndex: number;
  mask: MaskSnapshot;
  width: number;
  height: number;
}

export interface GeometryRemapPlan {
  oldGeometry: GeometryOperation[];
  newGeometry: GeometryOperation[];
  newFingerprint: string;
  finalWidth: number;
  finalHeight: number;
  items: MaskRemapItem[];
  newEmbeddedMasks: NewEmbeddedMask[];
}

export interface AppliedGeometryRemap {
  operations: EditOperation[];
  selection: SelectionState;
}

export function validateGeometryRemapResult(
  plan: GeometryRemapPlan,
  result: MaskRemapResult,
  documentId: number,
  requestId: number
): RemappedMaskItem[] {
  if (!result.isCurrent || result.documentId !== documentId || result.requestId !== requestId ||
    result.finalWidth !== plan.finalWidth || result.finalHeight !== plan.finalHeight ||
    result.masks.length !== plan.items.length) {
    throw new Error('The mask geometry result is stale, incomplete, or has unexpected dimensions.');
  }
  return result.masks;
}

export function planGeometryRemap(
  sourceWidth: number,
  sourceHeight: number,
  oldOperations: EditOperation[],
  newOperations: EditOperation[],
  selection: SelectionState
): GeometryRemapPlan {
  const oldGeometry = extractGeometryOperations(oldOperations);
  const newGeometry = extractGeometryOperations(newOperations);
  const oldFinal = computeStageDimensions(sourceWidth, sourceHeight, oldGeometry);
  const newFinal = computeStageDimensions(sourceWidth, sourceHeight, newGeometry);
  const items: MaskRemapItem[] = [];

  if (selection.activeMask) {
    requireDimensions(selection.activeMask, oldFinal.width, oldFinal.height, 'active selection');
    items.push({
      key: 'active',
      mask: structuredClone(selection.activeMask),
      oldStage: oldGeometry.length,
      newStage: newGeometry.length
    });
  }
  selection.namedMasks.forEach((named, index) => {
    requireDimensions(named.mask, oldFinal.width, oldFinal.height, `named mask ${named.name}`);
    items.push({
      key: `named:${index}`,
      mask: structuredClone(named.mask),
      oldStage: oldGeometry.length,
      newStage: newGeometry.length
    });
  });

  const oldEmbedded = embeddedOperationQueues(oldOperations);
  const newEmbeddedMasks: NewEmbeddedMask[] = [];
  newOperations.forEach((operation, operationIndex) => {
    if (operation.type !== 'masked') return;
    const signature = embeddedSignature(operation);
    const previous = oldEmbedded.get(signature)?.shift();
    const newStage = geometryStageBefore(newOperations, operationIndex);
    if (previous) {
      items.push({
        key: `embedded:${operationIndex}`,
        mask: structuredClone(previous.operation.mask),
        oldStage: geometryStageBefore(oldOperations, previous.index),
        newStage
      });
    } else {
      const dimensions = computeStageDimensions(
        sourceWidth,
        sourceHeight,
        newGeometry.slice(0, newStage)
      );
      requireDimensions(operation.mask, dimensions.width, dimensions.height, `masked operation ${operationIndex + 1}`);
      newEmbeddedMasks.push({
        operationIndex,
        mask: structuredClone(operation.mask),
        width: dimensions.width,
        height: dimensions.height
      });
    }
  });

  if (items.length > MAX_REMAP_ITEMS) {
    throw new Error(`Geometry update contains ${items.length} masks; the safe limit is ${MAX_REMAP_ITEMS}.`);
  }

  return {
    oldGeometry,
    newGeometry,
    newFingerprint: geometryFingerprint(newGeometry),
    finalWidth: newFinal.width,
    finalHeight: newFinal.height,
    items,
    newEmbeddedMasks
  };
}

export function applyGeometryRemap(
  plan: GeometryRemapPlan,
  newOperations: EditOperation[],
  selection: SelectionState,
  remapped: RemappedMaskItem[]
): AppliedGeometryRemap {
  const expected = new Set(plan.items.map((item) => item.key));
  if (remapped.length !== expected.size) {
    throw new Error('Geometry remap returned an incomplete mask batch.');
  }
  const byKey = new Map<string, RemappedMaskItem>();
  for (const item of remapped) {
    if (!expected.has(item.key) || byKey.has(item.key)) {
      throw new Error('Geometry remap returned an unexpected or duplicate mask key.');
    }
    byKey.set(item.key, item);
  }

  const operations = cloneOperations(newOperations);
  for (const item of plan.items) {
    if (!item.key.startsWith('embedded:')) continue;
    const operationIndex = Number(item.key.slice('embedded:'.length));
    const operation = operations[operationIndex];
    const result = byKey.get(item.key);
    if (!result || operation?.type !== 'masked') {
      throw new Error('Geometry remap could not reconcile an embedded workflow mask.');
    }
    operation.mask = structuredClone(result.mask);
  }

  const next = cloneSelectionState(selection);
  const active = byKey.get('active');
  if (next.activeMask) {
    if (!active) throw new Error('Geometry remap omitted the active selection.');
    next.activeMask = structuredClone(active.mask);
    next.activeDiagnostics = structuredClone(active.diagnostics);
  }
  next.namedMasks = next.namedMasks.map((named, index) => {
    const result = byKey.get(`named:${index}`);
    if (!result) throw new Error(`Geometry remap omitted named mask ${named.name}.`);
    return { ...named, mask: structuredClone(result.mask) };
  });
  next.canvasWidth = plan.finalWidth;
  next.canvasHeight = plan.finalHeight;
  next.geometryOperations = structuredClone(plan.newGeometry);
  next.geometryFingerprint = plan.newFingerprint;

  return { operations, selection: next };
}

function embeddedOperationQueues(operations: EditOperation[]): Map<string, Array<{ index: number; operation: Extract<EditOperation, { type: 'masked' }> }>> {
  const queues = new Map<string, Array<{ index: number; operation: Extract<EditOperation, { type: 'masked' }> }>>();
  operations.forEach((operation, index) => {
    if (operation.type !== 'masked') return;
    const key = embeddedSignature(operation);
    const values = queues.get(key) ?? [];
    values.push({ index, operation });
    queues.set(key, values);
  });
  return queues;
}

function embeddedSignature(operation: Extract<EditOperation, { type: 'masked' }>): string {
  return JSON.stringify({
    operation: operation.operation,
    checksum: operation.mask.checksum,
    width: operation.mask.width,
    height: operation.mask.height,
    invert: operation.invert,
    maskId: operation.mask_id
  });
}

function geometryStageBefore(operations: EditOperation[], operationIndex: number): number {
  return extractGeometryOperations(operations.slice(0, operationIndex)).length;
}

function requireDimensions(mask: MaskSnapshot, width: number, height: number, label: string): void {
  if (mask.width !== width || mask.height !== height) {
    throw new Error(
      `${label} is ${mask.width}×${mask.height}, but its geometry stage is ${width}×${height}.`
    );
  }
}
