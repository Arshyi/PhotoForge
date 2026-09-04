export type HistoryEvent = 'edit' | 'selection' | 'geometry' | 'compound' | 'layer' |
  'layer_edit' | 'layer_selection' | 'layer_compound';

function includesEdit(event: HistoryEvent): boolean {
  return event === 'edit' || event === 'geometry' || event === 'compound' ||
    event === 'layer_edit' || event === 'layer_compound';
}

function includesSelection(event: HistoryEvent): boolean {
  return event === 'selection' || event === 'geometry' || event === 'compound' ||
    event === 'layer_selection' || event === 'layer_compound';
}

/**
 * Layer-tree changes have their own undo stack. A workflow can pair them with
 * document operations and remapped selections as a single atomic undo step.
 */
function includesLayer(event: HistoryEvent): boolean {
  return event === 'layer' || event.startsWith('layer_');
}

export function historyEventStacks(event: HistoryEvent) {
  return { edit: includesEdit(event), selection: includesSelection(event), layer: includesLayer(event) };
}

export function historyEventForChanges(edit: boolean, selection: boolean, layer: boolean): HistoryEvent | null {
  if (layer) return edit ? (selection ? 'layer_compound' : 'layer_edit') :
    (selection ? 'layer_selection' : 'layer');
  return edit ? (selection ? 'compound' : 'edit') : selection ? 'selection' : null;
}

export interface RetainedHistoryTimeline {
  events: HistoryEvent[];
  editDepth: number;
  selectionDepth: number;
  layerDepth: number;
}

export function retainedHistorySuffix(
  events: HistoryEvent[],
  availableEditDepth: number,
  availableSelectionDepth: number,
  availableLayerDepth = Number.POSITIVE_INFINITY
): RetainedHistoryTimeline {
  let editDepth = 0;
  let selectionDepth = 0;
  let layerDepth = 0;
  let start = events.length;
  for (let index = events.length - 1; index >= 0; index -= 1) {
    const event = events[index];
    const nextEditDepth = editDepth + (includesEdit(event) ? 1 : 0);
    const nextSelectionDepth = selectionDepth + (includesSelection(event) ? 1 : 0);
    const nextLayerDepth = layerDepth + (includesLayer(event) ? 1 : 0);
    if (
      nextEditDepth > availableEditDepth ||
      nextSelectionDepth > availableSelectionDepth ||
      nextLayerDepth > availableLayerDepth
    ) {
      break;
    }
    editDepth = nextEditDepth;
    selectionDepth = nextSelectionDepth;
    layerDepth = nextLayerDepth;
    start = index;
  }
  return { events: events.slice(start), editDepth, selectionDepth, layerDepth };
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

export function eventDepths(events: HistoryEvent[]): {
  editDepth: number;
  selectionDepth: number;
  layerDepth: number;
} {
  let editDepth = 0;
  let selectionDepth = 0;
  let layerDepth = 0;
  for (const event of events) {
    if (includesEdit(event)) editDepth += 1;
    if (includesSelection(event)) selectionDepth += 1;
    if (includesLayer(event)) layerDepth += 1;
  }
  return { editDepth, selectionDepth, layerDepth };
}
