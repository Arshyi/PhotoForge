import { cloneSelectionState, createSelectionState } from './state';
import type { MaskSnapshot, SelectionState } from './types';

const STORAGE_PREFIX = 'photoforge.selection-session.v1:';
const MAX_SESSION_CHARACTERS = 3_500_000;

export function documentSelectionKey(filename: string, width: number, height: number): string {
  let hash = 0x811c9dc5;
  const value = `${filename}\u0000${width}x${height}`;
  for (let index = 0; index < value.length; index += 1) {
    hash ^= value.charCodeAt(index);
    hash = Math.imul(hash, 0x01000193);
  }
  return `${width}x${height}-${(hash >>> 0).toString(16).padStart(8, '0')}`;
}

export function saveSelectionSession(
  state: SelectionState,
  storage: Pick<Storage, 'setItem'> = localStorage
): boolean {
  const json = JSON.stringify(state);
  if (json.length > MAX_SESSION_CHARACTERS) return false;
  try {
    storage.setItem(`${STORAGE_PREFIX}${state.documentKey}`, json);
    return true;
  } catch {
    return false;
  }
}

export function loadSelectionSession(
  documentKey: string,
  width: number,
  height: number,
  storage: Pick<Storage, 'getItem'> = localStorage
): SelectionState {
  const fallback = createSelectionState(documentKey);
  try {
    const raw = storage.getItem(`${STORAGE_PREFIX}${documentKey}`);
    if (!raw || raw.length > MAX_SESSION_CHARACTERS) return fallback;
    const parsed = JSON.parse(raw) as Partial<SelectionState>;
    if (parsed.schemaVersion !== 1 || parsed.documentKey !== documentKey) return fallback;
    const candidate = { ...fallback, ...parsed } as SelectionState;
    candidate.overlay = { ...fallback.overlay, ...parsed.overlay };
    candidate.settings = { ...fallback.settings, ...parsed.settings };
    candidate.namedMasks = Array.isArray(parsed.namedMasks)
      ? parsed.namedMasks.filter((mask) => validSnapshot(mask?.mask, width, height)).slice(0, 100)
      : [];
    if (!validSnapshot(parsed.activeMask, width, height)) {
      candidate.activeMask = null;
      candidate.activeDiagnostics = null;
    }
    return cloneSelectionState(candidate);
  } catch {
    return fallback;
  }
}

export function createMaskFile(
  id: string,
  name: string,
  mask: MaskSnapshot,
  createdAt: string,
  modifiedAt: string,
  sourceTool?: string
) {
  return {
    format: 'photoforge-mask' as const,
    version: 1 as const,
    id,
    name,
    mask: structuredClone(mask),
    metadata: { createdAt, modifiedAt, ...(sourceTool ? { sourceTool } : {}) }
  };
}

export function validSnapshot(
  value: MaskSnapshot | null | undefined,
  width?: number,
  height?: number
): value is MaskSnapshot {
  if (!value) return false;
  const pixels = value.width * value.height;
  return (
    value.version === 1 &&
    Number.isInteger(value.width) &&
    Number.isInteger(value.height) &&
    value.width > 0 &&
    value.height > 0 &&
    pixels <= 100_000_000 &&
    (!width || value.width === width) &&
    (!height || value.height === height) &&
    ['base64_u8', 'base64_rle_u8'].includes(value.encoding) &&
    typeof value.data === 'string' &&
    value.data.length <= Math.ceil((pixels * 4) / 3) + 4 &&
    /^fnv1a64:[0-9a-f]{16}$/.test(value.checksum)
  );
}
