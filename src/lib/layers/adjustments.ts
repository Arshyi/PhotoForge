import type { BaseEditOperation, HslAdjustment, HslSettings, OperationType } from '../types/editor';

/**
 * One editable scalar field of an adjustment operation.
 *
 * Ranges mirror `EditOperation::validate` in Rust exactly. Keeping them in one
 * table means the dialog can never offer a value the backend will reject.
 */
export interface ScalarField {
  key: string;
  label: string;
  min: number;
  max: number;
  step: number;
  defaultValue: number;
  format?: 'percent' | 'decimal' | 'integer';
}

export interface AdjustmentDefinition {
  type: OperationType;
  label: string;
  description: string;
  /** Scalar sliders, empty for operations with no parameters. */
  fields: ScalarField[];
  /** Non-scalar shape this operation needs a dedicated editor for. */
  editor?: 'levels' | 'hsl' | 'unsupported';
  build: () => BaseEditOperation;
}

const percent = { format: 'percent' as const, step: 0.01 };

function neutralHsl(): HslAdjustment {
  return { hue: 0, saturation: 0, lightness: 0 };
}

export function neutralHslSettings(): HslSettings {
  return {
    master: neutralHsl(),
    red: neutralHsl(),
    yellow: neutralHsl(),
    green: neutralHsl(),
    cyan: neutralHsl(),
    blue: neutralHsl(),
    magenta: neutralHsl()
  };
}

export const hslBands: (keyof HslSettings)[] = [
  'master',
  'red',
  'yellow',
  'green',
  'cyan',
  'blue',
  'magenta'
];

/**
 * Every operation PhotoForge can hold in an adjustment layer.
 *
 * The compositor accepts any operation that preserves the canvas dimensions, so
 * this list is the user-facing catalogue rather than an engine limit. Curves and
 * selective colour are engine-supported and render correctly but have no
 * parameter editor in this release; see `docs/layers.md`.
 */
