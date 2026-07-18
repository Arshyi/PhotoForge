import type { EditOperation } from '../types/editor';
import { cloneOperations } from '../utils/operations';

export class EditHistory {
  private current: EditOperation[] = [];
  private undoStack: EditOperation[][] = [];
  private redoStack: EditOperation[][] = [];

  get operations(): EditOperation[] {
    return cloneOperations(this.current);
  }

  get canUndo(): boolean {
    return this.undoStack.length > 0;
  }

  get canRedo(): boolean {
    return this.redoStack.length > 0;
  }

  commit(operations: EditOperation[]): EditOperation[] {
    if (JSON.stringify(operations) === JSON.stringify(this.current)) return this.operations;
    this.undoStack.push(cloneOperations(this.current));
    this.current = cloneOperations(operations);
    this.redoStack = [];
    return this.operations;
  }

  undo(): EditOperation[] {
    const previous = this.undoStack.pop();
    if (!previous) return this.operations;
    this.redoStack.push(cloneOperations(this.current));
    this.current = previous;
    return this.operations;
  }

  redo(): EditOperation[] {
    const next = this.redoStack.pop();
    if (!next) return this.operations;
    this.undoStack.push(cloneOperations(this.current));
    this.current = next;
    return this.operations;
  }

  reset(): EditOperation[] {
    return this.commit([]);
  }

  clear(): void {
    this.current = [];
    this.undoStack = [];
    this.redoStack = [];
  }
}
