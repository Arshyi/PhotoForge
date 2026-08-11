import { describe, expect, it } from 'vitest';
import {
  eventDepths,
  retainedHistorySuffix,
  selectionPanelHistoryAvailability,
  type HistoryEvent
} from './historyTimeline';

describe('history timeline retention', () => {
  it('keeps only the newest chronologically accessible mixed-event suffix', () => {
    const events: HistoryEvent[] = ['edit', 'selection', 'edit', 'geometry', 'selection'];
    expect(retainedHistorySuffix(events, 2, 2)).toEqual({
      events: ['edit', 'geometry', 'selection'],
      editDepth: 2,
      selectionDepth: 2
    });
  });

  it('drops otherwise available edit snapshots before an evicted paired geometry event', () => {
    const events: HistoryEvent[] = ['edit', 'edit', 'geometry', 'selection', 'selection'];
    const retained = retainedHistorySuffix(events, 3, 2);
    expect(retained.events).toEqual(['selection', 'selection']);
    expect(eventDepths(retained.events)).toEqual({ editDepth: 0, selectionDepth: 2 });
  });

  it('allows selection-panel history only for a top selection event', () => {
    expect(selectionPanelHistoryAvailability(['selection', 'geometry'], [])).toEqual({
      canUndo: false,
      canRedo: false
    });
    expect(selectionPanelHistoryAvailability(['geometry', 'selection'], ['edit', 'selection'])).toEqual({
      canUndo: true,
      canRedo: true
    });
    expect(selectionPanelHistoryAvailability([], ['selection', 'geometry'])).toEqual({
      canUndo: false,
      canRedo: false
    });
    expect(selectionPanelHistoryAvailability(['selection', 'compound'], ['compound'])).toEqual({
      canUndo: false,
      canRedo: false
    });
  });

  it('retains compound edit-and-selection events as paired history entries', () => {
    const events: HistoryEvent[] = ['edit', 'selection', 'compound', 'selection'];
    expect(eventDepths(events)).toEqual({ editDepth: 2, selectionDepth: 3 });
    expect(retainedHistorySuffix(events, 1, 2)).toEqual({
      events: ['compound', 'selection'],
      editDepth: 1,
      selectionDepth: 2
    });
  });
});
