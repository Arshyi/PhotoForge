import { describe, expect, it } from 'vitest';
import {
  aspectRatio,
  applyAspect,
  centerRect,
  clampRect,
  initialRegion,
  largestRegion,
  megapixels,
  moveRect,
  reducedSize,
  regionPeakBytes,
  regionStatus,
  resizeRect,
  type Handle
} from './region';
import type { AdmissionReport, Rect } from './types';

const SOURCE = { width: 12_000, height: 9_000 };

/** A small deterministic generator, so a failure names a seed that reproduces it. */
function random(seed: number) {
  let state = seed >>> 0 || 1;
  return () => {
    state ^= state << 13;
    state >>>= 0;
    state ^= state >>> 17;
    state ^= state << 5;
    state >>>= 0;
    return state / 0x1_0000_0000;
  };
}

function report(overrides: Partial<AdmissionReport> = {}): AdmissionReport {
  return {
    verdict: { kind: 'regionRequired' },
    options: [
      {
        kind: 'openRegion',
        maxRegionPixels: 20_000_000,
        decode: { kind: 'rows' },
        peakBytesAtMax: 900_000_000
      }
    ],
    fullPeakBytes: 5_000_000_000,
    budgetBytes: 4_000_000_000,
    availableBytes: 8_000_000_000,
    regionCost: { openingFixed: 1_000_000, openingPerPixel: 20, editingFixed: 40_960_000, editingPerPixel: 44 },
    outOfCore: 'not available',
    ...overrides
  };
}

function inside(rect: Rect) {
  return (
    rect.width >= 1 &&
    rect.height >= 1 &&
    rect.x >= 0 &&
    rect.y >= 0 &&
    rect.x + rect.width <= SOURCE.width &&
    rect.y + rect.height <= SOURCE.height &&
    Number.isInteger(rect.x) &&
    Number.isInteger(rect.y) &&
    Number.isInteger(rect.width) &&
    Number.isInteger(rect.height)
  );
}

describe('clamping and moving', () => {
  it('forces any rectangle inside the source and at least one pixel', () => {
    for (const rect of [
      { x: -50, y: -50, width: 100, height: 100 },
      { x: 11_990, y: 8_990, width: 500, height: 500 },
      { x: 5, y: 5, width: 0, height: -3 },
      { x: 0, y: 0, width: 99_999, height: 99_999 },
      { x: Number.MAX_SAFE_INTEGER, y: 0, width: 10, height: 10 }
    ]) {
      expect(inside(clampRect(rect, SOURCE.width, SOURCE.height))).toBe(true);
    }
  });

  it('does not move a rectangle off any edge', () => {
    const rect = { x: 100, y: 100, width: 400, height: 300 };
    expect(moveRect(rect, -1_000, 0, SOURCE.width, SOURCE.height).x).toBe(0);
    expect(moveRect(rect, 0, -1_000, SOURCE.width, SOURCE.height).y).toBe(0);
    expect(moveRect(rect, 99_999, 0, SOURCE.width, SOURCE.height).x).toBe(SOURCE.width - 400);
    expect(moveRect(rect, 0, 99_999, SOURCE.width, SOURCE.height).y).toBe(SOURCE.height - 300);
    expect(moveRect(rect, 10, 20, SOURCE.width, SOURCE.height)).toEqual({ ...rect, x: 110, y: 120 });
  });

  it('centres a rectangle', () => {
    const centred = centerRect({ x: 0, y: 0, width: 2_000, height: 1_000 }, SOURCE.width, SOURCE.height);
    expect(centred).toEqual({ x: 5_000, y: 4_000, width: 2_000, height: 1_000 });
  });
});

