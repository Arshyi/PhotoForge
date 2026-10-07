import { describe, expect, it } from 'vitest';
import {
  GIB,
  MANUAL_STEP_BYTES,
  MIB,
  describeBasis,
  describeLimits,
  gibibytes,
  manualRange,
  manualWarning,
  megapixels,
  snapBudget,
  waitsForNextDocument
} from './budget';
import { statusFixture } from './testing';

describe('the manual range', () => {
  it('is exactly what the policy will accept, in steps the control can move by', () => {
    const range = manualRange(statusFixture());
    expect(range.step).toBe(MANUAL_STEP_BYTES);
    expect(range.min).toBe(512 * MIB);
    expect(range.max % range.step).toBe(0);
    expect(range.max).toBeLessThanOrEqual(14.4 * GIB);
    expect(range.max).toBeGreaterThan(14 * GIB);
  });

  it('never has a maximum below its minimum, however small the machine', () => {
    const tiny = statusFixture({}, { ceilingBytes: 100 * MIB });
    const range = manualRange(tiny);
    expect(range.max).toBeGreaterThanOrEqual(range.min);
    const odd = manualRange(statusFixture({ minBudgetBytes: 600 * MIB }));
    expect(odd.min % odd.step).toBe(0);
    expect(odd.min).toBeGreaterThanOrEqual(600 * MIB);
  });

  it('snaps a figure into the range, and has an answer for nonsense', () => {
    const range = { min: 512 * MIB, max: 8 * GIB, step: 256 * MIB };
    expect(snapBudget(1 * GIB + 100 * MIB, range)).toBe(1 * GIB);
    expect(snapBudget(1, range)).toBe(range.min);
    expect(snapBudget(99 * GIB, range)).toBe(range.max);
    expect(snapBudget(Number.NaN, range)).toBe(range.min);
    expect(snapBudget(Number.POSITIVE_INFINITY, range)).toBe(range.min);
    expect(snapBudget(3 * GIB, range)).toBe(3 * GIB);
  });
});

describe('numbers in words', () => {
  it('rounds gigabytes and megapixels sensibly', () => {
    expect(gibibytes(1.5 * GIB)).toBe(1.5);
    expect(gibibytes(GIB / 3)).toBe(0.33);
    expect(megapixels(12_000_000)).toBe('12.0 MP');
    expect(megapixels(45_000_000)).toBe('45.0 MP');
    expect(megapixels(100_663_296)).toBe('101 MP');
  });

  it('says where each budget came from, and never presents a guess as a measurement', () => {
    expect(describeBasis(statusFixture().budget)).toMatch(/Worked out from this computer.*2\.40 GB kept back/);
    const unmeasured = statusFixture({}, { basis: 'unmeasured', reserveBytes: 0 }).budget;
    expect(describeBasis(unmeasured)).toMatch(/could not measure.*conservative/);
    expect(describeBasis(statusFixture({}, { basis: 'manual' }).budget)).toBe('The figure you chose.');
    expect(describeBasis(statusFixture({}, { basis: 'manual', clamped: true }).budget)).toMatch(/moved into the range.*up to 14\.4 GB/);
  });

  it('says what the budget lets a person open, and that a bigger image is offered an alternative', () => {
    const sentence = describeLimits(statusFixture());
    expect(sentence).toContain('101 MP');
    expect(sentence).toMatch(/region or a smaller copy instead of being refused/);
  });
});

describe('manual figures', () => {
  it('say when a lower figure waits for the next document', () => {
    const status = statusFixture();
    expect(waitsForNextDocument(status, 4 * GIB)).toBe(true);
    expect(waitsForNextDocument(status, 6 * GIB)).toBe(false);
    expect(waitsForNextDocument(status, 8 * GIB)).toBe(false);
  });

  it('warn when little is left for everything else, or more than is free is asked for', () => {
    const status = statusFixture();
    expect(manualWarning(status, 4 * GIB)).toBeNull();
    expect(manualWarning(status, 14 * GIB)).toMatch(/leaves only 2\.00 GB of 16\.0 GB/);
    expect(manualWarning(status, 10 * GIB)).toMatch(/Only 9\.00 GB is free right now/);
    expect(manualWarning(statusFixture({ system: null }), 14 * GIB)).toBeNull();
    // Asking for more than is installed does not produce a negative figure.
    expect(manualWarning(status, 40 * GIB)).toMatch(/leaves only 0 B/);
  });
});
