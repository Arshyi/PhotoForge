import { describe, expect, it } from 'vitest';
import { EditHistory } from '../stores/history';
import { errorMessage, formatBytes } from './format';
import { presets, replaceOperation } from './operations';
import { analysisObservations } from './analysis';

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
    expect(presets.length).toBeGreaterThanOrEqual(13);
    expect(presets.every((preset) => preset.operations.length > 0)).toBe(true);
  });

  it('defines every Phase 2 preset with explicit typed restoration operations', () => {
    const phaseTwoIds = [
      'indoor-lighting',
      'old-scan',
      'jpeg-cleanup',
      'mild-detail',
      'document-color',
      'document-grayscale',
      'uneven-lighting',
      'conservative-restore'
    ];
    for (const id of phaseTwoIds) {
      const preset = presets.find((candidate) => candidate.id === id);
      expect(preset?.operations.length).toBeGreaterThan(0);
      expect(preset?.operations.every((operation) => typeof operation.type === 'string')).toBe(true);
    }
  });

  it('coalesces restoration strength gestures and preserves advanced changes', () => {
    const history = new EditHistory();
    history.commit([{ type: 'denoise', strength: 0.2, preserve_edges: 0.8 }], 'denoise', 1_000);
    history.commit([{ type: 'denoise', strength: 0.4, preserve_edges: 0.8 }], 'denoise', 1_100);
    history.commit(
      [{ type: 'denoise', strength: 0.4, preserve_edges: 0.95 }],
      'denoise:preserve_edges',
      1_200
    );
    expect(history.undo()).toEqual([{ type: 'denoise', strength: 0.4, preserve_edges: 0.8 }]);
    expect(history.undo()).toEqual([]);
  });
});

describe('analysis observations', () => {
  it('uses cautious language and returns a bounded list', () => {
    const observations = analysisObservations({
      averageLuminance: 0.2,
      luminanceSpread: 0.5,
      estimatedColorCast: { dominant: 'warm', redBias: 0.1, greenBias: 0, blueBias: -0.1 },
      estimatedNoise: 0.4,
      estimatedSharpness: 0.01,
      estimatedLocalContrast: 0.01,
      edgeDensity: 0.1,
      whiteBackgroundRatio: 0.6,
      likelyDocument: true
    });
    expect(observations.length).toBeLessThanOrEqual(4);
    expect(observations.join(' ')).toContain('appears');
    expect(observations.join(' ')).not.toContain('definitely');
  });
});

describe('formatBytes', () => {
  it('formats file sizes for the metadata panel', () => {
    expect(formatBytes(1_048_576)).toBe('1.00 MB');
  });

  it('renders typed user-facing failures without internal fallback details', () => {
    expect(errorMessage({ code: 'analysis_failure', message: 'Try reopening this image.' })).toBe(
      'Try reopening this image.'
    );
    expect(errorMessage(new Error())).toBe('PhotoForge could not complete that action.');
    expect(errorMessage(null)).toBe('PhotoForge could not complete that action.');
  });
});
