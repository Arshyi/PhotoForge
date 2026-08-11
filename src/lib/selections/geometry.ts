import type { EditOperation } from '../types/editor';
import type {
  GeometryOperation,
  GeometryPerspectiveCorners
} from './types';

const MAX_GEOMETRY_OPERATIONS = 64;
const MAX_CANVAS_PIXELS = 100_000_000;
const FNV_OFFSET = 0xcbf29ce484222325n;
const FNV_PRIME = 0x100000001b3n;
const FNV_MASK = 0xffffffffffffffffn;

export interface CanvasDimensions {
  width: number;
  height: number;
}

export function extractGeometryOperations(operations: EditOperation[]): GeometryOperation[] {
  const candidates = operations.filter((operation) => isGeometryType(operation.type));
  const canonical = canonicalizeGeometryOperations(candidates);
  if (!canonical) throw new Error('Edit pipeline contains invalid geometry.');
  return canonical;
}

export function canonicalizeGeometryOperations(value: unknown): GeometryOperation[] | null {
  if (!Array.isArray(value) || value.length > MAX_GEOMETRY_OPERATIONS) return null;
  const operations: GeometryOperation[] = [];
  for (const candidate of value) {
    const operation = canonicalGeometryOperation(candidate);
    if (!operation) return null;
    operations.push(operation);
  }
  return operations;
}

export function geometryOperationsToEditOperations(
  operations: GeometryOperation[]
): EditOperation[] {
  const canonical = canonicalizeGeometryOperations(operations);
  if (!canonical) throw new Error('Geometry operations are invalid.');
  return structuredClone(canonical) as EditOperation[];
}

export function computeStageDimensions(
  sourceWidth: number,
  sourceHeight: number,
  operations: GeometryOperation[]
): CanvasDimensions {
  validateCanvasDimensions(sourceWidth, sourceHeight);
  const canonical = canonicalizeGeometryOperations(operations);
  if (!canonical) throw new Error('Geometry operations are invalid.');
  let width = sourceWidth;
  let height = sourceHeight;

  for (const operation of canonical) {
    if (operation.type === 'crop') {
      const left = Math.floor(f32Product(operation.x, width));
      const top = Math.floor(f32Product(operation.y, height));
      if (left >= width || top >= height) throw new Error('Crop starts outside the current canvas.');
      width = Math.min(width - left, Math.max(1, Math.round(f32Product(operation.width, width))));
      height = Math.min(height - top, Math.max(1, Math.round(f32Product(operation.height, height))));
    } else if (operation.type === 'rotate' && (operation.degrees === 90 || operation.degrees === 270)) {
      [width, height] = [height, width];
    } else if (operation.type === 'lens_correction') {
      validateLensMapping(width, height, operation.distortion);
    }
    validateCanvasDimensions(width, height);
  }

  return { width, height };
}

export function geometryFingerprint(operations: GeometryOperation[]): string {
  const canonical = canonicalizeGeometryOperations(operations);
  if (!canonical) throw new Error('Geometry operations are invalid.');
  const serialized = JSON.stringify(canonical);
  let hash = FNV_OFFSET;
  for (let index = 0; index < serialized.length; index += 1) {
    const code = serialized.charCodeAt(index);
    hash ^= BigInt(code & 0xff);
    hash = (hash * FNV_PRIME) & FNV_MASK;
    hash ^= BigInt(code >>> 8);
    hash = (hash * FNV_PRIME) & FNV_MASK;
  }
  return `geometry-v1:${hash.toString(16).padStart(16, '0')}`;
}

