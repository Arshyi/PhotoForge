import { describe, expect, it } from 'vitest';
import {
  normalizePointerPressure,
  resolveBrushSample,
  shouldAppendPointerUpSample,
  type PressureSettings
} from './pressure';

const enabled: PressureSettings = {
  enabled: true,
  affectsSize: true,
  affectsOpacity: true,
  minimumSizeFactor: 0.25,
  minimumOpacityFactor: 0.5
};

describe('pointer pressure resolution', () => {
  it('accepts only finite pen pressure and clamps it', () => {
    expect(normalizePointerPressure({ pointerType: 'pen', pressure: -2 }, true)).toBe(0);
    expect(normalizePointerPressure({ pointerType: 'pen', pressure: 3 }, true)).toBe(1);
    expect(normalizePointerPressure({ pointerType: 'mouse', pressure: 0.5 }, true)).toBeNull();
    expect(normalizePointerPressure({ pointerType: 'pen', pressure: Number.NaN }, true)).toBeNull();
    expect(normalizePointerPressure({ pointerType: 'pen', pressure: 0.5 }, false)).toBeNull();
  });

  it('leaves mouse and disabled input at the base values', () => {
    expect(resolveBrushSample({ x: 1, y: 2 }, { pointerType: 'mouse', pressure: 0.5 }, 80, 0.8, enabled)).toEqual({
      x: 1,
      y: 2,
      diameter: 80,
      opacity: 0.8
    });
    expect(resolveBrushSample({ x: 1, y: 2 }, { pointerType: 'pen', pressure: 0.1 }, 80, 0.8, { ...enabled, enabled: false }).diameter).toBe(80);
  });

  it('resolves effective values deterministically and conservatively', () => {
    const sample = resolveBrushSample(
      { x: 10, y: 20 },
      { pointerType: 'pen', pressure: 0.5 },
      100,
      0.8,
      enabled
    );
    expect(sample).toEqual({ x: 10, y: 20, diameter: 62.5, opacity: 0.6 });
    expect(resolveBrushSample({ x: 0, y: 0 }, { pointerType: 'pen', pressure: 0.5 }, Number.NaN, Number.POSITIVE_INFINITY, enabled)).toMatchObject({ diameter: 30, opacity: 0.75 });
  });

  it('does not append the zero-pressure pen-up artifact', () => {
    expect(shouldAppendPointerUpSample({ pointerType: 'pen', pressure: 0 }, enabled, 2)).toBe(false);
    expect(shouldAppendPointerUpSample({ pointerType: 'pen', pressure: 0.2 }, enabled, 2)).toBe(true);
    expect(shouldAppendPointerUpSample({ pointerType: 'mouse', pressure: 0 }, enabled, 2)).toBe(true);
  });
});
