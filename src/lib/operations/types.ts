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

/** A layer a step or a condition names, as the backend resolves it. */
export type LayerSelector =
  | { type: 'id'; id: string }
  | { type: 'active' }
  | { type: 'last_created' }
  | { type: 'name'; name: string }
  | { type: 'bottom' }
  | { type: 'top' };

/**
 * The only decision an automation can make: a question about the document as it is
 * when a step is reached, answered yes or no. A step whose condition does not hold is
 * skipped. There is nothing else: no else, no loop, no variable, no expression.
 */
export type Condition =
  | { kind: 'layer_exists'; selector: LayerSelector }
  | { kind: 'layer_count_at_least'; count: number }
  | { kind: 'active_layer_kind'; layerKind: 'pixel' | 'group' | 'adjustment' | 'shape' | 'text' | 'smart_object' }
  | { kind: 'precision'; precision: 'linear_srgb_f32' | 'legacy_srgb8' }
  | { kind: 'not'; condition: Condition };

/** One call: an operation's registered id and its parameters. */
export interface OperationCall {
  op: string;
  params?: Record<string, unknown>;
  /** The step is skipped unless this holds when the step is reached. */
  when?: Condition | null;
}

export interface StepReport {
  index: number;
  op: string;
  /** Its condition did not hold, so it was not run. */
  skipped: boolean;
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