function canonicalGeometryOperation(value: unknown): GeometryOperation | null {
  if (!value || typeof value !== 'object' || !('type' in value)) return null;
  const candidate = value as Record<string, unknown>;
  if (candidate.type === 'rotate') {
    if (!hasOnlyKeys(candidate, ['type', 'degrees'])) return null;
    if (!Number.isSafeInteger(candidate.degrees)) return null;
    const degrees = ((Number(candidate.degrees) % 360) + 360) % 360;
    if (![0, 90, 180, 270].includes(degrees)) return null;
    return { type: 'rotate', degrees };
  }
  if (candidate.type === 'straighten') {
    if (!hasOnlyKeys(candidate, ['type', 'degrees'])) return null;
    if (!finiteRange(candidate.degrees, -45, 45)) return null;
    return { type: 'straighten', degrees: normalizedNumber(Number(candidate.degrees)) };
  }
  if (candidate.type === 'crop') {
    if (!hasOnlyKeys(candidate, ['type', 'x', 'y', 'width', 'height', 'aspect_ratio', 'overlay'])) {
      return null;
    }
    if (
      !finiteRange(candidate.x, 0, 1.000_001) ||
      !finiteRange(candidate.y, 0, 1.000_001) ||
      !finiteRange(candidate.width, Number.MIN_VALUE, 1.000_001) ||
      !finiteRange(candidate.height, Number.MIN_VALUE, 1.000_001)
    ) return null;
    const x = normalizedNumber(Number(candidate.x));
    const y = normalizedNumber(Number(candidate.y));
    const width = normalizedNumber(Number(candidate.width));
    const height = normalizedNumber(Number(candidate.height));
    if (x + width > 1.000_001 || y + height > 1.000_001) return null;
    if (
      candidate.aspect_ratio !== null &&
      (typeof candidate.aspect_ratio !== 'string' || candidate.aspect_ratio.length > 32)
    ) return null;
    if (candidate.overlay !== 'none' && candidate.overlay !== 'rule_of_thirds' &&
      candidate.overlay !== 'golden_ratio') return null;
    return {
      type: 'crop', x, y, width, height,
      aspect_ratio: candidate.aspect_ratio as string | null,
      overlay: candidate.overlay as Extract<GeometryOperation, { type: 'crop' }>['overlay']
    };
  }
  if (candidate.type === 'reflect_horizontal') {
    return hasOnlyKeys(candidate, ['type']) ? { type: 'reflect_horizontal' } : null;
  }
  if (candidate.type === 'perspective') {
    if (!hasOnlyKeys(candidate, ['type', 'corners'])) return null;
    const corners = canonicalCorners(candidate.corners);
    return corners ? { type: 'perspective', corners } : null;
  }
  if (candidate.type === 'lens_correction') {
    if (!hasOnlyKeys(candidate, [
      'type', 'distortion', 'vignetting', 'chromatic_aberration'
    ])) return null;
    if (
      !finiteRange(candidate.distortion, -0.16, 1) ||
      !finiteRange(candidate.vignetting, -1, 1) ||
      !finiteRange(candidate.chromatic_aberration, -1, 1)
    ) return null;
    return {
      type: 'lens_correction',
      distortion: normalizedNumber(Number(candidate.distortion)),
      vignetting: normalizedNumber(Number(candidate.vignetting)),
      chromatic_aberration: normalizedNumber(Number(candidate.chromatic_aberration))
    };
  }
  return null;
}

function canonicalCorners(value: unknown): GeometryPerspectiveCorners | null {
  if (!value || typeof value !== 'object') return null;
  const corners = value as Record<string, unknown>;
  if (!hasOnlyKeys(corners, ['topLeft', 'topRight', 'bottomRight', 'bottomLeft'])) return null;
  const topLeft = canonicalPoint(corners.topLeft);
  const topRight = canonicalPoint(corners.topRight);
  const bottomRight = canonicalPoint(corners.bottomRight);
  const bottomLeft = canonicalPoint(corners.bottomLeft);
  if (!topLeft || !topRight || !bottomRight || !bottomLeft) return null;
  if (
    topLeft[0] >= topRight[0] || bottomLeft[0] >= bottomRight[0] ||
    topLeft[1] >= bottomLeft[1] || topRight[1] >= bottomRight[1]
  ) return null;
  return { topLeft, topRight, bottomRight, bottomLeft };
}

function canonicalPoint(value: unknown): [number, number] | null {
  if (!Array.isArray(value) || value.length !== 2) return null;
  if (!finiteRange(value[0], 0, 1) || !finiteRange(value[1], 0, 1)) return null;
  return [normalizedNumber(Number(value[0])), normalizedNumber(Number(value[1]))];
}

function finiteRange(value: unknown, minimum: number, maximum: number): boolean {
  return typeof value === 'number' && Number.isFinite(value) && value >= minimum && value <= maximum;
}

function normalizedNumber(value: number): number {
  return Object.is(value, -0) ? 0 : value;
}

function f32Product(factor: number, dimension: number): number {
  return Math.fround(Math.fround(factor) * Math.fround(dimension));
}

function hasOnlyKeys(value: Record<string, unknown>, expected: string[]): boolean {
  const keys = Object.keys(value);
  return keys.length === expected.length && expected.every((key) => key in value);
}

function isGeometryType(value: string): value is GeometryOperation['type'] {
  return value === 'crop' || value === 'rotate' || value === 'reflect_horizontal' ||
    value === 'straighten' || value === 'perspective' || value === 'lens_correction';
}

function validateCanvasDimensions(width: number, height: number): void {
  const pixels = width * height;
  if (
    !Number.isSafeInteger(width) || !Number.isSafeInteger(height) ||
    width < 1 || height < 1 || !Number.isSafeInteger(pixels) || pixels > MAX_CANVAS_PIXELS
  ) throw new Error('Canvas dimensions are invalid or exceed the bounded limit.');
}

function validateLensMapping(width: number, height: number, value: number): void {
  const distortion = Math.fround(value);
  if (distortion >= 0) return;
  const maximumRadiusSquared = Number(width > 1) + Number(height > 1);
  const tangentialJacobian = 1 + distortion * maximumRadiusSquared;
  const radialJacobian = 1 + 3 * distortion * maximumRadiusSquared;
  if (tangentialJacobian <= 1e-6 || radialJacobian <= 1e-6) {
    throw new Error('Lens distortion folds the canvas or is too close to singular.');
  }
}
