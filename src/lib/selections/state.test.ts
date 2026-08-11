import { describe, expect, it } from 'vitest';
import {
  createNamedMask,
  createSelectionState,
  deleteNamedMask,
  duplicateNamedMask,
  loadNamedMask,
  MAX_NAMED_MASKS,
  moveNamedMask,
  operationModeFromModifiers,
  renameNamedMask,
  replaceNamedMask,
  SelectionHistory,
  setActiveMask,
  toggleNamedMask
} from './state';
import type { MaskSnapshot } from './types';
import { geometryFingerprint } from './geometry';

function mask(values = [0, 64, 128, 255]): MaskSnapshot {
  return {
    version: 1,
    width: 2,
    height: 2,
    encoding: 'base64_u8',
    data: btoa(String.fromCharCode(...values)).replace(/=+$/, ''),
    checksum: 'fnv1a64:0123456789abcdef'
  };
}

describe('selection state', () => {
  it('creates schema-v2 identity geometry with optional original dimensions', () => {
    expect(createSelectionState('doc', 640, 480)).toMatchObject({
      schemaVersion: 2,
      documentKey: 'doc',
      canvasWidth: 640,
      canvasHeight: 480,
      geometryOperations: [],
      geometryFingerprint: geometryFingerprint([])
    });
    expect(createSelectionState('unopened')).toMatchObject({ canvasWidth: 0, canvasHeight: 0 });
  });

  it('maps keyboard modifiers to conventional composition modes', () => {
    expect(operationModeFromModifiers('replace', false, false)).toBe('replace');
    expect(operationModeFromModifiers('replace', true, false)).toBe('add');
    expect(operationModeFromModifiers('replace', false, true)).toBe('subtract');
    expect(operationModeFromModifiers('replace', true, true)).toBe('intersect');
  });

  it('supports the named mask lifecycle by stable identifier', () => {
    let state = setActiveMask(createSelectionState('doc'), mask(), null);
    state = createNamedMask(state, 'Subject', new Date('2026-01-01T00:00:00Z'), 'mask-1');
    expect(state.namedMasks).toHaveLength(1);
    expect(state.namedMasks[0].id).toBe('mask-1');
    state = renameNamedMask(state, 'mask-1', 'Person', new Date('2026-01-02T00:00:00Z'));
    state = toggleNamedMask(state, 'mask-1', 'locked');
    expect(state.namedMasks[0]).toMatchObject({ name: 'Person', locked: true });
    state = duplicateNamedMask(state, 'mask-1', new Date('2026-01-03T00:00:00Z'), 'mask-2');
    expect(state.namedMasks.map((item) => item.id)).toEqual(['mask-1', 'mask-2']);
    state = moveNamedMask(state, 'mask-2', -1);
    expect(state.namedMasks[0].id).toBe('mask-2');
    state = deleteNamedMask(state, 'mask-2');
    expect(state.namedMasks.map((item) => item.id)).toEqual(['mask-1']);
  });

  it('does not replace locked named masks and loads immutable snapshots', () => {
    let state = setActiveMask(createSelectionState('doc'), mask([255, 0, 0, 0]), null);
    state = createNamedMask(state, 'Locked', new Date(0), 'mask-1');
    state = toggleNamedMask(state, 'mask-1', 'locked');
    state = setActiveMask(state, mask([0, 255, 0, 0]), null);
    expect(replaceNamedMask(state, 'mask-1').namedMasks[0].mask).toEqual(mask([255, 0, 0, 0]));
    state = loadNamedMask(state, 'mask-1');
    expect(state.activeMask).toEqual(mask([255, 0, 0, 0]));
    expect(state.activeMask).not.toBe(state.namedMasks[0].mask);
  });

  it('bounds named masks without discarding existing entries', () => {
    let state = setActiveMask(createSelectionState('doc', 2, 2), mask(), null);
    state.namedMasks = Array.from({ length: MAX_NAMED_MASKS }, (_, index) => ({
      id: `mask-${index}`,
      name: `Mask ${index}`,
      mask: mask(),
      visible: true,
      locked: false,
      createdAt: new Date(0).toISOString(),
      modifiedAt: new Date(0).toISOString()
    }));
    expect(createNamedMask(state, 'Overflow').namedMasks).toHaveLength(MAX_NAMED_MASKS);
    expect(duplicateNamedMask(state, 'mask-0').namedMasks).toHaveLength(MAX_NAMED_MASKS);
  });
});

