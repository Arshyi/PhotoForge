import { describe, expect, it } from 'vitest';
import {
  eventDepths,
  historyEventStacks,
  historyEventForChanges,
  retainedHistorySuffix,
  selectionPanelHistoryAvailability,
  type HistoryEvent
} from './historyTimeline';

describe('history timeline retention', () => {
  it('retains a workflow only while every paired stack is available', () => {
    expect(eventDepths(['layer_compound', 'layer_edit', 'selection'])).toEqual({
      editDepth: 2, selectionDepth: 2, layerDepth: 2
    });
    expect(retainedHistorySuffix(['edit', 'layer_compound', 'layer'], 1, 0, 2).events).toEqual(['layer']);
    expect(historyEventStacks('layer_compound')).toEqual({ edit: true, selection: true, layer: true });
    expect(historyEventForChanges(true, false, true)).toBe('layer_edit');
    expect(historyEventForChanges(false, true, true)).toBe('layer_selection');
    expect(historyEventForChanges(false, false, false)).toBeNull();
  });
  it('keeps only the newest chronologically accessible mixed-event suffix', () => {
    const events: HistoryEvent[] = ['edit', 'selection', 'edit', 'geometry', 'selection'];
    expect(retainedHistorySuffix(events, 2, 2)).toEqual({
      events: ['edit', 'geometry', 'selection'],
      editDepth: 2,
      selectionDepth: 2,
      layerDepth: 0
    });
  });

  it('drops otherwise available edit snapshots before an evicted paired geometry event', () => {
    const events: HistoryEvent[] = ['edit', 'edit', 'geometry', 'selection', 'selection'];
    const retained = retainedHistorySuffix(events, 3, 2);
    expect(retained.events).toEqual(['selection', 'selection']);
    expect(eventDepths(retained.events)).toEqual({
      editDepth: 0,
      selectionDepth: 2,
      layerDepth: 0
    });
  });

  it('counts layer events on their own stack', () => {
    const events: HistoryEvent[] = ['layer', 'edit', 'layer', 'selection'];
    expect(eventDepths(events)).toEqual({ editDepth: 1, selectionDepth: 1, layerDepth: 2 });
  });

  it('evicts the oldest events when layer snapshots run short', () => {
    const events: HistoryEvent[] = ['layer', 'layer', 'edit', 'layer'];
    const retained = retainedHistorySuffix(events, 5, 5, 2);
    expect(retained.events).toEqual(['layer', 'edit', 'layer']);
    expect(retained.layerDepth).toBe(2);
    expect(retained.editDepth).toBe(1);
  });

  it('keeps a layer event that does not consume edit or selection snapshots', () => {
    const events: HistoryEvent[] = ['edit', 'layer'];
    const retained = retainedHistorySuffix(events, 1, 0, 1);
    expect(retained.events).toEqual(['edit', 'layer']);
    expect(retained).toEqual({
      events: ['edit', 'layer'],
      editDepth: 1,
      selectionDepth: 0,
      layerDepth: 1
    });
  });

  it('never offers selection-panel history for a layer event', () => {
    expect(selectionPanelHistoryAvailability(['layer'], ['layer'])).toEqual({
      canUndo: false,
      canRedo: false
    });
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
    expect(eventDepths(events)).toEqual({ editDepth: 2, selectionDepth: 3, layerDepth: 0 });
    expect(retainedHistorySuffix(events, 1, 2)).toEqual({
      events: ['compound', 'selection'],
      editDepth: 1,
      selectionDepth: 2,
      layerDepth: 0
    });
  });
});
