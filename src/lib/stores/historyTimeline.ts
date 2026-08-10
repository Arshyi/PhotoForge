export type HistoryEvent = 'edit' | 'selection' | 'geometry';

export interface RetainedHistoryTimeline {
  events: HistoryEvent[];
  editDepth: number;
  selectionDepth: number;
}

export function retainedHistorySuffix(
  events: HistoryEvent[],
  availableEditDepth: number,
  availableSelectionDepth: number
): RetainedHistoryTimeline {
  let editDepth = 0;
  let selectionDepth = 0;
  let start = events.length;
  for (let index = events.length - 1; index >= 0; index -= 1) {
    const event = events[index];
    const nextEditDepth = editDepth + (event === 'edit' || event === 'geometry' ? 1 : 0);
    const nextSelectionDepth = selectionDepth + (event === 'selection' || event === 'geometry' ? 1 : 0);
    if (nextEditDepth > availableEditDepth || nextSelectionDepth > availableSelectionDepth) break;
    editDepth = nextEditDepth;
    selectionDepth = nextSelectionDepth;
    start = index;
  }
  return { events: events.slice(start), editDepth, selectionDepth };
}

export function selectionPanelHistoryAvailability(
  historyEvents: HistoryEvent[],
  redoEvents: HistoryEvent[]
): { canUndo: boolean; canRedo: boolean } {
  return {
    canUndo: historyEvents.at(-1) === 'selection',
    canRedo: redoEvents.at(-1) === 'selection'
  };
}

export function eventDepths(events: HistoryEvent[]): { editDepth: number; selectionDepth: number } {
  let editDepth = 0;
  let selectionDepth = 0;
  for (const event of events) {
    if (event === 'edit' || event === 'geometry') editDepth += 1;
    if (event === 'selection' || event === 'geometry') selectionDepth += 1;
  }
  return { editDepth, selectionDepth };
}
