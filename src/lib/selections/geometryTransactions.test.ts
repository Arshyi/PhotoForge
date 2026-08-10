import { describe, expect, it } from 'vitest';
import type { EditOperation } from '../types/editor';
import { createSelectionState } from './state';
import type { MaskSnapshot, SelectionState } from './types';
import {
  applyGeometryRemap,
  planGeometryRemap,
  validateGeometryRemapResult
} from './geometryTransactions';

function mask(width: number, height: number, checksum: string): MaskSnapshot {
  return {
    version: 1,
    width,
    height,
    encoding: 'base64_u8',
    data: '',
    checksum
  };
}

function state(active: MaskSnapshot | null): SelectionState {
  return { ...createSelectionState('doc', 4, 2), activeMask: active };
}

describe('geometry mask transactions', () => {
  it('plans active and named masks through crop then exact rotation', () => {
    const selection = state(mask(4, 2, 'fnv1a64:0000000000000001'));
    selection.namedMasks = [{
      id: 'subject', name: 'Subject', mask: mask(4, 2, 'fnv1a64:0000000000000002'),
      visible: true, locked: true, createdAt: 'a', modifiedAt: 'b'
    }];
    const next: EditOperation[] = [
      { type: 'crop', x: 0.25, y: 0, width: 0.5, height: 1, aspect_ratio: null, overlay: 'none' },
      { type: 'rotate', degrees: 90 }
    ];
    const plan = planGeometryRemap(4, 2, [], next, selection);
    expect(plan.items).toEqual([
      expect.objectContaining({ key: 'active', oldStage: 0, newStage: 2 }),
      expect.objectContaining({ key: 'named:0', oldStage: 0, newStage: 2 })
    ]);
    expect([plan.finalWidth, plan.finalHeight]).toEqual([2, 2]);
  });

  it('remaps a persistent embedded mask at its operation stage', () => {
    const embedded: EditOperation = {
      type: 'masked',
      operation: { type: 'brightness', amount: 0.1 },
      mask: mask(4, 2, 'fnv1a64:0000000000000003'),
      invert: false,
      mask_id: 'subject'
    };
    const next: EditOperation[] = [
      { type: 'rotate', degrees: 90 },
      structuredClone(embedded)
    ];
    const plan = planGeometryRemap(4, 2, [embedded], next, state(null));
    expect(plan.items[0]).toMatchObject({ key: 'embedded:1', oldStage: 0, newStage: 1 });
  });

  it('accepts a new workflow mask only when it matches the new stage', () => {
    const workflow: EditOperation[] = [
      { type: 'rotate', degrees: 90 },
      {
        type: 'masked', operation: { type: 'contrast', amount: 0.2 },
        mask: mask(2, 4, 'fnv1a64:0000000000000004'), invert: false, mask_id: null
      }
    ];
    expect(planGeometryRemap(4, 2, [], workflow, state(null)).newEmbeddedMasks).toHaveLength(1);
    const invalid = structuredClone(workflow);
    if (invalid[1].type === 'masked') invalid[1].mask = mask(4, 2, 'fnv1a64:0000000000000005');
    expect(() => planGeometryRemap(4, 2, [], invalid, state(null))).toThrow(/geometry stage/);
  });

  it('applies an all-or-error response while preserving named metadata', () => {
    const selection = state(mask(4, 2, 'fnv1a64:0000000000000001'));
    selection.namedMasks = [{
      id: 'subject', name: 'Subject', mask: mask(4, 2, 'fnv1a64:0000000000000002'),
      visible: false, locked: true, createdAt: 'created', modifiedAt: 'modified'
    }];
    const next: EditOperation[] = [{ type: 'rotate', degrees: 90 }];
    const plan = planGeometryRemap(4, 2, [], next, selection);
    const result = applyGeometryRemap(plan, next, selection, [
      { key: 'active', mask: mask(2, 4, 'fnv1a64:0000000000000011'), diagnostics: { width: 2, height: 4, selectedPixels: 1, fullySelectedPixels: 1, averageCoverage: 1, bounds: [0, 0, 1, 1], memoryBytes: 8 } },
      { key: 'named:0', mask: mask(2, 4, 'fnv1a64:0000000000000012'), diagnostics: { width: 2, height: 4, selectedPixels: 1, fullySelectedPixels: 1, averageCoverage: 1, bounds: [0, 0, 1, 1], memoryBytes: 8 } }
    ]);
    expect(result.selection.namedMasks[0]).toMatchObject({ id: 'subject', visible: false, locked: true, createdAt: 'created', modifiedAt: 'modified' });
    expect(result.selection.activeMask?.width).toBe(2);
  });

  it('rejects incomplete or duplicate backend responses atomically', () => {
    const selection = state(mask(4, 2, 'fnv1a64:0000000000000001'));
    const next: EditOperation[] = [{ type: 'rotate', degrees: 90 }];
    const plan = planGeometryRemap(4, 2, [], next, selection);
    expect(() => applyGeometryRemap(plan, next, selection, [])).toThrow(/incomplete/);
  });

  it('requires a validated backend response even when no masks need remapping', () => {
    const next: EditOperation[] = [{
      type: 'perspective',
      corners: {
        topLeft: [0, 0], topRight: [1, 0], bottomRight: [0.8, 1], bottomLeft: [0.2, 1]
      }
    }];
    const plan = planGeometryRemap(4, 2, [], next, state(null));
    expect(plan.items).toEqual([]);
    expect(validateGeometryRemapResult(plan, {
      masks: [], finalWidth: 4, finalHeight: 2,
      documentId: 1, requestId: 9, processingTimeMs: 1, isCurrent: true
    }, 1, 9)).toEqual([]);
    expect(() => validateGeometryRemapResult(plan, {
      masks: [], finalWidth: 2, finalHeight: 4,
      documentId: 1, requestId: 9, processingTimeMs: 1, isCurrent: true
    }, 1, 9)).toThrow(/unexpected dimensions/i);
  });
});
