import { invoke } from '@tauri-apps/api/core';
import type { LayerDocument } from '../layers/types';
import type { OperationSpec, TransactionRequest, TransactionResult } from './types';

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
