import { referencedPixelIds } from './tree';
import type { LayerDocument } from './types';

const MAX_HISTORY_ENTRIES = 200;
/** How long a repeated coalescing gesture keeps folding into one undo step. */
const COALESCE_WINDOW_MS = 500;

export interface LayerHistoryEntry {
  document: LayerDocument;
  label: string;
}

/**
 * Undo history for the layer tree.
 *
 * Entries store whole document trees rather than diffs, which is affordable
 * because the tree carries no pixels: layers reference immutable buffers by
 * identifier and untouched subtrees are shared by reference between entries.
 * A slider drag or a drag-and-drop reorder coalesces into a single logical undo
 * step through `coalesceKey`.
 */
export class LayerHistory {
  private current: LayerDocument | null = null;
  private undoStack: LayerHistoryEntry[] = [];
  private redoStack: LayerHistoryEntry[] = [];
  private label = '';
  private coalesceKey: string | null = null;
  private coalesceAt = 0;

  constructor(private readonly maxEntries = MAX_HISTORY_ENTRIES) {}

  get document(): LayerDocument | null {
    return this.current;
  }

  get canUndo(): boolean {
    return this.undoStack.length > 0;
  }

  get canRedo(): boolean {
    return this.redoStack.length > 0;
  }

  get undoLabel(): string {
    return this.undoStack.at(-1)?.label ?? '';
  }

  get redoLabel(): string {
    return this.redoStack.at(-1)?.label ?? '';
  }

  /** The label of the change that produced the current state. */
  get currentLabel(): string {
    return this.label;
  }

  get undoDepth(): number {
    return this.undoStack.length;
  }

  get redoDepth(): number {
    return this.redoStack.length;
  }

  /** Replaces the document and clears history, as when opening a file. */
  replace(document: LayerDocument, label = 'Open'): LayerDocument {
    this.current = document;
    this.label = label;
    this.undoStack = [];
    this.redoStack = [];
    this.endCoalescing();
    return document;
  }

  /**
   * Swaps the current document without touching the undo stacks.
   *
   * Used for changes that are not edits — selecting a different layer, for
   * example — so browsing the stack never fills history with noise.
   */
  replaceCurrent(document: LayerDocument): LayerDocument {
    this.current = document;
    return document;
  }

  commit(
    document: LayerDocument,
    label: string,
    coalesceKey?: string,
    now = Date.now()
  ): LayerDocument {
    if (this.current === document) return document;
    if (this.current === null) return this.replace(document, label);

    const canCoalesce =
      coalesceKey !== undefined &&
      this.coalesceKey === coalesceKey &&
      now - this.coalesceAt <= COALESCE_WINDOW_MS;

    if (!canCoalesce) {
      this.undoStack.push({ document: this.current, label: this.label });
      if (this.undoStack.length > this.maxEntries) this.undoStack.shift();
    }
    this.current = document;
    this.label = label;
    this.redoStack = [];
    this.coalesceKey = coalesceKey ?? null;
    this.coalesceAt = now;
    return document;
  }

  undo(): LayerDocument | null {
    const previous = this.undoStack.pop();
    if (!previous || !this.current) return this.current;
    this.redoStack.push({ document: this.current, label: this.label });
    this.current = previous.document;
    this.label = previous.label;
    this.endCoalescing();
    return this.current;
  }

  redo(): LayerDocument | null {
    const next = this.redoStack.pop();
    if (!next || !this.current) return this.current;
    this.undoStack.push({ document: this.current, label: this.label });
    this.current = next.document;
    this.label = next.label;
    this.endCoalescing();
    return this.current;
  }

  /** Ends the coalescing window, so the next change starts a new undo step. */
  endCoalescing(): void {
    this.coalesceKey = null;
    this.coalesceAt = 0;
  }

  clearRedo(): void {
    this.redoStack = [];
  }

  /**
   * Trims the undo stack so it never claims more steps than the shared history
   * timeline still tracks, matching how the edit and selection stacks are kept
   * in step with one another.
   */
  retainUndoDepth(depth: number): void {
    const bounded = Math.max(0, Math.min(this.undoStack.length, Math.floor(depth)));
    if (this.undoStack.length > bounded) {
      this.undoStack.splice(0, this.undoStack.length - bounded);
    }
  }

  retainRedoDepth(depth: number): void {
    const bounded = Math.max(0, Math.min(this.redoStack.length, Math.floor(depth)));
    if (this.redoStack.length > bounded) {
      this.redoStack.splice(0, this.redoStack.length - bounded);
    }
  }

  clear(): void {
    this.current = null;
    this.label = '';
    this.undoStack = [];
    this.redoStack = [];
    this.endCoalescing();
  }

  /**
   * Every pixel buffer still reachable from the current document or anything in
   * the undo and redo stacks. The session store keeps exactly these buffers, so
   * an undone merge can still be redone while genuinely orphaned pixels are
   * released.
   */
  reachablePixelIds(): string[] {
    const ids = new Set<string>();
    const documents = [
      ...(this.current ? [this.current] : []),
      ...this.undoStack.map((entry) => entry.document),
      ...this.redoStack.map((entry) => entry.document)
    ];
    for (const document of documents) {
      for (const id of referencedPixelIds(document)) ids.add(id);
    }
    return [...ids].sort();
  }
}
