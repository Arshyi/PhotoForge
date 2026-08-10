import type {
  GeometryOperation,
  MaskDiagnostics,
  MaskSnapshot,
  NamedMask,
  SelectionState,
  SelectionTool
} from './types';
import { geometryFingerprint } from './geometry';

const MAX_HISTORY_BYTES = 64 * 1024 * 1024;
const MAX_HISTORY_ENTRIES = 60;
export const MAX_NAMED_MASKS = 100;

export function createSelectionState(
  documentKey = '',
  width?: number,
  height?: number
): SelectionState {
  const canvas = initialCanvasDimensions(width, height);
  const geometryOperations: GeometryOperation[] = [];
  return {
    schemaVersion: 2,
    documentKey,
    canvasWidth: canvas.width,
    canvasHeight: canvas.height,
    geometryOperations,
    geometryFingerprint: geometryFingerprint(geometryOperations),
    activeMask: null,
    activeDiagnostics: null,
    namedMasks: [],
    tool: 'rectangle',
    mode: 'replace',
    applyScope: 'global',
    overlay: {
      visible: true,
      mode: 'color',
      opacity: 0.42,
      color: '#ef5b5b'
    },
    settings: {
      brushDiameter: 48,
      brushHardness: 0.75,
      brushOpacity: 1,
      pressureEnabled: false,
      pressureAffectsSize: true,
      pressureAffectsOpacity: false,
      pressureMinSizeFactor: 0.35,
      pressureMinOpacityFactor: 0.25,
      wandTolerance: 0.12,
      wandConnectivity: 'eight',
      wandAntiAlias: true,
      wandContiguous: true,
      sampleMerged: true,
      colorTolerance: 0.14,
      luminanceSensitivity: 0.7,
      hueSensitivity: 1,
      saturationSensitivity: 0.75,
      fixedAspect: false,
      fromCenter: false
    },
    panelCollapsed: false,
    updatedAt: new Date(0).toISOString()
  };
}

interface HistoryEntry {
  state: SelectionState;
  bytes: number;
}

export class SelectionHistory {
  private current = createSelectionState();
  private undoStack: HistoryEntry[] = [];
  private redoStack: HistoryEntry[] = [];
  private undoBytes = 0;
  private coalesceKey: string | null = null;
  private coalesceAt = 0;
  private pushedOnLastCommit = false;
  private readonly maxEntries: number;
  private readonly maxBytes: number;

  constructor(options: { maxEntries?: number; maxBytes?: number } = {}) {
    this.maxEntries = options.maxEntries ?? MAX_HISTORY_ENTRIES;
    this.maxBytes = options.maxBytes ?? MAX_HISTORY_BYTES;
  }

  get state(): SelectionState {
    return cloneSelectionState(this.current);
  }

  get canUndo(): boolean {
    return this.undoStack.length > 0;
  }

  get canRedo(): boolean {
    return this.redoStack.length > 0;
  }

  get undoDepth(): number {
    return this.undoStack.length;
  }

  get redoDepth(): number {
    return this.redoStack.length;
  }

  get lastCommitCreatedEntry(): boolean {
    return this.pushedOnLastCommit;
  }

  replace(state: SelectionState): SelectionState {
    this.current = cloneSelectionState(state);
    this.undoStack = [];
    this.redoStack = [];
    this.undoBytes = 0;
    this.pushedOnLastCommit = false;
    this.endCoalescing();
    return this.state;
  }

  commit(state: SelectionState, coalesceKey?: string, now = Date.now()): SelectionState {
    this.pushedOnLastCommit = false;
    const candidate = cloneSelectionState({ ...state, updatedAt: this.current.updatedAt });
    if (JSON.stringify(candidate) === JSON.stringify(this.current)) return this.state;
    const next = { ...candidate, updatedAt: new Date(now).toISOString() };
    const canCoalesce =
      coalesceKey !== undefined && this.coalesceKey === coalesceKey && now - this.coalesceAt <= 700;
    if (!canCoalesce) {
      this.pushUndo(this.current);
      this.pushedOnLastCommit = true;
    }
    this.current = next;
    this.redoStack = [];
    this.coalesceKey = coalesceKey ?? null;
    this.coalesceAt = now;
    return this.state;
  }

