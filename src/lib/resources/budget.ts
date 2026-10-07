/**
 * Words and numbers for the memory budget.
 *
 * The backend decides every limit; this only turns them into text a person can check
 * against their own machine, and into the range a manual setting may be chosen from.
 */
import { formatBytes } from '../utils/format';
import type { Budget, ResourceStatus } from './types';

export const GIB = 1024 ** 3;
export const MIB = 1024 ** 2;
/** The manual control moves in steps this big, so the slider has a sensible number of positions. */
export const MANUAL_STEP_BYTES = 256 * MIB;

/** The range a manual budget can be chosen from, exactly what the backend will accept. */
export function manualRange(status: ResourceStatus): { min: number; max: number; step: number } {
  const step = MANUAL_STEP_BYTES;
  const min = Math.ceil(status.minBudgetBytes / step) * step;
  const max = Math.max(min, Math.floor(status.budget.ceilingBytes / step) * step);
  return { min, max, step };
}

/** Moves a requested figure to the nearest step inside the range. */
export function snapBudget(bytes: number, range: { min: number; max: number; step: number }): number {
  if (!Number.isFinite(bytes)) return range.min;
  const snapped = Math.round(bytes / range.step) * range.step;
  return Math.min(range.max, Math.max(range.min, snapped));
}

export function gibibytes(bytes: number): number {
  return Math.round((bytes / GIB) * 100) / 100;
}

export function megapixels(pixels: number): string {
  const value = pixels / 1_000_000;
  return `${value >= 100 ? value.toFixed(0) : value.toFixed(1)} MP`;
}

/** One sentence on where the budget came from. */
export function describeBasis(budget: Budget): string {
  switch (budget.basis) {
    case 'measured':
      return `Worked out from this computer: its installed and currently free memory, less ${formatBytes(budget.reserveBytes)} kept back for Windows and your other programs.`;
    case 'unmeasured':
      return 'PhotoForge could not measure this computer’s memory, so it is using a conservative default rather than guessing high.';
    case 'manual':
      return budget.clamped
        ? `The figure you chose, moved into the range this computer can support (up to ${formatBytes(budget.ceilingBytes)}).`
        : 'The figure you chose.';
  }
}

/** What the budget allows, in terms of images. */
export function describeLimits(status: ResourceStatus): string {
  return `The largest image PhotoForge will open whole under this budget is about ${megapixels(status.maxWorkingPixels)}. A larger one is offered as a region or a smaller copy instead of being refused.`;
}

/** Whether a manual figure is lower than the one in force, so it waits for the next document. */
export function waitsForNextDocument(status: ResourceStatus, requestedBytes: number): boolean {
  return requestedBytes < status.budget.bytes;
}

/** A warning when a manual figure leaves little for everything else, or `null`. */
export function manualWarning(status: ResourceStatus, requestedBytes: number): string | null {
  const system = status.system;
  if (!system) return null;
  const left = system.totalPhysical - requestedBytes;
  if (left < system.totalPhysical * 0.15) {
    return `That leaves only ${formatBytes(Math.max(0, left))} of ${formatBytes(system.totalPhysical)} for Windows and your other programs. PhotoForge may slow your computer down.`;
  }
  if (requestedBytes > system.availablePhysical) {
    return `Only ${formatBytes(system.availablePhysical)} is free right now, less than this budget. Close other programs before opening a very large image.`;
  }
  return null;
}
