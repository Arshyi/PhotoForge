import { describe, expect, it } from 'vitest';
import type { EditOperation } from '../types/editor';
import { computeStageDimensions } from './geometry';
import { createSelectionState } from './state';
import {
  canReplacePendingGeometry,
  createGeometryCommitToken,
  createWorkspaceMutationGuard,
  geometryCoalesceIntent,
  isGeometryCommitTokenCurrent,
  isMaskIoRequestCurrent,
  isMaskRequestCurrent,
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

  it('rejects mask file results after cancellation, request replacement, or workspace mutation', () => {
    const operations: EditOperation[] = [{ type: 'brightness', amount: 0.1 }];
    const selection = createSelectionState('document-a', 8, 9);
    const guard = createWorkspaceMutationGuard(7, operations, selection);

    expect(isMaskIoRequestCurrent(7, 12, 0, 7, 12, guard, operations, selection)).toBe(true);
    expect(isMaskIoRequestCurrent(7, 12, 12, 7, 12, guard, operations, selection)).toBe(false);
    expect(isMaskIoRequestCurrent(7, 12, 0, 7, 13, guard, operations, selection)).toBe(false);
    expect(isMaskIoRequestCurrent(7, 12, 0, 8, 12, guard, operations, selection)).toBe(false);
    expect(isMaskIoRequestCurrent(
      7,
      12,
      0,
      7,
      12,
      guard,
      [{ type: 'brightness', amount: 0.2 }],
      selection
    )).toBe(false);
  });

  it('rejects every mask result after cancellation, document switch, or request replacement', () => {
    expect(isMaskRequestCurrent(7, 12, 0, 7, 12)).toBe(true);
    expect(isMaskRequestCurrent(7, 12, 12, 7, 12)).toBe(false);
    expect(isMaskRequestCurrent(7, 12, 0, 8, 12)).toBe(false);
    expect(isMaskRequestCurrent(7, 12, 0, 7, 13)).toBe(false);
  });

  it('blocks mutations during mask, geometry, or refinement work', () => {
    expect(workspaceMutationBlocked(false, false, false)).toBe(false);
    expect(workspaceMutationBlocked(true, false, false)).toBe(true);
    expect(workspaceMutationBlocked(false, true, false)).toBe(true);
    expect(workspaceMutationBlocked(false, false, true)).toBe(true);
  });

  it('allows only the same coalesced geometry control to replace a queued value', () => {
    expect(canReplacePendingGeometry('straighten', 'straighten')).toBe(true);
    expect(canReplacePendingGeometry('straighten', 'rotate')).toBe(false);
    expect(canReplacePendingGeometry('straighten', undefined)).toBe(false);

    let pending: EditOperation[] = [{ type: 'straighten', degrees: 0.1 }];
    const latest: EditOperation[] = [{ type: 'straighten', degrees: 7.5 }];
    if (canReplacePendingGeometry('straighten', 'straighten')) pending = latest;
    expect(pending).toEqual(latest);
  });

  it('queues the final same-control value behind an active geometry transaction', () => {
    const first: EditOperation[] = [{
      type: 'lens_correction', distortion: 0.1, vignetting: 0, chromatic_aberration: 0
    }];
    const latest: EditOperation[] = [{
      type: 'lens_correction', distortion: 0.6, vignetting: 0, chromatic_aberration: 0
    }];
    let active: EditOperation[] | null = first;
    let pending: EditOperation[] | null = null;

    expect(geometryCoalesceIntent(
      true, undefined, 'lens_correction', true, 'lens_correction'
    )).toBe('queue');
    if (geometryCoalesceIntent(
      true, undefined, 'lens_correction', true, 'lens_correction'
    ) === 'queue') pending = latest;

    active = pending;
    pending = null;
    expect(active).toEqual(latest);
    expect(pending).toBeNull();
  });

  it('does not admit unrelated or non-geometry edits during active coalescing', () => {
    expect(geometryCoalesceIntent(
      true, undefined, 'lens_correction', true, 'straighten'
    )).toBe('reject');
    expect(geometryCoalesceIntent(
      true, undefined, 'lens_correction', true, undefined
    )).toBe('reject');
    expect(geometryCoalesceIntent(
      true, undefined, 'lens_correction', false, 'lens_correction'
    )).toBe('reject');
  });

  it('cancels a queued value when the same control returns to committed baseline before start', () => {
    let pending: EditOperation[] | null = [{
      type: 'lens_correction', distortion: 0.6, vignetting: 0, chromatic_aberration: 0
    }];
    let timerScheduled = true;
    const intent = geometryCoalesceIntent(
      false, 'lens_correction', undefined, false, 'lens_correction'
    );
    if (intent === 'cancel_pending') {
      pending = null;
      timerScheduled = false;
    }
    expect(intent).toBe('cancel_pending');
    expect(pending).toBeNull();
    expect(timerScheduled).toBe(false);
  });

  it('queues committed baseline behind an active value so the final intent wins', () => {
    const baseline: EditOperation[] = [];
    let pending: EditOperation[] | null = null;
    const intent = geometryCoalesceIntent(
      false, undefined, 'lens_correction', true, 'lens_correction'
    );
    if (intent === 'queue') pending = baseline;
    expect(intent).toBe('queue');
    expect(pending).toEqual(baseline);
  });

  it('drops the queued final value and rejects further coalescing after cancellation', () => {
    let pending: EditOperation[] | null = [{
      type: 'lens_correction', distortion: 0.6, vignetting: 0, chromatic_aberration: 0
    }];
    let timerScheduled = true;
    const activeRequestCancelled = true;

    if (activeRequestCancelled) {
      pending = null;
      timerScheduled = false;
    }

    expect(pending).toBeNull();
    expect(timerScheduled).toBe(false);
    expect(geometryCoalesceIntent(
      true, undefined, 'lens_correction', true, 'lens_correction', activeRequestCancelled
    )).toBe('reject');
  });
});
