import { describe, expect, it } from 'vitest';
import { createSelectionState } from './state';
import { buildRefineApplyTransaction } from './refineApply';

const mask = {
  version: 1 as const,
  width: 2,
  height: 1,
  encoding: 'base64_u8' as const,
  data: 'AP8',
  checksum: 'fnv1a64:0831c907b4ea2b60'
};
const diagnostics = {
  width: 2,
  height: 1,
  averageCoverage: 0.5,
  selectedPixels: 1,
  fullySelectedPixels: 1,
  bounds: [1, 0, 1, 0] as [number, number, number, number],
  memoryBytes: 2
};

describe('refine apply transaction', () => {
  it('keeps mask-only refinement free of image operations', () => {
    const state = createSelectionState('document', 2, 1);
    const result = buildRefineApplyTransaction([], state, mask, diagnostics, {
      enabled: false, strength: 0.5, radius: 4
    });
    expect(result.includesImageEdit).toBe(false);
    expect(result.operations).toEqual([]);
    expect(result.selection.activeMask).toEqual(mask);
    expect(result.selection.overlay.visible).toBe(true);
  });

  it('appends a stage-bound masked image edit and does not mutate its inputs', () => {
    const operations = [{ type: 'brightness' as const, amount: 0.1 }];
    const state = createSelectionState('document', 2, 1);
    const result = buildRefineApplyTransaction(operations, state, mask, diagnostics, {
      enabled: true, strength: 0.75, radius: 6
    });
    expect(result.includesImageEdit).toBe(true);
    expect(result.operations).toHaveLength(2);
    expect(result.operations[1]).toEqual({
      type: 'masked',
      operation: { type: 'decontaminate_colors', enabled: true, strength: 0.75, radius: 6 },
      mask,
      invert: false,
      mask_id: null
    });
    expect(operations).toEqual([{ type: 'brightness', amount: 0.1 }]);
    expect(state.activeMask).toBeNull();
    expect(result.selection.activeMask).not.toBe(mask);
  });

  it.each([
    { enabled: true, strength: Number.NaN, radius: 4 },
    { enabled: true, strength: 1.1, radius: 4 },
    { enabled: true, strength: 0.5, radius: 0 },
    { enabled: true, strength: 0.5, radius: 4.5 }
  ])('rejects invalid enabled settings %#', (settings) => {
    expect(() => buildRefineApplyTransaction(
      [], createSelectionState('document', 2, 1), mask, diagnostics, settings
    )).toThrow('Decontaminate Colors settings are invalid.');
  });
});