export const adjustmentDefinitions: AdjustmentDefinition[] = [
  {
    type: 'brightness',
    label: 'Exposure / Brightness',
    description: 'Shifts every channel up or down.',
    fields: [{ key: 'amount', label: 'Brightness', min: -1, max: 1, defaultValue: 0, ...percent }],
    build: () => ({ type: 'brightness', amount: 0 })
  },
  {
    type: 'contrast',
    label: 'Contrast',
    description: 'Expands or compresses tones around mid grey.',
    fields: [{ key: 'amount', label: 'Contrast', min: -1, max: 1, defaultValue: 0, ...percent }],
    build: () => ({ type: 'contrast', amount: 0 })
  },
  {
    type: 'saturation',
    label: 'Saturation',
    description: 'Moves colours toward or away from grey.',
    fields: [{ key: 'amount', label: 'Saturation', min: -1, max: 1, defaultValue: 0, ...percent }],
    build: () => ({ type: 'saturation', amount: 0 })
  },
  {
    type: 'gamma',
    label: 'Gamma',
    description: 'Reshapes midtones without moving black or white.',
    fields: [
      { key: 'value', label: 'Gamma', min: 0.2, max: 3, step: 0.01, defaultValue: 1, format: 'decimal' }
    ],
    build: () => ({ type: 'gamma', value: 1 })
  },
  {
    type: 'levels',
    label: 'Levels',
    description: 'Remaps the input and output black and white points.',
    fields: [],
    editor: 'levels',
    build: () => ({
      type: 'levels',
      input_black: 0,
      input_white: 255,
      gamma: 1,
      output_black: 0,
      output_white: 255
    })
  },
  {
    type: 'hsl',
    label: 'HSL',
    description: 'Adjusts hue, saturation, and lightness per colour band.',
    fields: [],
    editor: 'hsl',
    build: () => ({ type: 'hsl', settings: neutralHslSettings() })
  },
  {
    type: 'temperature_tint',
    label: 'Temperature / Tint',
    description: 'Warms or cools the image and shifts green against magenta.',
    fields: [
      { key: 'temperature', label: 'Temperature', min: -1, max: 1, defaultValue: 0, ...percent },
      { key: 'tint', label: 'Tint', min: -1, max: 1, defaultValue: 0, ...percent }
    ],
    build: () => ({ type: 'temperature_tint', temperature: 0, tint: 0 })
  },
  {
    type: 'auto_white_balance',
    label: 'Auto White Balance',
    description: 'Neutralises a colour cast by a chosen strength.',
    fields: [{ key: 'strength', label: 'Strength', min: 0, max: 1, defaultValue: 0.5, ...percent }],
    build: () => ({ type: 'auto_white_balance', strength: 0.5 })
  },
  {
    type: 'local_contrast',
    label: 'Local Contrast',
    description: 'Adds regional contrast with a bounded tile size.',
    fields: [
      { key: 'strength', label: 'Strength', min: 0, max: 1, defaultValue: 0.35, ...percent },
      { key: 'tile_size', label: 'Tile size', min: 8, max: 128, step: 1, defaultValue: 32, format: 'integer' },
      { key: 'clip_limit', label: 'Clip limit', min: 0.5, max: 4, step: 0.05, defaultValue: 1.4, format: 'decimal' }
    ],
    build: () => ({ type: 'local_contrast', strength: 0.35, tile_size: 32, clip_limit: 1.4 })
  },
  {
    type: 'sharpen',
    label: 'Sharpen',
    description: 'Raises edge contrast. It does not recover missing detail.',
    fields: [{ key: 'strength', label: 'Strength', min: 0, max: 2, defaultValue: 0.4, ...percent }],
    build: () => ({ type: 'sharpen', strength: 0.4 })
  },
  {
    type: 'edge_aware_sharpen',
    label: 'Edge-Aware Sharpen',
    description: 'Sharpens edges while leaving flat areas alone.',
    fields: [
      { key: 'strength', label: 'Strength', min: 0, max: 2, defaultValue: 0.45, ...percent },
      { key: 'radius', label: 'Radius', min: 0.5, max: 4, step: 0.1, defaultValue: 1.2, format: 'decimal' },
      { key: 'threshold', label: 'Threshold', min: 0, max: 0.25, step: 0.005, defaultValue: 0.035, format: 'decimal' }
    ],
    build: () => ({ type: 'edge_aware_sharpen', strength: 0.45, radius: 1.2, threshold: 0.035 })
  },
  {
    type: 'denoise',
    label: 'Denoise',
    description: 'Smooths noise while preserving edges.',
    fields: [
      { key: 'strength', label: 'Strength', min: 0, max: 1, defaultValue: 0.3, ...percent },
      { key: 'preserve_edges', label: 'Preserve edges', min: 0, max: 1, defaultValue: 0.8, ...percent }
    ],
    build: () => ({ type: 'denoise', strength: 0.3, preserve_edges: 0.8 })
  },
  {
    type: 'deblock',
    label: 'Deblock',
    description: 'Softens JPEG block boundaries.',
    fields: [{ key: 'strength', label: 'Strength', min: 0, max: 1, defaultValue: 0.5, ...percent }],
    build: () => ({ type: 'deblock', strength: 0.5 })
  },
  {
    type: 'mild_deblur',
    label: 'Mild Deblur',
    description: 'Recovers a little sharpness from a slightly soft capture.',
    fields: [
      { key: 'strength', label: 'Strength', min: 0, max: 1, defaultValue: 0.4, ...percent },
      { key: 'radius', label: 'Radius', min: 0.5, max: 3, step: 0.1, defaultValue: 1.2, format: 'decimal' }
    ],
    build: () => ({ type: 'mild_deblur', strength: 0.4, radius: 1.2 })
  },
  {
    type: 'uneven_lighting_correction',
    label: 'Uneven Lighting',
    description: 'Evens out a gradient across a scan or photograph.',
    fields: [
      { key: 'strength', label: 'Strength', min: 0, max: 1, defaultValue: 0.6, ...percent },
      { key: 'radius', label: 'Radius', min: 4, max: 96, step: 1, defaultValue: 40, format: 'integer' }
    ],
    build: () => ({ type: 'uneven_lighting_correction', strength: 0.6, radius: 40 })
  },
  {
    type: 'gaussian_blur',
    label: 'Blur',
    description: 'Softens the layers beneath.',
    fields: [
      { key: 'radius', label: 'Radius', min: 0, max: 20, step: 0.1, defaultValue: 2, format: 'decimal' }
    ],
    build: () => ({ type: 'gaussian_blur', radius: 2 })
  },
  {
    type: 'grayscale',
    label: 'Black and White',
    description: 'Converts the layers beneath to neutral grey.',
    fields: [],
    build: () => ({ type: 'grayscale' })
  },
  {
    type: 'sepia',
    label: 'Sepia',
    description: 'Applies a warm monochrome tone.',
    fields: [],
    build: () => ({ type: 'sepia' })
  }
];

export function definitionFor(type: string): AdjustmentDefinition | null {
  return adjustmentDefinitions.find((definition) => definition.type === type) ?? null;
}

/** Operations the compositor accepts but this release cannot edit in a dialog. */
export function isEditableAdjustment(operation: BaseEditOperation): boolean {
  const definition = definitionFor(operation.type);
  return Boolean(definition) && definition?.editor !== 'unsupported';
}

export function adjustmentLabel(operation: BaseEditOperation): string {
  return definitionFor(operation.type)?.label ?? operation.type;
}

/** Reads a scalar field from an operation, falling back to the field default. */
export function readField(operation: BaseEditOperation, field: ScalarField): number {
  const value = (operation as unknown as Record<string, unknown>)[field.key];
  return typeof value === 'number' && Number.isFinite(value) ? value : field.defaultValue;
}

/** Returns a copy of the operation with one scalar field replaced and clamped. */
export function writeField(
  operation: BaseEditOperation,
  field: ScalarField,
  value: number
): BaseEditOperation {
  const clamped = Math.min(field.max, Math.max(field.min, value));
  const rounded = field.format === 'integer' ? Math.round(clamped) : clamped;
  return { ...operation, [field.key]: rounded } as BaseEditOperation;
}

export function formatField(field: ScalarField, value: number): string {
  if (field.format === 'percent') return `${Math.round(value * 100)}%`;
  if (field.format === 'integer') return String(Math.round(value));
  return value.toFixed(2);
}
