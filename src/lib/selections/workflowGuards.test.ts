import { describe, expect, it } from 'vitest';
import type { EditOperation } from '../types/editor';
import { computeStageDimensions } from './geometry';
import { createSelectionState } from './state';
import {
  createGeometryCommitToken,
  createWorkspaceMutationGuard,
  isGeometryCommitTokenCurrent,
  isWorkspaceMutationGuardCurrent,
  selectionCanvasRectangle,
  workspaceMutationBlocked
} from './workflowGuards';

describe('selection workflow guards', () => {
  it('builds Select All from the exact transformed non-square canvas', () => {
    const dimensions = computeStageDimensions(12, 8, [
      {
        type: 'crop', x: 0, y: 0, width: 0.75, height: 1,
        aspect_ratio: null, overlay: 'none'
      },
      { type: 'rotate', degrees: 90 }
    ]);
    expect(dimensions).toEqual({ width: 8, height: 9 });
    expect(selectionCanvasRectangle(dimensions.width, dimensions.height)).toEqual({
      type: 'rectangle',
      start: { x: 0, y: 0 },
      end: { x: 8, y: 9 }
    });
  });

  it('rejects delayed geometry work after a document or generation switch', () => {
    const token = createGeometryCommitToken(7, 12, 3, 'document-a');
    expect(isGeometryCommitTokenCurrent(token, 7, 12, 3, 'document-a')).toBe(true);
    expect(isGeometryCommitTokenCurrent(token, 8, 13, 4, 'document-b')).toBe(false);
    expect(isGeometryCommitTokenCurrent(token, 7, 12, 4, 'document-a')).toBe(false);
  });

  it('rejects async mask results after selection or edit state changes', () => {
    const operations: EditOperation[] = [{ type: 'brightness', amount: 0.1 }];
    const selection = createSelectionState('document-a', 8, 9);
    const guard = createWorkspaceMutationGuard(7, operations, selection);
    expect(isWorkspaceMutationGuardCurrent(guard, 7, operations, selection)).toBe(true);
    expect(isWorkspaceMutationGuardCurrent(guard, 7, [{ type: 'brightness', amount: 0.2 }], selection)).toBe(false);
    expect(isWorkspaceMutationGuardCurrent(guard, 7, operations, { ...selection, mode: 'add' })).toBe(false);
    expect(isWorkspaceMutationGuardCurrent(guard, 8, operations, selection)).toBe(false);
  });

  it('blocks mutations during mask, geometry, or refinement work', () => {
    expect(workspaceMutationBlocked(false, false, false)).toBe(false);
    expect(workspaceMutationBlocked(true, false, false)).toBe(true);
    expect(workspaceMutationBlocked(false, true, false)).toBe(true);
    expect(workspaceMutationBlocked(false, false, true)).toBe(true);
  });
});
