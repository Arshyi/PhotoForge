import type { EditOperation } from '../types/editor';
import { cloneOperations } from '../utils/operations';

const MAX_HISTORY_ENTRIES = 200;

export class EditHistory {
  private current: EditOperation[] = [];
  private undoStack: EditOperation[][] = [];
  private redoStack: EditOperation[][] = [];
  private coalesceKey: string | null = null;
  private coalesceAt = 0;
  private pushedOnLastCommit = false;

  constructor(private readonly maxEntries = MAX_HISTORY_ENTRIES) {}

  get operations(): EditOperation[] {
    return cloneOperations(this.current);
  }

  get canUndo(): boolean {
    return this.undoStack.length > 0;
  }

  get canRedo(): boolean {
    return this.redoStack.length > 0;
  }

  get undoDepth(): number {
    return this.undoStack.length;
  }

  get redoDepth(): number {
    return this.redoStack.length;
  }

  get lastCommitCreatedEntry(): boolean {
    return this.pushedOnLastCommit;
  }

  replace(operations: EditOperation[]): EditOperation[] {
    this.current = cloneOperations(operations);
    this.undoStack = [];
    this.redoStack = [];
    this.pushedOnLastCommit = false;
    this.endCoalescing();
    return this.operations;
  }

  commit(operations: EditOperation[], coalesceKey?: string, now = Date.now()): EditOperation[] {
    this.pushedOnLastCommit = false;
    if (JSON.stringify(operations) === JSON.stringify(this.current)) return this.operations;
    const canCoalesce =
      coalesceKey !== undefined && this.coalesceKey === coalesceKey && now - this.coalesceAt <= 500;
    if (!canCoalesce) {
      this.undoStack.push(cloneOperations(this.current));
      this.pushedOnLastCommit = true;
      if (this.undoStack.length > this.maxEntries) this.undoStack.shift();
    }
    this.current = cloneOperations(operations);
    this.redoStack = [];
    this.coalesceKey = coalesceKey ?? null;
    this.coalesceAt = now;
    return this.operations;
  }

  undo(): EditOperation[] {
    const previous = this.undoStack.pop();
    if (!previous) return this.operations;
    this.redoStack.push(cloneOperations(this.current));
    this.current = previous;
    this.endCoalescing();
    return this.operations;
  }

  redo(): EditOperation[] {
    const next = this.redoStack.pop();
    if (!next) return this.operations;
    this.undoStack.push(cloneOperations(this.current));
    this.current = next;
    this.endCoalescing();
    return this.operations;
  }

  reset(): EditOperation[] {
    this.endCoalescing();
    return this.commit([]);
  }

  clear(): void {
    this.current = [];
    this.undoStack = [];
    this.redoStack = [];
    this.pushedOnLastCommit = false;
    this.endCoalescing();
  }

  endCoalescing(): void {
    this.coalesceKey = null;
    this.coalesceAt = 0;
  }

  clearRedo(): void {
    this.redoStack = [];
  }

  retainUndoDepth(depth: number): void {
    const bounded = Math.max(0, Math.min(this.undoStack.length, Math.floor(depth)));
    if (this.undoStack.length > bounded) this.undoStack.splice(0, this.undoStack.length - bounded);
  }

  retainRedoDepth(depth: number): void {
    const bounded = Math.max(0, Math.min(this.redoStack.length, Math.floor(depth)));
    if (this.redoStack.length > bounded) this.redoStack.splice(0, this.redoStack.length - bounded);
  }
}
