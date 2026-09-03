import { describe, expect, it } from 'vitest';
import {
  addCurvePoint,
  curveChannels,
  definitionFor,
  identityCurve,
  identityCurves,
  isIdentityCurve,
  isValidCurve,
  MAX_CURVE_POINTS,
  MIN_CURVE_GAP,
  moveCurvePoint,
  readField,
  removeCurvePoint,
  sampleCurve,
  writeField
} from './adjustments';
import type { CurvePoint } from '../types/editor';

function points(...pairs: [number, number][]): CurvePoint[] {
  return pairs.map(([input, output]) => ({ input, output }));
}

describe('curve construction', () => {
  it('starts as a valid identity on every channel', () => {
    const curves = identityCurves();
    for (const channel of curveChannels) {
      expect(isIdentityCurve(curves[channel])).toBe(true);
      expect(isValidCurve(curves[channel])).toBe(true);
    }
  });

  it('recognizes a curve that is no longer the identity', () => {
    expect(isIdentityCurve(points([0, 0], [0.5, 0.7], [1, 1]))).toBe(false);
    expect(isIdentityCurve(points([0, 0.1], [1, 1]))).toBe(false);
  });
});

describe('moving points', () => {
  it('anchors the first and last inputs while letting their outputs move', () => {
    const moved = moveCurvePoint(identityCurve(), 0, 0.6, 0.3);
    expect(moved[0].input).toBe(0);
    expect(moved[0].output).toBeCloseTo(0.3);

    const end = moveCurvePoint(identityCurve(), 1, 0.2, 0.4);
    expect(end[1].input).toBe(1);
    expect(end[1].output).toBeCloseTo(0.4);
  });

  it('keeps an interior point strictly between its neighbours', () => {
    const start = points([0, 0], [0.5, 0.5], [1, 1]);
    const pushedLeft = moveCurvePoint(start, 1, -5, 0.5);
    expect(pushedLeft[1].input).toBeGreaterThan(pushedLeft[0].input);
    expect(pushedLeft[1].input).toBeCloseTo(MIN_CURVE_GAP);

    const pushedRight = moveCurvePoint(start, 1, 5, 0.5);
    expect(pushedRight[1].input).toBeLessThan(pushedRight[2].input);
    expect(pushedRight[1].input).toBeCloseTo(1 - MIN_CURVE_GAP);
  });

  it('clamps outputs into range and rejects non-finite input', () => {
    const clamped = moveCurvePoint(identityCurve(), 0, 0, 9);
    expect(clamped[0].output).toBe(1);
    const nan = moveCurvePoint(identityCurve(), 0, 0, Number.NaN);
    expect(nan[0].output).toBe(0);
  });

  it('ignores an out-of-range index', () => {
    const start = identityCurve();
    expect(moveCurvePoint(start, -1, 0.5, 0.5)).toBe(start);
    expect(moveCurvePoint(start, 9, 0.5, 0.5)).toBe(start);
  });

  it('leaves the original array untouched', () => {
    const start = points([0, 0], [0.5, 0.5], [1, 1]);
    const snapshot = JSON.stringify(start);
    moveCurvePoint(start, 1, 0.8, 0.2);
    expect(JSON.stringify(start)).toBe(snapshot);
  });

  it('keeps the result valid for every move it allows', () => {
    let current = points([0, 0], [0.3, 0.3], [0.6, 0.6], [1, 1]);
    for (const target of [-1, 0, 0.1, 0.5, 0.95, 1, 2]) {
      current = moveCurvePoint(current, 1, target, 0.4);
      expect(isValidCurve(current)).toBe(true);
    }
  });
});

describe('adding points', () => {
  it('inserts in sorted order', () => {
    const next = addCurvePoint(identityCurve(), 0.4, 0.7);
    expect(next.map((point) => point.input)).toEqual([0, 0.4, 1]);
    expect(isValidCurve(next)).toBe(true);
  });

  it('refuses a position that collides with an existing point', () => {
    const start = points([0, 0], [0.5, 0.5], [1, 1]);
    expect(addCurvePoint(start, 0.5 + MIN_CURVE_GAP / 2, 0.9)).toBe(start);
  });

  it('refuses the anchored endpoints', () => {
    const start = identityCurve();
    expect(addCurvePoint(start, 0, 0.5)).toBe(start);
    expect(addCurvePoint(start, 1, 0.5)).toBe(start);
  });

  it('stops at the maximum point count', () => {
    let current = identityCurve();
    for (let index = 1; index < MAX_CURVE_POINTS + 5; index += 1) {
      current = addCurvePoint(current, index / (MAX_CURVE_POINTS + 6), 0.5);
    }
    expect(current.length).toBe(MAX_CURVE_POINTS);
    expect(isValidCurve(current)).toBe(true);
  });
});

