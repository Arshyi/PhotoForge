import { describe, expect, it } from 'vitest';
import type { EditOperation } from '../types/editor';
import {
  canonicalizeGeometryOperations,
  computeStageDimensions,
  extractGeometryOperations,
  geometryFingerprint,
  geometryOperationsToEditOperations
} from './geometry';
import type { GeometryOperation } from './types';

const perspective: GeometryOperation = {
  type: 'perspective',
  corners: {
    topLeft: [0.05, 0.05],
    topRight: [0.95, 0.04],
    bottomRight: [0.9, 0.96],
    bottomLeft: [0.08, 0.92]
  }
};

const lens: GeometryOperation = {
  type: 'lens_correction',
  distortion: 0.15,
  vignetting: -0.2,
  chromatic_aberration: 0.05
};

describe('selection geometry', () => {
  it('extracts only canonical geometry operations, including horizontal reflection', () => {
    const operations: EditOperation[] = [
      { type: 'brightness', amount: 0.1 },
      { type: 'rotate', degrees: -90 },
      { type: 'reflect_horizontal' },
      { type: 'straighten', degrees: -0 },
      lens,
      { type: 'contrast', amount: 0.2 }
    ];

    const extracted = extractGeometryOperations(operations);
    expect(extracted).toEqual([
      { type: 'rotate', degrees: 270 },
      { type: 'reflect_horizontal' },
      { type: 'straighten', degrees: 0 },
      lens
    ]);
    expect(geometryFingerprint(extracted)).toBe(geometryFingerprint([
      { type: 'rotate', degrees: 270 },
      { type: 'reflect_horizontal' },
      { type: 'straighten', degrees: 0 },
      lens
    ]));
  });

  it('matches crop floor, round, clamp, and quarter-turn dimension semantics', () => {
    expect(computeStageDimensions(101, 51, [
      {
        type: 'crop',
        x: 0.1,
        y: 0.2,
        width: 0.5,
        height: 0.5,
        aspect_ratio: null,
        overlay: 'none'
      },
      { type: 'reflect_horizontal' },
      { type: 'rotate', degrees: 90 },
      { type: 'straighten', degrees: 2 },
      perspective,
      lens
    ])).toEqual({ width: 26, height: 51 });

    expect(computeStageDimensions(10, 10, [{
      type: 'crop',
      x: 0.8,
      y: 0,
      width: 0.2,
      height: 1,
      aspect_ratio: null,
      overlay: 'rule_of_thirds'
    }])).toEqual({ width: 2, height: 10 });

    expect(computeStageDimensions(16_777_217, 1, [{
      type: 'crop',
      x: 0,
      y: 0,
      width: 0.5,
      height: 1,
      aspect_ratio: null,
      overlay: 'none'
    }])).toEqual({ width: 8_388_608, height: 1 });
  });

  it('converts canonical geometry back to independent edit operations', () => {
    const geometry: GeometryOperation[] = [
      { type: 'reflect_horizontal' },
      { type: 'rotate', degrees: 90 },
      perspective,
      lens
    ];
    const edits = geometryOperationsToEditOperations(geometry);
    expect(edits).toEqual(geometry);
    expect(extractGeometryOperations(edits)).toEqual(geometry);
    expect(edits).not.toBe(geometry);
  });

  it('uses a deterministic canonical fingerprint', () => {
    expect(geometryFingerprint([{ type: 'rotate', degrees: -90 }])).toBe(
      geometryFingerprint([{ type: 'rotate', degrees: 270 }])
    );
    expect(geometryFingerprint([{ type: 'straighten', degrees: -0 }])).toBe(
      geometryFingerprint([{ type: 'straighten', degrees: 0 }])
    );
    expect(geometryFingerprint([{ type: 'reflect_horizontal' }])).not.toBe(
      geometryFingerprint([])
    );
    expect(geometryFingerprint([lens])).not.toBe(geometryFingerprint([{
      ...lens, vignetting: 0
    }]));
    expect(geometryFingerprint([
      { type: 'rotate', degrees: 90 },
      { type: 'reflect_horizontal' }
    ])).not.toBe(geometryFingerprint([
      { type: 'reflect_horizontal' },
      { type: 'rotate', degrees: 90 }
    ]));
  });

  it('rejects non-finite, out-of-bounds, future, and degenerate geometry', () => {
    expect(canonicalizeGeometryOperations([{ type: 'straighten', degrees: Number.NaN }])).toBeNull();
    expect(canonicalizeGeometryOperations([{
      type: 'crop', x: 0.8, y: 0, width: 0.3, height: 1, aspect_ratio: null, overlay: 'none'
    }])).toBeNull();
    expect(canonicalizeGeometryOperations([{
      ...perspective,
      corners: { ...perspective.corners, topLeft: [0.99, 0.05] }
    }])).toBeNull();
    expect(canonicalizeGeometryOperations([{ type: 'reflect_horizontal', future: true }])).toBeNull();
    expect(canonicalizeGeometryOperations([{
      ...lens, chromatic_aberration: Number.POSITIVE_INFINITY
    }])).toBeNull();
    expect(canonicalizeGeometryOperations([{ ...lens, distortion: -1.01 }])).toBeNull();
    expect(canonicalizeGeometryOperations([{ ...lens, distortion: -0.17 }])).toBeNull();
    expect(canonicalizeGeometryOperations(Array.from(
      { length: 65 },
      () => ({ type: 'reflect_horizontal' as const })
    ))).toBeNull();
  });

  it('keeps lens dimensions stable across the supported distortion range', () => {
    expect(computeStageDimensions(101, 51, [lens])).toEqual({ width: 101, height: 51 });
    expect(computeStageDimensions(101, 51, [{ ...lens, distortion: -0.16 }]))
      .toEqual({ width: 101, height: 51 });
    expect(() => computeStageDimensions(101, 51, [{ ...lens, distortion: -0.17 }]))
      .toThrow(/invalid/i);
  });
});
