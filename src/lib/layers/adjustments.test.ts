import { describe, expect, it } from 'vitest';
import {
  adjustmentDefinitions,
  adjustmentLabel,
  definitionFor,
  formatField,
  hslBands,
  isEditableAdjustment,
  neutralHslSettings,
  readField,
  writeField
} from './adjustments';
import type { BaseEditOperation } from '../types/editor';

describe('adjustment definitions', () => {
  it('covers every adjustment the phase promises', () => {
    const types = adjustmentDefinitions.map((definition) => definition.type);
    for (const expected of [
      'brightness',
      'contrast',
      'levels',
      'hsl',
      'temperature_tint',
      'raw_development',
      'local_contrast',
      'sharpen',
      'denoise'
    ]) {
      expect(types).toContain(expected);
    }
  });

  it('lists each operation only once', () => {
    const types = adjustmentDefinitions.map((definition) => definition.type);
    expect(new Set(types).size).toBe(types.length);
  });

  it('builds an operation whose type matches its definition', () => {
    for (const definition of adjustmentDefinitions) {
      expect(definition.build().type).toBe(definition.type);
    }
  });

  it('builds neutral defaults that match each field default', () => {
    for (const definition of adjustmentDefinitions) {
      const operation = definition.build();
      for (const field of definition.fields) {
        expect(readField(operation, field)).toBe(field.defaultValue);
      }
    }
  });

  it('keeps every field default inside its own range', () => {
    for (const definition of adjustmentDefinitions) {
      for (const field of definition.fields) {
        expect(field.min).toBeLessThan(field.max);
        expect(field.defaultValue).toBeGreaterThanOrEqual(field.min);
        expect(field.defaultValue).toBeLessThanOrEqual(field.max);
        expect(field.step).toBeGreaterThan(0);
      }
    }
  });

  it('never offers a geometry operation as an adjustment layer', () => {
    const types = adjustmentDefinitions.map((definition) => definition.type);
    for (const forbidden of [
      'crop',
      'rotate',
      'straighten',
      'perspective',
      'lens_correction',
      'reflect_horizontal',
      'decontaminate_colors'
    ]) {
      expect(types).not.toContain(forbidden);
    }
  });

  it('resolves a definition by type and reports unknown ones', () => {
    expect(definitionFor('brightness')?.label).toBe('Exposure / Brightness');
    expect(definitionFor('nonsense')).toBeNull();
  });

  it('labels an operation by its definition and falls back to its type', () => {
    expect(adjustmentLabel({ type: 'contrast', amount: 0 })).toBe('Contrast');
    expect(adjustmentLabel({ type: 'crop' } as unknown as BaseEditOperation)).toBe('crop');
  });

  it('reports which operations the dialog can edit', () => {
    expect(isEditableAdjustment({ type: 'gamma', value: 1 })).toBe(true);
    expect(isEditableAdjustment({ type: 'crop' } as unknown as BaseEditOperation)).toBe(false);
  });
});

describe('scalar fields', () => {
  const brightness = definitionFor('brightness');
  const field = brightness?.fields[0];

  it('reads a stored value and falls back to the default', () => {
    expect(readField({ type: 'brightness', amount: 0.3 }, field!)).toBeCloseTo(0.3);
    expect(readField({ type: 'brightness' } as unknown as BaseEditOperation, field!)).toBe(0);
    expect(
      readField({ type: 'brightness', amount: Number.NaN } as BaseEditOperation, field!)
    ).toBe(0);
  });

  it('writes a value without mutating the original operation', () => {
    const original = { type: 'brightness', amount: 0 } as BaseEditOperation;
    const updated = writeField(original, field!, 0.4);
    expect(readField(updated, field!)).toBeCloseTo(0.4);
    expect(readField(original, field!)).toBe(0);
  });

  it('clamps a written value into the validated range', () => {
    expect(readField(writeField({ type: 'brightness', amount: 0 }, field!, 9), field!)).toBe(1);
    expect(readField(writeField({ type: 'brightness', amount: 0 }, field!, -9), field!)).toBe(-1);
  });

  it('rounds integer fields so the backend never sees a fraction', () => {
    const tile = definitionFor('local_contrast')?.fields.find((entry) => entry.key === 'tile_size');
    const operation = definitionFor('local_contrast')!.build();
    expect(readField(writeField(operation, tile!, 33.7), tile!)).toBe(34);
  });

  it('formats values for display by kind', () => {
    expect(formatField({ ...field!, format: 'percent' }, 0.25)).toBe('25%');
    expect(formatField({ ...field!, format: 'decimal' }, 1.234)).toBe('1.23');
    expect(formatField({ ...field!, format: 'integer' }, 33.7)).toBe('34');
  });
});

describe('HSL defaults', () => {
  it('starts every band neutral', () => {
    const settings = neutralHslSettings();
    for (const band of hslBands) {
      expect(settings[band]).toEqual({ hue: 0, saturation: 0, lightness: 0 });
    }
  });

  it('lists the seven bands the backend validates', () => {
    expect(hslBands).toEqual([
      'master',
      'red',
      'yellow',
      'green',
      'cyan',
      'blue',
      'magenta'
    ]);
  });

  it('builds an hsl operation with neutral settings', () => {
    const operation = definitionFor('hsl')!.build();
    expect(operation.type).toBe('hsl');
    if (operation.type === 'hsl') {
      expect(operation.settings.master.hue).toBe(0);
    }
  });
});

describe('levels defaults', () => {
  it('starts as an identity mapping', () => {
    const operation = definitionFor('levels')!.build();
    expect(operation).toEqual({
      type: 'levels',
      input_black: 0,
      input_white: 255,
      gamma: 1,
      output_black: 0,
      output_white: 255
    });
  });
});
