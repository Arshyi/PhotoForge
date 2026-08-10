import type { Point, ResolvedBrushSample } from './types';

export interface PressureSettings {
  enabled: boolean;
  affectsSize: boolean;
  affectsOpacity: boolean;
  minimumSizeFactor: number;
  minimumOpacityFactor: number;
}

export interface PointerPressureInput {
  pointerType?: string;
  pressure?: number;
}

export function normalizePointerPressure(
  input: PointerPressureInput,
  enabled: boolean
): number | null {
  if (!enabled || input.pointerType !== 'pen' || !Number.isFinite(input.pressure)) return null;
  return Math.max(0, Math.min(1, input.pressure as number));
}

export function resolveBrushSample(
  point: Point,
  input: PointerPressureInput,
  baseDiameter: number,
  baseOpacity: number,
  settings: PressureSettings
): ResolvedBrushSample {
  const diameter = clampFinite(baseDiameter, 1, 512, 48);
  const opacity = clampFinite(baseOpacity, 0.01, 1, 1);
  const pressure = normalizePointerPressure(input, settings.enabled);
  if (pressure === null) return { ...point, diameter, opacity };

  const sizeFloor = clampFinite(settings.minimumSizeFactor, 0.05, 1, 0.35);
  const opacityFloor = clampFinite(settings.minimumOpacityFactor, 0.01, 1, 0.25);
  const sizeFactor = settings.affectsSize ? sizeFloor + (1 - sizeFloor) * pressure : 1;
  const opacityFactor = settings.affectsOpacity
    ? opacityFloor + (1 - opacityFloor) * pressure
    : 1;

  return {
    ...point,
    diameter: quantize(clampFinite(diameter * sizeFactor, 1, 512, diameter)),
    opacity: quantize(clampFinite(opacity * opacityFactor, 0.01, 1, opacity))
  };
}

export function shouldAppendPointerUpSample(
  input: PointerPressureInput,
  settings: PressureSettings,
  existingSamples: number
): boolean {
  const pressure = normalizePointerPressure(input, settings.enabled);
  return !(existingSamples > 0 && input.pointerType === 'pen' && pressure === 0);
}

function clampFinite(value: number, minimum: number, maximum: number, fallback: number): number {
  return Number.isFinite(value) ? Math.max(minimum, Math.min(maximum, value)) : fallback;
}

function quantize(value: number): number {
  return Math.round(value * 1_000_000) / 1_000_000;
}