describe('resizing', () => {
  const handles: Handle[] = ['n', 's', 'e', 'w', 'ne', 'nw', 'se', 'sw'];

  /** The invariants that must hold for every drag, however wild. */
  it('never leaves the source or collapses, for any handle, drag and aspect', () => {
    const next = random(20260930);
    for (let trial = 0; trial < 4_000; trial += 1) {
      const width = 1 + Math.floor(next() * 6_000);
      const height = 1 + Math.floor(next() * 4_500);
      const rect: Rect = {
        x: Math.floor(next() * (SOURCE.width - width)),
        y: Math.floor(next() * (SOURCE.height - height)),
        width,
        height
      };
      const handle = handles[Math.floor(next() * handles.length)];
      const dx = Math.round((next() - 0.5) * 30_000);
      const dy = Math.round((next() - 0.5) * 30_000);
      const ratio = next() < 0.5 ? null : [1, 1.5, 4 / 3, 1.25, 16 / 9][Math.floor(next() * 5)];
      const result = resizeRect(rect, handle, dx, dy, SOURCE.width, SOURCE.height, ratio);
      expect(
        inside(result),
        `seed trial ${trial}: ${handle} (${dx},${dy}) ratio ${ratio} on ${JSON.stringify(rect)} gave ${JSON.stringify(result)}`
      ).toBe(true);
    }
  });

  it('keeps the aspect ratio when locked, to within a pixel of rounding', () => {
    const next = random(77);
    for (let trial = 0; trial < 2_000; trial += 1) {
      const ratio = [1, 1.5, 4 / 3, 1.25, 16 / 9][Math.floor(next() * 5)];
      const height = 50 + Math.floor(next() * 3_000);
      const width = Math.round(height * ratio);
      const rect: Rect = {
        x: Math.floor(next() * (SOURCE.width - width)),
        y: Math.floor(next() * (SOURCE.height - height)),
        width,
        height
      };
      const handle = (['ne', 'nw', 'se', 'sw', 'e', 'w', 'n', 's'] as Handle[])[Math.floor(next() * 8)];
      const result = resizeRect(
        rect,
        handle,
        Math.round((next() - 0.5) * 4_000),
        Math.round((next() - 0.5) * 4_000),
        SOURCE.width,
        SOURCE.height,
        ratio
      );
      const drift = Math.min(
        Math.abs(result.height - result.width / ratio),
        Math.abs(result.width - result.height * ratio)
      );
      expect(drift, `${handle} on ${JSON.stringify(rect)} -> ${JSON.stringify(result)}`).toBeLessThanOrEqual(1);
    }
  });

  it('holds the opposite corner fixed when a corner is dragged without a lock', () => {
    const rect = { x: 1_000, y: 1_000, width: 2_000, height: 1_500 };
    const result = resizeRect(rect, 'se', 300, 200, SOURCE.width, SOURCE.height);
    expect(result).toEqual({ x: 1_000, y: 1_000, width: 2_300, height: 1_700 });
    const grown = resizeRect(rect, 'nw', -300, -200, SOURCE.width, SOURCE.height);
    expect(grown).toEqual({ x: 700, y: 800, width: 2_300, height: 1_700 });
  });

  it('cannot be dragged past the opposite edge', () => {
    const rect = { x: 1_000, y: 1_000, width: 100, height: 100 };
    const result = resizeRect(rect, 'e', -5_000, 0, SOURCE.width, SOURCE.height);
    expect(result.width).toBe(1);
    expect(result.x).toBe(1_000);
  });

  it('grows an edge only as far as the source allows', () => {
    const rect = { x: 11_000, y: 0, width: 500, height: 500 };
    expect(resizeRect(rect, 'e', 50_000, 0, SOURCE.width, SOURCE.height).width).toBe(1_000);
  });
});

describe('aspect', () => {
  it('maps presets to ratios', () => {
    expect(aspectRatio('free', 100, 50)).toBeNull();
    expect(aspectRatio('original', 12_000, 9_000)).toBeCloseTo(4 / 3);
    expect(aspectRatio('1:1', 1, 1)).toBe(1);
    expect(aspectRatio('3:2', 1, 1)).toBe(1.5);
    expect(aspectRatio('16:9', 1, 1)).toBeCloseTo(16 / 9);
    expect(aspectRatio('5:4', 1, 1)).toBe(1.25);
  });

  it('reshapes a rectangle to a ratio about its centre, inside the source', () => {
    const rect = { x: 1_000, y: 1_000, width: 2_000, height: 1_000 };
    const square = applyAspect(rect, 1, SOURCE.width, SOURCE.height);
    expect(square.width).toBe(square.height);
    expect(Math.abs(square.x + square.width / 2 - 2_000)).toBeLessThanOrEqual(1);
    expect(inside(square)).toBe(true);
    // Free leaves it alone.
    expect(applyAspect(rect, null, SOURCE.width, SOURCE.height)).toBe(rect);
  });

  it('keeps a reshaped rectangle inside the source even at the edge', () => {
    const rect = { x: 0, y: 0, width: 12_000, height: 9_000 };
    for (const ratio of [1, 1.5, 4 / 3, 16 / 9, 0.5, 3]) {
      expect(inside(applyAspect(rect, ratio, SOURCE.width, SOURCE.height))).toBe(true);
    }
  });
});

