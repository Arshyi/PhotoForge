/** Fixtures for tests of the memory settings. A 16 GB machine with an automatic 6 GB budget. */
import { GIB, MIB } from './budget';
import type { Budget, ResourceStatus } from './types';

export function statusFixture(overrides: Partial<ResourceStatus> = {}, budget: Partial<Budget> = {}): ResourceStatus {
  return {
    system: { totalPhysical: 16 * GIB, availablePhysical: 9 * GIB },
    process: { workingSet: 300 * MIB, peakWorkingSet: 400 * MIB, privateBytes: 280 * MIB },
    budget: {
      mode: { mode: 'automatic' },
      basis: 'measured',
      bytes: 6 * GIB,
      reserveBytes: 2.4 * GIB,
      ceilingBytes: 14.4 * GIB,
      clamped: false,
      ...budget
    },
    minBudgetBytes: 512 * MIB,
    limits: { jobBytes: 6 * GIB, workingImageBytes: 1.5 * GIB, storeBytes: 1.5 * GIB, entryBytes: 1.5 * GIB },
    maxWorkingPixels: 100_663_296,
    residentPixelBytes: 64 * MIB,
    renderCache: {
      hits: 0, misses: 0, evictions: 0, refusals: 0, entries: 0, bytes: 0, capacityBytes: 256 * MIB,
      diskEntries: 0, diskBytes: 0, diskCapacityBytes: 0
    },
    diskFreeBytes: 120 * GIB,
    gpuMemory: { measurable: false, reason: 'Video memory is not reliably measurable.' },
    reductionPending: false,
    ...overrides
  };
}
