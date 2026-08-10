import { describe, expect, it } from 'vitest';
import { MaskProgressTracker } from './progress';
import type { MaskProgress } from './types';

function progress(overrides: Partial<MaskProgress> = {}): MaskProgress {
  return {
    documentId: 2,
    requestId: 7,
    operation: 'Feather selection',
    phase: 'Horizontal pass',
    completedUnits: 25,
    totalUnits: 100,
    state: 'running',
    ...overrides
  };
}

describe('MaskProgressTracker', () => {
  it('delays short operations to avoid a flashing indicator', () => {
    const tracker = new MaskProgressTracker(180);
    tracker.begin(2, 7, 'Remap masks through geometry', 1_000);
    const early = tracker.ingest(progress({ operation: 'remap_geometry' }), 1_100);
    expect(early?.visible).toBe(false);
    expect(early?.label).toBe('Remap masks through geometry');
    expect(tracker.view(1_180)?.visible).toBe(true);
  });

  it('keeps progress monotonic and clamps it to a valid percentage', () => {
    const tracker = new MaskProgressTracker(0);
    tracker.begin(2, 7, 'Feather selection', 0);
    expect(tracker.ingest(progress({ completedUnits: 70 }), 1)?.percent).toBe(70);
    expect(tracker.ingest(progress({ completedUnits: 20 }), 2)?.percent).toBe(70);
    expect(tracker.ingest(progress({ completedUnits: 200 }), 3)?.percent).toBe(100);
  });

  it('ignores stale document and request updates', () => {
    const tracker = new MaskProgressTracker(0);
    tracker.begin(2, 7, 'Current', 0);
    tracker.ingest(progress({ completedUnits: 40 }), 1);
    expect(tracker.ingest(progress({ requestId: 6, completedUnits: 90 }), 2)?.percent).toBe(40);
    expect(tracker.ingest(progress({ documentId: 3, completedUnits: 90 }), 3)?.percent).toBe(40);
  });

  it('uses an indeterminate display when no honest total exists', () => {
    const tracker = new MaskProgressTracker(0);
    tracker.begin(2, 7, 'Import mask', 0);
    expect(tracker.ingest(progress({ totalUnits: 0 }), 1)?.percent).toBeNull();
  });

  it('shows cancelling until acknowledgement and clears every terminal path', () => {
    const tracker = new MaskProgressTracker(0);
    tracker.begin(2, 7, 'Refine selection', 0);
    expect(tracker.markCancelling(1)?.state).toBe('cancelling');
    tracker.finish(2, 6);
    expect(tracker.view(2)).not.toBeNull();
    tracker.finish(2, 7);
    expect(tracker.view(3)).toBeNull();
  });
});
