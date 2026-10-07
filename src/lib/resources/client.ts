import { invoke } from '@tauri-apps/api/core';
import type { BudgetMode, ModeChange, ResourceStatus } from './types';

/** The machine, the budget in force and what is currently held. */
export function resourceStatus(): Promise<ResourceStatus> {
  return invoke<ResourceStatus>('resource_status');
}

/** Changes the budget. A higher one applies at once; a lower one at the next document. */
export function setMemoryBudget(mode: BudgetMode): Promise<[ModeChange, ResourceStatus]> {
  return invoke<[ModeChange, ResourceStatus]>('set_memory_budget', { mode });
}
