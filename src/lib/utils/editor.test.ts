import { describe, expect, it } from 'vitest';
import { EditHistory } from '../stores/history';
import { formatBytes } from './format';
import { presets, replaceOperation } from './operations';

describe('EditHistory', () => {
  it('supports commit, undo, redo, and reset', () => {
    const history = new EditHistory();
    history.commit([{ type: 'brightness', amount: 0.2 }]);
    history.commit([{ type: 'grayscale' }]);

    expect(history.undo()).toEqual([{ type: 'brightness', amount: 0.2 }]);
    expect(history.redo()).toEqual([{ type: 'grayscale' }]);
    expect(history.reset()).toEqual([]);
    expect(history.canUndo).toBe(true);
  });

  it('clears redo history after a new branch', () => {
    const history = new EditHistory();
    history.commit([{ type: 'sepia' }]);
    history.undo();
    history.commit([{ type: 'grayscale' }]);
    expect(history.canRedo).toBe(false);
  });

  it('coalesces rapid updates to the same slider into one undo step', () => {
    const history = new EditHistory();
    history.commit([{ type: 'brightness', amount: 0.1 }], 'brightness', 1_000);
    history.commit([{ type: 'brightness', amount: 0.2 }], 'brightness', 1_100);
    history.commit([{ type: 'brightness', amount: 0.3 }], 'brightness', 1_200);

    expect(history.undo()).toEqual([]);
    expect(history.canUndo).toBe(false);
  });

  it('keeps separate slider gestures as separate undo steps', () => {
    const history = new EditHistory();
    history.commit([{ type: 'brightness', amount: 0.1 }], 'brightness', 1_000);
    history.commit([{ type: 'brightness', amount: 0.2 }], 'brightness', 1_800);

    expect(history.undo()).toEqual([{ type: 'brightness', amount: 0.1 }]);
    expect(history.undo()).toEqual([]);
  });

  it('bounds retained history snapshots', () => {
    const history = new EditHistory();
    for (let index = 0; index < 205; index += 1) {
      history.commit([{ type: 'brightness', amount: index / 1_000 }]);
    }
    let undoCount = 0;
    while (history.canUndo) {
      history.undo();
      undoCount += 1;
    }
    expect(undoCount).toBe(200);
  });
});

describe('operation helpers', () => {
  it('replaces an operation without reordering the pipeline', () => {
    const operations = [
      { type: 'brightness', amount: 0.1 },
      { type: 'sepia' }
    ] as const;
    expect(replaceOperation([...operations], { type: 'brightness', amount: 0.2 })).toEqual([
      { type: 'brightness', amount: 0.2 },
      { type: 'sepia' }
    ]);
  });

  it('defines presets as ordinary typed pipelines', () => {
    expect(presets.length).toBeGreaterThanOrEqual(5);
    expect(presets.every((preset) => preset.operations.length > 0)).toBe(true);
  });
});

describe('formatBytes', () => {
  it('formats file sizes for the metadata panel', () => {
    expect(formatBytes(1_048_576)).toBe('1.00 MB');
  });
});