  undo(): SelectionState {
    const previous = this.undoStack.pop();
    if (!previous) return this.state;
    this.undoBytes -= previous.bytes;
    this.redoStack.push(entry(this.current));
    this.current = previous.state;
    this.endCoalescing();
    return this.state;
  }

  redo(): SelectionState {
    const next = this.redoStack.pop();
    if (!next) return this.state;
    this.pushUndo(this.current);
    this.current = next.state;
    this.endCoalescing();
    return this.state;
  }

  endCoalescing(): void {
    this.coalesceKey = null;
    this.coalesceAt = 0;
  }

  clearRedo(): void {
    this.redoStack = [];
  }

  retainUndoDepth(depth: number): void {
    const bounded = Math.max(0, Math.min(this.undoStack.length, Math.floor(depth)));
    while (this.undoStack.length > bounded) {
      const removed = this.undoStack.shift();
      if (removed) this.undoBytes -= removed.bytes;
    }
  }

  retainRedoDepth(depth: number): void {
    const bounded = Math.max(0, Math.min(this.redoStack.length, Math.floor(depth)));
    if (this.redoStack.length > bounded) this.redoStack.splice(0, this.redoStack.length - bounded);
  }

  private pushUndo(state: SelectionState): void {
    const snapshot = entry(state);
    this.undoStack.push(snapshot);
    this.undoBytes += snapshot.bytes;
    while (
      this.undoStack.length > 1 &&
      (this.undoStack.length > this.maxEntries || this.undoBytes > this.maxBytes)
    ) {
      const removed = this.undoStack.shift();
      if (removed) this.undoBytes -= removed.bytes;
    }
  }
}

export function setActiveMask(
  state: SelectionState,
  mask: MaskSnapshot | null,
  diagnostics: MaskDiagnostics | null
): SelectionState {
  return cloneSelectionState({ ...state, activeMask: mask, activeDiagnostics: diagnostics });
}

export function createNamedMask(
  state: SelectionState,
  name: string,
  now = new Date(),
  id = createMaskId()
): SelectionState {
  if (!state.activeMask || state.namedMasks.length >= MAX_NAMED_MASKS) return cloneSelectionState(state);
  const timestamp = now.toISOString();
  const named: NamedMask = {
    id,
    name: normalizedName(name, state.namedMasks.length + 1),
    mask: structuredClone(state.activeMask),
    visible: true,
    locked: false,
    createdAt: timestamp,
    modifiedAt: timestamp,
    sourceTool: state.tool
  };
  return cloneSelectionState({ ...state, namedMasks: [...state.namedMasks, named] });
}

export function renameNamedMask(
  state: SelectionState,
  id: string,
  name: string,
  now = new Date()
): SelectionState {
  return updateNamedMask(state, id, (mask) => ({
    ...mask,
    name: normalizedName(name, 1),
    modifiedAt: now.toISOString()
  }));
}

export function duplicateNamedMask(
  state: SelectionState,
  id: string,
  now = new Date(),
  duplicateId = createMaskId()
): SelectionState {
  if (state.namedMasks.length >= MAX_NAMED_MASKS) return cloneSelectionState(state);
  const index = state.namedMasks.findIndex((mask) => mask.id === id);
  if (index < 0) return cloneSelectionState(state);
  const timestamp = now.toISOString();
  const duplicate: NamedMask = {
    ...structuredClone(state.namedMasks[index]),
    id: duplicateId,
    name: `${state.namedMasks[index].name} Copy`.slice(0, 120),
    createdAt: timestamp,
    modifiedAt: timestamp,
    locked: false
  };
  const masks = state.namedMasks.map((mask) => structuredClone(mask));
  masks.splice(index + 1, 0, duplicate);
  return cloneSelectionState({ ...state, namedMasks: masks });
}

