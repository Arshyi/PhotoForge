import { invoke } from '@tauri-apps/api/core';
import type { LayerDocument } from '../layers/types';
import type { OperationSpec, StepReport, TransactionRequest, TransactionResult } from './types';

/**
 * Runs a list of operations as one transaction in the backend: all of them, or
 * none, with one validated document coming back.
 */
export function applyTransaction(request: TransactionRequest): Promise<TransactionResult> {
  return invoke<TransactionResult>('apply_transaction', { request });
}

/** Every operation that may edit a document. */
export function listOperations(): Promise<OperationSpec[]> {
  return invoke<OperationSpec[]>('list_operations');
}

/** The revision of a document, to name in a plan made against it. */
export function layerDocumentRevision(document: LayerDocument): Promise<string> {
  return invoke<string>('layer_document_revision', { document });
}

/**
 * What a transaction would do, without doing it: every check, and a dry run of every
 * step saying which conditions hold. Produces nothing and changes nothing.
 */
export function planTransaction(request: TransactionRequest): Promise<StepReport[]> {
  return invoke<StepReport[]>('plan_transaction', { request });
}

/** The text of an exported macro file. Nothing in it is run; the caller checks it. */
export function importMacroFile(path: string): Promise<string> {
  return invoke<string>('import_macro', { path });
}

/** Writes a macro document to an absolute `.json` path, returning where it went. */
export function exportMacroFile(path: string, text: string): Promise<string> {
  return invoke<string>('export_macro', { path, text });
}
