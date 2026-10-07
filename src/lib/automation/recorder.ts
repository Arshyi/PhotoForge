/**
 * Recording: turning what a person does in the Layers panel into macro steps.
 *
 * The recorder is deliberately narrow. It records the panel actions that *are* a
 * registered operation — show or hide, opacity, blend mode, rename, move, group,
 * ungroup, duplicate, delete, merge down, flatten, add a group or pixel layer — and
 * nothing else. An action with no registered operation (editing text, placing an
 * image, developing RAW, painting) is not guessed at: it is counted and reported, so
 * a recording that left something out says so instead of replaying a different
 * document than the one that was made.
 *
 * Layers are named in the way most likely to mean the same thing later. A layer with
 * a name no other layer shares is recorded by that name, which replays on a document
 * that has such a layer. A layer whose name is shared, or empty, can only be named by
 * its identifier, which replays on this document only, and the recorder says so.
 */
import type { LayerDocument } from '../layers/types';
import { eachLayer } from '../layers/tree';
import type { LayerSelector } from '../operations/types';
import { MAX_MACRO_STEPS, newStep, type MacroStep } from './macros';

export interface RecordedNote {
  /** `skipped`: an action that has no registered operation. `identifier`: names one layer of one document. */
  kind: 'skipped' | 'identifier' | 'full';
  message: string;
}

export interface RecorderState {
  recording: boolean;
  steps: MacroStep[];
  notes: RecordedNote[];
}

/** How a layer is named in a recording, and whether that name is portable. */
export function selectorFor(document: LayerDocument, layerId: string): { selector: LayerSelector; portable: boolean } {
  const layers = eachLayer(document);
  const name = layers.find((layer) => layer.id === layerId)?.name ?? '';
  const matches = layers.filter((layer) => layer.name === name).length;
  if (name.trim() && name.length <= 120 && matches === 1) return { selector: { type: 'name', name }, portable: true };
  return { selector: { type: 'id', id: layerId }, portable: false };
}

/** A continuous gesture (an opacity drag) is one step, not one per frame. */
const COALESCING = new Set(['core.layer.set_opacity', 'core.layer.move']);

export class Recorder {
  private state: RecorderState = { recording: false, steps: [], notes: [] };
  private readonly listeners = new Set<(state: RecorderState) => void>();
  private lastKey: string | null = null;

  get snapshot(): RecorderState {
    return this.state;
  }

  subscribe(listener: (state: RecorderState) => void): () => void {
    this.listeners.add(listener);
    listener(this.state);
    return () => this.listeners.delete(listener);
  }

  private set(next: Partial<RecorderState>) {
    this.state = { ...this.state, ...next };
    for (const listener of this.listeners) listener(this.state);
  }

  start() {
    this.lastKey = null;
    this.set({ recording: true, steps: [], notes: [] });
  }

  /** Stops, returning what was recorded. */
  stop(): { steps: MacroStep[]; notes: RecordedNote[] } {
    this.lastKey = null;
    const result = { steps: this.state.steps, notes: this.state.notes };
    this.set({ recording: false });
    return result;
  }

  discard() {
    this.lastKey = null;
    this.set({ recording: false, steps: [], notes: [] });
  }

  private note(note: RecordedNote) {
    if (this.state.notes.some((existing) => existing.message === note.message)) return;
    this.set({ notes: [...this.state.notes, note] });
  }

  /**
   * Records one registered operation. `layer` names the layer it acted on, in the
   * document as it was *before* the action; `gestureKey` marks a continuous gesture.
   */
  record(op: string, params: Record<string, unknown>, options: { gestureKey?: string } = {}) {
    if (!this.state.recording) return;
    const key = options.gestureKey && COALESCING.has(op) ? `${op}:${options.gestureKey}` : null;
    const last = this.state.steps[this.state.steps.length - 1];
    if (key && key === this.lastKey && last) {
      this.set({ steps: [...this.state.steps.slice(0, -1), { ...last, params: structuredClone(params) }] });
      return;
    }
    if (this.state.steps.length >= MAX_MACRO_STEPS) {
      this.note({ kind: 'full', message: `A macro holds at most ${MAX_MACRO_STEPS} steps; later actions were not recorded.` });
      return;
    }
    this.lastKey = key;
    this.set({ steps: [...this.state.steps, newStep(op, params)] });
  }

  /** Records an operation on one layer, naming the layer as portably as it can be named. */
  recordOnLayer(document: LayerDocument, layerId: string, op: string, params: Record<string, unknown>, options: { gestureKey?: string } = {}) {
    if (!this.state.recording) return;
    const { selector, portable } = selectorFor(document, layerId);
    if (!portable) {
      this.note({
        kind: 'identifier',
        message: 'A layer whose name is shared or empty was recorded by its identifier, so those steps replay on this document only. Give it a unique name first to make them portable.'
      });
    }
    this.record(op, { selector, ...params }, options);
  }

  /** Records an operation on several layers, each named as portably as it can be. */
  recordOnLayers(document: LayerDocument, layerIds: string[], op: string, params: Record<string, unknown>) {
    if (!this.state.recording) return;
    const named = layerIds.map((layerId) => selectorFor(document, layerId));
    if (named.some((entry) => !entry.portable)) {
      this.note({
        kind: 'identifier',
        message: 'A layer whose name is shared or empty was recorded by its identifier, so those steps replay on this document only. Give it a unique name first to make them portable.'
      });
    }
    this.record(op, { selectors: named.map((entry) => entry.selector), ...params });
  }

  /** An action the panel made that has no registered operation, so was not recorded. */
  skip(what: string) {
    if (!this.state.recording) return;
    this.lastKey = null;
    this.note({ kind: 'skipped', message: `${what} cannot be recorded, so it is not in this macro.` });
  }
}

export const recorder = new Recorder();