describe('pricing', () => {
  it('evaluates the backend coefficients as the larger of opening and editing', () => {
    const cost = { openingFixed: 1_000, openingPerPixel: 20, editingFixed: 500, editingPerPixel: 44 };
    expect(regionPeakBytes(cost, 10)).toBe(Math.max(1_200, 940));
    // Past the crossover the editing term governs.
    expect(regionPeakBytes(cost, 1_000_000)).toBe(500 + 44_000_000);
  });

  it('calls a region within the offered maximum supported, and one beyond it not', () => {
    const r = report();
    const ok = regionStatus({ x: 0, y: 0, width: 4_000, height: 4_000 }, r);
    expect(ok.supported).toBe(true);
    expect(ok.reason).toBe('');
    expect(ok.pixels).toBe(16_000_000);
    const tooBig = regionStatus({ x: 0, y: 0, width: 5_000, height: 5_000 }, r);
    expect(tooBig.supported).toBe(false);
    expect(tooBig.reason).toMatch(/Too large/);
    // A rectangle exactly at the limit is still supported: the figure is a boundary.
    const edge = regionStatus({ x: 0, y: 0, width: 4_000, height: 5_000 }, r);
    expect(edge.supported).toBe(true);
  });

  it('offers no region at all when the decoder cannot produce one', () => {
    const r = report({ options: [], regionCost: null });
    const status = regionStatus({ x: 0, y: 0, width: 10, height: 10 }, r);
    expect(status.supported).toBe(false);
    expect(status.peakBytes).toBeNull();
  });

  it('computes megapixels', () => {
    expect(megapixels({ x: 0, y: 0, width: 4_000, height: 3_000 })).toBe(12);
  });
});

describe('the largest region', () => {
  it('fits the budget in the requested aspect and stays inside the source', () => {
    const r = report();
    for (const ratio of [null, 1, 1.5, 4 / 3, 16 / 9, 1.25]) {
      const rect = largestRegion(r, SOURCE.width, SOURCE.height, ratio, null);
      expect(rect).not.toBeNull();
      expect(inside(rect as Rect)).toBe(true);
      expect((rect as Rect).width * (rect as Rect).height).toBeLessThanOrEqual(20_000_000);
      // And it is close to the cap: not an arbitrarily small rectangle.
      expect((rect as Rect).width * (rect as Rect).height).toBeGreaterThan(19_000_000);
    }
  });

  it('is centred on the current rectangle where the source allows', () => {
    const r = report();
    const rect = largestRegion(r, SOURCE.width, SOURCE.height, 1, { x: 8_000, y: 6_000, width: 100, height: 100 }) as Rect;
    expect(inside(rect)).toBe(true);
    // Pushed against the edge rather than falling outside it.
    expect(rect.x + rect.width).toBeLessThanOrEqual(SOURCE.width);
    expect(rect.x).toBeGreaterThan(5_000);
  });

  it('takes the whole source when the source is small enough', () => {
    const r = report({
      options: [{ kind: 'openRegion', maxRegionPixels: 500_000_000, decode: { kind: 'rows' }, peakBytesAtMax: 1 }]
    });
    const rect = largestRegion(r, 1_000, 800, null, null) as Rect;
    expect(rect).toEqual({ x: 0, y: 0, width: 1_000, height: 800 });
  });

  it('returns nothing when no region is on offer', () => {
    expect(largestRegion(report({ options: [] }), SOURCE.width, SOURCE.height, null, null)).toBeNull();
  });

  it('starts from a usable rectangle even with no region option', () => {
    const rect = initialRegion(report({ options: [] }), SOURCE.width, SOURCE.height);
    expect(inside(rect)).toBe(true);
  });
});

describe('reduced size', () => {
  it('rounds each side and never reaches zero', () => {
    expect(reducedSize(12_000, 9_000, 0.5)).toEqual({ width: 6_000, height: 4_500 });
    expect(reducedSize(10, 10, 0.001)).toEqual({ width: 1, height: 1 });
    expect(reducedSize(10, 10, 5)).toEqual({ width: 10, height: 10 });
  });
});