describe('selection history', () => {
  it('undoes and redoes complete visible selection state', () => {
    const history = new SelectionHistory();
    const initial = createSelectionState('doc');
    history.replace(initial);
    history.commit({ ...initial, tool: 'brush' }, undefined, 100);
    history.commit({ ...history.state, mode: 'add' }, undefined, 200);
    expect(history.undo().mode).toBe('replace');
    expect(history.undo().tool).toBe('rectangle');
    expect(history.redo().tool).toBe('brush');
  });

  it('coalesces rapid setting changes into one undo entry', () => {
    const history = new SelectionHistory();
    const initial = createSelectionState('doc');
    history.replace(initial);
    history.commit({ ...initial, overlay: { ...initial.overlay, opacity: 0.2 } }, 'opacity', 100);
    history.commit({ ...history.state, overlay: { ...history.state.overlay, opacity: 0.3 } }, 'opacity', 200);
    expect(history.undo().overlay.opacity).toBe(initial.overlay.opacity);
    expect(history.canUndo).toBe(false);
  });

  it('force-coalesces a matching async geometry continuation beyond the normal time window', () => {
    const history = new SelectionHistory();
    const initial = createSelectionState('doc');
    history.replace(initial);
    history.commit({ ...initial, canvasWidth: 10 }, 'lens_correction', 100);
    history.commit(
      { ...history.state, canvasWidth: 20 },
      'lens_correction',
      10_000,
      true
    );
    expect(history.lastCommitCreatedEntry).toBe(false);
    expect(history.undoDepth).toBe(1);
    expect(history.undo().canvasWidth).toBe(initial.canvasWidth);
  });

  it('does not create undo entries for semantic no-ops', () => {
    const history = new SelectionHistory();
    const initial = createSelectionState('doc');
    history.replace(initial);
    const result = history.commit(structuredClone(initial), undefined, 99_999);
    expect(result.updatedAt).toBe(initial.updatedAt);
    expect(history.canUndo).toBe(false);
  });

  it('reports coalescing and exposes bounded retention for atomic history alignment', () => {
    const history = new SelectionHistory({ maxEntries: 2, maxBytes: 1_000_000 });
    const initial = createSelectionState('doc');
    history.replace(initial);
    history.commit({ ...initial, mode: 'add' }, 'mode', 100);
    expect(history.lastCommitCreatedEntry).toBe(true);
    history.commit({ ...history.state, mode: 'subtract' }, 'mode', 200);
    expect(history.lastCommitCreatedEntry).toBe(false);
    history.endCoalescing();
    history.commit({ ...history.state, tool: 'brush' });
    history.commit({ ...history.state, tool: 'eraser' });
    expect(history.undoDepth).toBe(2);
    history.retainUndoDepth(1);
    expect(history.undoDepth).toBe(1);
  });

  it('reports the surviving depth after byte-budget eviction', () => {
    const history = new SelectionHistory({ maxEntries: 60, maxBytes: 1 });
    const initial = createSelectionState('doc');
    history.replace(initial);
    history.commit({ ...initial, mode: 'add' });
    history.commit({ ...history.state, mode: 'subtract' });
    history.commit({ ...history.state, mode: 'intersect' });
    expect(history.undoDepth).toBe(1);
    expect(history.undo().mode).toBe('subtract');
  });
});
