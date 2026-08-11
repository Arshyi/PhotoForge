import type { EditOperation } from '../types/editor';
import { cloneOperations, maskedOperation } from '../utils/operations';
import { setActiveMask } from './state';
import type { MaskDiagnostics, MaskSnapshot, SelectionState } from './types';

export interface RefineColorSettings {
  enabled: boolean;
  strength: number;
  radius: number;
}

export interface RefineApplyTransaction {
  operations: EditOperation[];
  selection: SelectionState;
  includesImageEdit: boolean;
}

export function buildRefineApplyTransaction(
  operations: EditOperation[],
  selection: SelectionState,
  mask: MaskSnapshot,
  diagnostics: MaskDiagnostics,
  color: RefineColorSettings
): RefineApplyTransaction {
  const nextSelection = setActiveMask(
    { ...selection, overlay: { ...selection.overlay, visible: true } },
    mask,
    diagnostics
  );
  const nextOperations = cloneOperations(operations);
  if (!color.enabled) {
    return { operations: nextOperations, selection: nextSelection, includesImageEdit: false };
  }
  if (!Number.isFinite(color.strength) || color.strength < 0 || color.strength > 1 ||
    !Number.isSafeInteger(color.radius) || color.radius < 1 || color.radius > 32) {
    throw new Error('Decontaminate Colors settings are invalid.');
  }
  nextOperations.push(maskedOperation({
    type: 'decontaminate_colors',
    enabled: true,
    strength: color.strength,
    radius: color.radius
  }, mask, 'inside'));
  return { operations: nextOperations, selection: nextSelection, includesImageEdit: true };
}