export function deleteNamedMask(state: SelectionState, id: string): SelectionState {
  return cloneSelectionState({
    ...state,
    namedMasks: state.namedMasks.filter((mask) => mask.id !== id)
  });
}

export function toggleNamedMask(
  state: SelectionState,
  id: string,
  key: 'visible' | 'locked'
): SelectionState {
  return updateNamedMask(state, id, (mask) => ({ ...mask, [key]: !mask[key] }));
}

export function moveNamedMask(state: SelectionState, id: string, delta: -1 | 1): SelectionState {
  const index = state.namedMasks.findIndex((mask) => mask.id === id);
  const destination = index + delta;
  if (index < 0 || destination < 0 || destination >= state.namedMasks.length) {
    return cloneSelectionState(state);
  }
  const masks = state.namedMasks.map((mask) => structuredClone(mask));
  [masks[index], masks[destination]] = [masks[destination], masks[index]];
  return cloneSelectionState({ ...state, namedMasks: masks });
}

export function loadNamedMask(state: SelectionState, id: string): SelectionState {
  const named = state.namedMasks.find((mask) => mask.id === id);
  if (!named) return cloneSelectionState(state);
  return setActiveMask(state, structuredClone(named.mask), null);
}

export function replaceNamedMask(
  state: SelectionState,
  id: string,
  now = new Date()
): SelectionState {
  if (!state.activeMask) return cloneSelectionState(state);
  return updateNamedMask(state, id, (mask) =>
    mask.locked
      ? mask
      : {
          ...mask,
          mask: structuredClone(state.activeMask as MaskSnapshot),
          modifiedAt: now.toISOString(),
          sourceTool: state.tool
        }
  );
}

export function cloneSelectionState(state: SelectionState): SelectionState {
  return structuredClone(state);
}

export function operationModeFromModifiers(
  configured: SelectionState['mode'],
  shiftKey: boolean,
  altKey: boolean
): SelectionState['mode'] {
  if (shiftKey && altKey) return 'intersect';
  if (shiftKey) return 'add';
  if (altKey) return 'subtract';
  return configured;
}

export function isPaintTool(tool: SelectionTool): boolean {
  return tool === 'brush' || tool === 'eraser';
}

function updateNamedMask(
  state: SelectionState,
  id: string,
  update: (mask: NamedMask) => NamedMask
): SelectionState {
  return cloneSelectionState({
    ...state,
    namedMasks: state.namedMasks.map((mask) =>
      mask.id === id ? update(structuredClone(mask)) : structuredClone(mask)
    )
  });
}

function entry(state: SelectionState): HistoryEntry {
  const snapshot = cloneSelectionState(state);
  return { state: snapshot, bytes: JSON.stringify(snapshot).length * 2 };
}

function normalizedName(name: string, fallback: number): string {
  return (name.trim() || `Mask ${fallback}`).slice(0, 120);
}

function createMaskId(): string {
  return globalThis.crypto?.randomUUID?.() ?? `mask-${Date.now()}-${Math.random().toString(16).slice(2)}`;
}

function initialCanvasDimensions(
  width: number | undefined,
  height: number | undefined
): { width: number; height: number } {
  if (width === undefined && height === undefined) return { width: 0, height: 0 };
  const pixels = Number(width) * Number(height);
  if (
    Number.isSafeInteger(width) && Number.isSafeInteger(height) &&
    Number(width) > 0 && Number(height) > 0 &&
    Number.isSafeInteger(pixels) && pixels <= 100_000_000
  ) {
    return { width: Number(width), height: Number(height) };
  }
  return { width: 0, height: 0 };
}