describe('removing points', () => {
  it('removes an interior point', () => {
    const next = removeCurvePoint(points([0, 0], [0.5, 0.5], [1, 1]), 1);
    expect(next).toHaveLength(2);
    expect(isValidCurve(next)).toBe(true);
  });

  it('never removes an anchor', () => {
    const start = points([0, 0], [0.5, 0.5], [1, 1]);
    expect(removeCurvePoint(start, 0)).toBe(start);
    expect(removeCurvePoint(start, 2)).toBe(start);
  });

  it('never drops below the minimum point count', () => {
    const start = identityCurve();
    expect(removeCurvePoint(start, 0)).toBe(start);
    expect(removeCurvePoint(start, 1)).toBe(start);
  });
});

describe('sampling', () => {
  it('is the identity for an identity curve', () => {
    for (const input of [0, 0.25, 0.5, 0.75, 1]) {
      expect(sampleCurve(identityCurve(), input)).toBeCloseTo(input);
    }
  });

  it('interpolates linearly between points', () => {
    const curve = points([0, 0], [0.5, 0.8], [1, 1]);
    expect(sampleCurve(curve, 0.25)).toBeCloseTo(0.4);
    expect(sampleCurve(curve, 0.5)).toBeCloseTo(0.8);
    expect(sampleCurve(curve, 0.75)).toBeCloseTo(0.9);
  });

  it('clamps outside the domain and survives odd input', () => {
    const curve = points([0, 0.2], [1, 0.9]);
    expect(sampleCurve(curve, -1)).toBeCloseTo(0.2);
    expect(sampleCurve(curve, 2)).toBeCloseTo(0.9);
    expect(sampleCurve(curve, Number.NaN)).toBeCloseTo(0.2);
  });
});

describe('curve validity mirrors the backend rules', () => {
  it('accepts a well formed curve', () => {
    expect(isValidCurve(points([0, 0], [0.5, 0.6], [1, 1]))).toBe(true);
  });

  it('rejects unsorted, unanchored, short, and out-of-range curves', () => {
    expect(isValidCurve(points([0, 0], [0.7, 0.7], [0.6, 1], [1, 1]))).toBe(false);
    expect(isValidCurve(points([0.1, 0], [1, 1]))).toBe(false);
    expect(isValidCurve(points([0, 0], [0.9, 1]))).toBe(false);
    expect(isValidCurve(points([0, 0]))).toBe(false);
    expect(isValidCurve(points([0, 0], [0.5, 2], [1, 1]))).toBe(false);
    expect(isValidCurve(points([0, 0], [0.5, Number.NaN], [1, 1]))).toBe(false);
  });
});

describe('selective colour fields', () => {
  const definition = definitionFor('selective_color');

  it('is offered as an adjustment layer with all six controls', () => {
    expect(definition).toBeTruthy();
    expect(definition?.fields.map((field) => field.key)).toEqual([
      'target_hue',
      'width',
      'adjustment.cyan',
      'adjustment.magenta',
      'adjustment.yellow',
      'adjustment.black'
    ]);
  });

  it('reads and writes nested CMYK amounts without disturbing siblings', () => {
    const cyan = definition!.fields.find((field) => field.key === 'adjustment.cyan')!;
    const magenta = definition!.fields.find((field) => field.key === 'adjustment.magenta')!;
    let operation = definition!.build();

    operation = writeField(operation, cyan, 0.6);
    expect(readField(operation, cyan)).toBeCloseTo(0.6);
    expect(readField(operation, magenta)).toBe(0);

    operation = writeField(operation, magenta, -0.4);
    expect(readField(operation, cyan)).toBeCloseTo(0.6);
    expect(readField(operation, magenta)).toBeCloseTo(-0.4);
    if (operation.type === 'selective_color') {
      expect(operation.adjustment.yellow).toBe(0);
      expect(operation.target_hue).toBe(0);
    }
  });

  it('clamps nested amounts into the validated range', () => {
    const cyan = definition!.fields.find((field) => field.key === 'adjustment.cyan')!;
    expect(readField(writeField(definition!.build(), cyan, 5), cyan)).toBe(1);
    expect(readField(writeField(definition!.build(), cyan, -5), cyan)).toBe(-1);
  });

  it('does not mutate the operation it was given', () => {
    const cyan = definition!.fields.find((field) => field.key === 'adjustment.cyan')!;
    const original = definition!.build();
    const snapshot = JSON.stringify(original);
    writeField(original, cyan, 0.9);
    expect(JSON.stringify(original)).toBe(snapshot);
  });
});

describe('curves as an adjustment layer', () => {
  it('is offered and builds a neutral curve set', () => {
    const definition = definitionFor('curves');
    expect(definition?.editor).toBe('curves');
    const operation = definition!.build();
    expect(operation.type).toBe('curves');
    if (operation.type === 'curves') {
      expect(isIdentityCurve(operation.curves.rgb)).toBe(true);
      expect(isIdentityCurve(operation.curves.blue)).toBe(true);
    }
  });
});
