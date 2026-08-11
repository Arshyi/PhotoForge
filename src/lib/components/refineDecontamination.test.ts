import { describe, expect, it } from 'vitest';
import { decontaminatePreviewPixels } from './refineDecontamination';

function rgba(pixels: number[][]): Uint8ClampedArray {
  return new Uint8ClampedArray(pixels.flat());
}

function mask(coverage: number[]): Uint8ClampedArray {
  return rgba(coverage.map((value) => [value, value, value, 255]));
}

describe('decontaminatePreviewPixels', () => {
  it('is off by default behavior and never mutates either input', () => {
    const source = rgba([[220, 20, 20, 17], [20, 220, 20, 29]]);
    const selection = mask([255, 128]);
    const originalSource = new Uint8ClampedArray(source);
    const originalMask = new Uint8ClampedArray(selection);

    const output = decontaminatePreviewPixels(source, selection, 2, 1, {
      enabled: false,
      strength: 0.5,
      radius: 4
    });

    expect(output).toEqual(source);
    expect(output).not.toBe(source);
    expect(source).toEqual(originalSource);
    expect(selection).toEqual(originalMask);
  });

  it('replaces partial-edge spill from nearby confident foreground and preserves alpha', () => {
    const source = rgba([
      [220, 20, 20, 31],
      [20, 220, 20, 47],
      [20, 20, 220, 63]
    ]);
    const output = decontaminatePreviewPixels(source, mask([255, 128, 0]), 3, 1, {
      enabled: true,
      strength: 1,
      radius: 1
    });

    expect([...output]).toEqual([
      220, 20, 20, 31,
      220, 20, 20, 47,
      20, 20, 220, 63
    ]);
  });

  it('averages only confident samples, is deterministic, and honors strength', () => {
    const source = rgba([
      [200, 0, 0, 255],
      [0, 200, 0, 91],
      [0, 0, 200, 255]
    ]);
    const selection = mask([255, 100, 240]);
    const options = { enabled: true, strength: 0.5, radius: 2 };
    const first = decontaminatePreviewPixels(source, selection, 3, 1, options);
    const second = decontaminatePreviewPixels(source, selection, 3, 1, options);

    expect(first).toEqual(second);
    expect([...first.slice(4, 8)]).toEqual([52, 100, 49, 91]);
  });

  it('uses the same circular neighborhood as export and excludes radius-one diagonals', () => {
    const source = rgba([
      [180, 20, 20, 255], [0, 0, 0, 255],
      [0, 0, 0, 255], [20, 180, 20, 255]
    ]);
    const selection = mask([255, 0, 0, 128]);
    const output = decontaminatePreviewPixels(source, selection, 2, 2, {
      enabled: true,
      strength: 1,
      radius: 1
    });
    expect([...output.slice(12, 16)]).toEqual([20, 180, 20, 255]);
  });

  it('fails closed before allocating work buffers for invalid or oversized dimensions', () => {
    const source = rgba([[10, 20, 30, 40]]);
    const selection = mask([128]);
    expect(() =>
      decontaminatePreviewPixels(source, selection, 0, 1, {
        enabled: true,
        strength: 1,
        radius: 32
      })
    ).toThrow('dimensions are invalid');
    expect(() =>
      decontaminatePreviewPixels(source, selection, 65_537, 1, {
        enabled: true,
        strength: 1,
        radius: 32
      })
    ).toThrow('dimensions are invalid');
  });

  it('fails closed when the bounded preview would exceed 128 million neighborhood visits', () => {
    const width = 200;
    const height = 200;
    const source = new Uint8ClampedArray(width * height * 4);
    const selection = new Uint8ClampedArray(width * height * 4);
    for (let index = 0; index < selection.length; index += 4) {
      selection.set([128, 128, 128, 255], index);
    }

    expect(() =>
      decontaminatePreviewPixels(source, selection, width, height, {
        enabled: true,
        strength: 1,
        radius: 32
      })
    ).toThrow('exceeds its bounded work limit');
  });
});
