import type { EditOperation } from '../types/editor';
import { cloneOperations } from '../utils/operations';

const MAX_HISTORY_ENTRIES = 200;

export class EditHistory {
  private current: EditOperation[] = [];
  private undoStack: EditOperation[][] = [];
  private redoStack: EditOperation[][] = [];
  private coalesceKey: string | null = null;
  private coalesceAt = 0;

  get operations(): EditOperation[] {
    return cloneOperations(this.current);
  }

  get canUndo(): boolean {
    return this.undoStack.length > 0;
  }

  get canRedo(): boolean {
    return this.redoStack.length > 0;
  }

  commit(operations: EditOperation[], coalesceKey?: string, now = Date.now()): EditOperation[] {
    if (JSON.stringify(operations) === JSON.stringify(this.current)) return this.operations;
    const canCoalesce =
      coalesceKey !== undefined && this.coalesceKey === coalesceKey && now - this.coalesceAt <= 500;
    if (!canCoalesce) {
      this.undoStack.push(cloneOperations(this.current));
      if (this.undoStack.length > MAX_HISTORY_ENTRIES) this.undoStack.shift();
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
    this.endCoalescing();
  }

  endCoalescing(): void {
    this.coalesceKey = null;
    this.coalesceAt = 0;
  }
}
