/**
 * What the backend reports about the machine and the memory budget.
 *
 * These mirror `commands::resources` and `resources::policy`. Every number here was
 * measured or chosen by the backend; the interface draws them and never derives its
 * own limits, so what it says cannot disagree with what is enforced.
 */
import type { RenderCacheStats } from '../types/editor';

export type BudgetMode = { mode: 'automatic' } | { mode: 'manual'; bytes: number };

/** Where a budget figure came from, so a guess is never presented as a measurement. */
export type BudgetBasis = 'measured' | 'unmeasured' | 'manual';

export interface Budget {
  mode: BudgetMode;
  basis: BudgetBasis;
  bytes: number;
  reserveBytes: number;
  ceilingBytes: number;
  /** A manual request that had to be moved into range. */
  clamped: boolean;
}

export interface SystemMemory {
  totalPhysical: number;
  availablePhysical: number;
}

export interface ProcessMemory {
  workingSet: number;
  peakWorkingSet: number;
  privateBytes: number;
}

export interface ResourceLimits {
  jobBytes: number;
  workingImageBytes: number;
  storeBytes: number;
  entryBytes: number;
}

export interface ResourceStatus {
  /** `null` when the machine could not be measured. */
  system: SystemMemory | null;
  process: ProcessMemory | null;
  budget: Budget;
  minBudgetBytes: number;
  limits: ResourceLimits;
  maxWorkingPixels: number;
  residentPixelBytes: number;
  renderCache: RenderCacheStats;
  /** `null` is "unknown", not "plenty". */
  diskFreeBytes: number | null;
  gpuMemory: { measurable: boolean; reason: string };
  /** A lower budget is saved but not yet in force. */
  reductionPending: boolean;
}

export interface ModeChange {
  budget: Budget;
  /** A lower budget is saved, and applies when the next document opens. */
  applied: boolean;
}
