import type {
  BaseEditOperation,
  CurvePoint,
  CurveSet,
  HslAdjustment,
  HslSettings,
  OperationType
} from '../types/editor';

/** Mirrors the curve bounds `EditOperation::validate` enforces in Rust. */
export const MIN_CURVE_POINTS = 2;
export const MAX_CURVE_POINTS = 32;
/** Smallest gap kept between neighbouring inputs so they stay strictly sorted. */
export const MIN_CURVE_GAP = 0.004;

export const curveChannels: (keyof CurveSet)[] = ['rgb', 'red', 'green', 'blue'];

export function identityCurve(): CurvePoint[] {
  return [
    { input: 0, output: 0 },
    { input: 1, output: 1 }
  ];
}

export function identityCurves(): CurveSet {
  return {
    rgb: identityCurve(),
    red: identityCurve(),
    green: identityCurve(),
    blue: identityCurve()
  };
}

export function isIdentityCurve(points: CurvePoint[]): boolean {
  return (
    points.length === 2 &&
    points[0].input === 0 &&
    points[0].output === 0 &&
    points[1].input === 1 &&
    points[1].output === 1
  );
}

const clampUnit = (value: number) =>
  Number.isFinite(value) ? Math.min(1, Math.max(0, value)) : 0;

/**
 * Moves one curve point, keeping the shape valid for the backend.
 *
 * The first and last points are anchored to inputs 0 and 1 — validation
 * requires exactly that — so only their outputs move. Interior points stay
 * strictly between their neighbours by at least `MIN_CURVE_GAP`.
 */
export function moveCurvePoint(
  points: CurvePoint[],
  index: number,
  input: number,
  output: number
): CurvePoint[] {
  if (index < 0 || index >= points.length) return points;
  const next = points.map((point) => ({ ...point }));
  next[index].output = clampUnit(output);
  if (index > 0 && index < next.length - 1) {
    const lower = next[index - 1].input + MIN_CURVE_GAP;
    const upper = next[index + 1].input - MIN_CURVE_GAP;
    next[index].input = upper < lower ? lower : Math.min(upper, Math.max(lower, clampUnit(input)));
  }
  return next;
}

/** Inserts a point, refusing a position that would collide with a neighbour. */
export function addCurvePoint(
  points: CurvePoint[],
  input: number,
  output: number
): CurvePoint[] {
  if (points.length >= MAX_CURVE_POINTS) return points;
  const x = clampUnit(input);
  if (points.some((point) => Math.abs(point.input - x) < MIN_CURVE_GAP)) return points;
  if (x <= 0 || x >= 1) return points;
  return [...points, { input: x, output: clampUnit(output) }].sort(
    (left, right) => left.input - right.input
  );
}

/** Removes an interior point. The two anchors can never be removed. */
export function removeCurvePoint(points: CurvePoint[], index: number): CurvePoint[] {
  if (index <= 0 || index >= points.length - 1) return points;
  if (points.length <= MIN_CURVE_POINTS) return points;
  return points.filter((_, position) => position !== index);
}

/** Samples the piecewise-linear curve, matching how the backend interpolates. */
export function sampleCurve(points: CurvePoint[], input: number): number {
  const x = clampUnit(input);
  if (points.length === 0) return x;
  if (x <= points[0].input) return points[0].output;
  for (let index = 1; index < points.length; index += 1) {
    const previous = points[index - 1];
    const current = points[index];
    if (x <= current.input) {
      const span = current.input - previous.input;
      if (span <= 0) return current.output;
      const ratio = (x - previous.input) / span;
      return previous.output + (current.output - previous.output) * ratio;
    }
  }
  return points[points.length - 1].output;
}

/** True when the point list satisfies every rule the backend validates. */
export function isValidCurve(points: CurvePoint[]): boolean {
  if (points.length < MIN_CURVE_POINTS || points.length > MAX_CURVE_POINTS) return false;
  if (points[0].input !== 0 || points[points.length - 1].input !== 1) return false;
  return points.every(
    (point, index) =>
      Number.isFinite(point.input) &&
      Number.isFinite(point.output) &&
      point.input >= 0 &&
      point.input <= 1 &&
      point.output >= 0 &&
      point.output <= 1 &&
      (index === 0 || points[index - 1].input < point.input)
  );
}

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
  editor?: 'levels' | 'hsl' | 'curves' | 'unsupported';
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
    type: 'curves',
    label: 'Curves',
    description: 'Reshapes tones with an editable curve per channel.',
    fields: [],
    editor: 'curves',
    build: () => ({ type: 'curves', curves: identityCurves() })
  },
  {
    type: 'selective_color',
    label: 'Selective Color',
    description: 'Shifts one hue band toward or away from cyan, magenta, yellow, and black.',
    fields: [
      { key: 'target_hue', label: 'Target hue', min: 0, max: 360, step: 1, defaultValue: 0, format: 'integer' },
      { key: 'width', label: 'Band width', min: 1, max: 180, step: 1, defaultValue: 60, format: 'integer' },
      { key: 'adjustment.cyan', label: 'Cyan', min: -1, max: 1, defaultValue: 0, ...percent },
      { key: 'adjustment.magenta', label: 'Magenta', min: -1, max: 1, defaultValue: 0, ...percent },
      { key: 'adjustment.yellow', label: 'Yellow', min: -1, max: 1, defaultValue: 0, ...percent },
      { key: 'adjustment.black', label: 'Black', min: -1, max: 1, defaultValue: 0, ...percent }
    ],
    build: () => ({
      type: 'selective_color',
      target_hue: 0,
      width: 60,
      adjustment: { cyan: 0, magenta: 0, yellow: 0, black: 0 }
    })
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
    type: 'raw_development',
    label: 'RAW Development',
    description: 'Applies non-destructive linear-light photographic controls.',
    fields: [],
    editor: 'unsupported',
    build: () => ({
      type: 'raw_development',
      parameters: {
        whiteBalance: { mode: 'asShot', multipliers: [1, 1, 1] },
        exposureEv: 0,
        contrast: 0,
        highlights: 0,
        shadows: 0,
        whites: 0,
        blacks: 0
      }
    })
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

/**
 * Reads a scalar field from an operation, falling back to the field default.
 *
 * A dotted key reaches one level into a nested settings object, which is how
 * selective colour addresses its CMYK amounts without needing a bespoke editor.
 */
export function readField(operation: BaseEditOperation, field: ScalarField): number {
  const record = operation as unknown as Record<string, unknown>;
  const [head, tail] = field.key.split('.');
  const container = tail ? (record[head] as Record<string, unknown> | undefined) : record;
  const value = container?.[tail ?? head];
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
  const [head, tail] = field.key.split('.');
  if (!tail) return { ...operation, [head]: rounded } as BaseEditOperation;
  const record = operation as unknown as Record<string, unknown>;
  const nested = { ...((record[head] as Record<string, unknown>) ?? {}), [tail]: rounded };
  return { ...operation, [head]: nested } as BaseEditOperation;
}

export function formatField(field: ScalarField, value: number): string {
  if (field.format === 'percent') return `${Math.round(value * 100)}%`;
  if (field.format === 'integer') return String(Math.round(value));
  return value.toFixed(2);
}
