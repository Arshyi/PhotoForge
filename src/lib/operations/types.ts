/**
 * Types for the operation registry and its transactions.
 *
 * They mirror `operations::{model, registry}` in the backend. The backend is the
 * only place that knows what an operation does and whether a list of them is
 * valid; nothing here re-derives either.
 */
import type { LayerDocument } from '../layers/types';
import type { MaskSnapshot } from '../selections/types';

/** Who is asking. It decides what is allowed, never how it is done. */
export type Origin = 'user' | 'automation' | 'planner' | 'plugin' | 'batch';

/** One call: an operation's registered id and its parameters. */
export interface OperationCall {
  op: string;
  params?: Record<string, unknown>;
}

export interface StepReport {
  index: number;
  op: string;
  createdLayers: string[];
  createdPixels: string[];
}

export interface TransactionRequest {
  document: LayerDocument;
  /** The name of the one undo entry a successful transaction becomes. */
  label: string;
  steps: OperationCall[];
  /** If given and not the document's revision, nothing runs. */
  expectedRevision?: string | null;
  selection?: MaskSnapshot | null;
  origin?: Origin;
}

export interface TransactionResult {
  document: LayerDocument;
  revision: string;
  label: string;
  steps: StepReport[];
  /** Every pixel buffer the transaction registered. */
  createdPixelIds: string[];
  lastCreated: string | null;
}

export type ParamKind =
  | { kind: 'selector' }
  | { kind: 'selectors' }
  | { kind: 'optionalSelector' }
  | { kind: 'text'; maxChars: number }
  | { kind: 'bool' }
  | { kind: 'number'; min: number; max: number }
  | { kind: 'integer'; min: number; max: number }
  | { kind: 'blendMode' }
  | { kind: 'editOperation' }
  | { kind: 'editOperations' }
  | { kind: 'json' };

export interface ParamSpec {
  name: string;
  kind: ParamKind;
  required: boolean;
  description: string;
}

export type OperationCategory = 'layer' | 'adjustment' | 'pixels' | 'document' | 'plugin';

export interface OperationSpec {
  id: string;
  version: number;
  title: string;
  summary: string;
  category: OperationCategory;
  effects: 'none' | 'pixels';
  lock: 'ignored' | 'automationOnly' | 'always';
  plannerSafe: boolean;
  needsSelection: boolean;
  params: ParamSpec[];
}
