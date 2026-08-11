import type { EditOperation } from '../types/editor';
import type { SelectionShape, SelectionState } from './types';

export interface WorkspaceMutationGuard {
  documentId: number;
  operationsFingerprint: string;
  selectionFingerprint: string;
}

export interface GeometryCommitToken {
  documentId: number;
  openRequest: number;
  generation: number;
  documentKey: string;
}

export function createWorkspaceMutationGuard(
  documentId: number,
  operations: EditOperation[],
  selection: SelectionState
): WorkspaceMutationGuard {
  return {
    documentId,
    operationsFingerprint: contentFingerprint(operations),
    selectionFingerprint: contentFingerprint(selection)
  };
}

export function isWorkspaceMutationGuardCurrent(
  guard: WorkspaceMutationGuard,
  documentId: number,
  operations: EditOperation[],
  selection: SelectionState
): boolean {
  return guard.documentId === documentId &&
    guard.operationsFingerprint === contentFingerprint(operations) &&
    guard.selectionFingerprint === contentFingerprint(selection);
}

export function isMaskIoRequestCurrent(
  ownDocumentId: number,
  ownRequestId: number,
  cancelledRequestId: number,
  currentDocumentId: number,
  currentRequestId: number,
  guard: WorkspaceMutationGuard,
  operations: EditOperation[],
  selection: SelectionState
): boolean {
  return isMaskRequestCurrent(
    ownDocumentId,
    ownRequestId,
    cancelledRequestId,
    currentDocumentId,
    currentRequestId
  ) &&
    isWorkspaceMutationGuardCurrent(guard, currentDocumentId, operations, selection);
}

export function isMaskRequestCurrent(
  ownDocumentId: number,
  ownRequestId: number,
  cancelledRequestId: number,
  currentDocumentId: number,
  currentRequestId: number
): boolean {
  return ownDocumentId === currentDocumentId &&
    ownRequestId === currentRequestId &&
    ownRequestId !== cancelledRequestId;
}

export function createGeometryCommitToken(
  documentId: number,
  openRequest: number,
  generation: number,
  documentKey: string
): GeometryCommitToken {
  return { documentId, openRequest, generation, documentKey };
}

export function isGeometryCommitTokenCurrent(
  token: GeometryCommitToken,
  documentId: number,
  openRequest: number,
  generation: number,
  documentKey: string
): boolean {
  return token.documentId === documentId && token.openRequest === openRequest &&
    token.generation === generation && token.documentKey === documentKey;
}

export function selectionCanvasRectangle(
  width: number,
  height: number
): Extract<SelectionShape, { type: 'rectangle' }> {
  if (!Number.isSafeInteger(width) || !Number.isSafeInteger(height) || width < 1 || height < 1) {
    throw new Error('Selection canvas dimensions must be positive safe integers.');
  }
  return {
    type: 'rectangle',
    start: { x: 0, y: 0 },
    end: { x: width, y: height }
  };
}

export function workspaceMutationBlocked(
  selectionBusy: boolean,
  geometryBusy: boolean,
  refineOpen: boolean
): boolean {
  return selectionBusy || geometryBusy || refineOpen;
}

export function canReplacePendingGeometry(
  pendingCoalesceKey: string | undefined,
  nextCoalesceKey: string | undefined
): boolean {
  return Boolean(
    pendingCoalesceKey && nextCoalesceKey && pendingCoalesceKey === nextCoalesceKey
  );
}

export type GeometryCoalesceIntent = 'reject' | 'queue' | 'cancel_pending';

export function geometryCoalesceIntent(
  geometryChanged: boolean,
  pendingCoalesceKey: string | undefined,
  activeCoalesceKey: string | undefined,
  geometryTransactionRunning: boolean,
  nextCoalesceKey: string | undefined,
  activeRequestCancelled = false
): GeometryCoalesceIntent {
  if (activeRequestCancelled) return 'reject';
  if (geometryTransactionRunning &&
    canReplacePendingGeometry(activeCoalesceKey, nextCoalesceKey)) {
    return 'queue';
  }
  if (canReplacePendingGeometry(pendingCoalesceKey, nextCoalesceKey)) {
    return geometryChanged ? 'queue' : 'cancel_pending';
  }
  return 'reject';
}

function contentFingerprint(value: unknown): string {
  const serialized = JSON.stringify(value);
  let first = 0x811c9dc5;
  let second = 0x9e3779b9;
  for (let index = 0; index < serialized.length; index += 1) {
    const code = serialized.charCodeAt(index);
    first = Math.imul(first ^ code, 0x01000193);
    second = Math.imul(second ^ code, 0x85ebca6b);
  }
  return `${serialized.length}:${(first >>> 0).toString(16).padStart(8, '0')}${
    (second >>> 0).toString(16).padStart(8, '0')
  }`;
}
