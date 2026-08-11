import { describe, expect, it } from 'vitest';
import { EditHistory } from './history';

describe('EditHistory', () => {
  it('replaces restored session operations without creating an undo entry', () => {
    const history = new EditHistory();
    const restored = history.replace([{ type: 'rotate', degrees: 90 }]);
    expect(restored).toEqual([{ type: 'rotate', degrees: 90 }]);
    expect(history.canUndo).toBe(false);
    history.commit([{ type: 'rotate', degrees: 180 }]);
    expect(history.undo()).toEqual([{ type: 'rotate', degrees: 90 }]);
  });

  it('reports coalesced pushes and supports synchronized oldest-entry retention', () => {
    const history = new EditHistory(2);
    history.commit([{ type: 'brightness', amount: 0.1 }], 'brightness', 100);
    expect(history.lastCommitCreatedEntry).toBe(true);
    history.commit([{ type: 'brightness', amount: 0.2 }], 'brightness', 200);
    expect(history.lastCommitCreatedEntry).toBe(false);
    history.endCoalescing();
    history.commit([{ type: 'contrast', amount: 0.1 }]);
    history.commit([{ type: 'saturation', amount: 0.1 }]);
    expect(history.undoDepth).toBe(2);
    history.retainUndoDepth(1);
    expect(history.undoDepth).toBe(1);
  });

  it('force-coalesces a matching async geometry continuation beyond the normal time window', () => {
    const history = new EditHistory();
    history.commit([{ type: 'lens_correction', distortion: 0.1, vignetting: 0, chromatic_aberration: 0 }], 'lens_correction', 100);
    history.commit(
      [{ type: 'lens_correction', distortion: 0.6, vignetting: 0, chromatic_aberration: 0 }],
      'lens_correction',
      10_000,
      true
    );
    expect(history.lastCommitCreatedEntry).toBe(false);
    expect(history.undoDepth).toBe(1);
    expect(history.undo()).toEqual([]);
  });
});
