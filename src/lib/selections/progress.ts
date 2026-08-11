import type { MaskProgress, MaskProgressState } from './types';

export interface MaskProgressView {
  documentId: number;
  requestId: number;
  label: string;
  phase: string;
  percent: number | null;
  state: MaskProgressState;
  visible: boolean;
}

const DEFAULT_REVEAL_DELAY_MS = 180;

export class MaskProgressTracker {
  private documentId = 0;
  private requestId = 0;
  private label = '';
  private phase = '';
  private state: MaskProgressState = 'queued';
  private percent: number | null = null;
  private startedAt = 0;
  private active = false;

  constructor(private readonly revealDelayMs = DEFAULT_REVEAL_DELAY_MS) {}

  begin(documentId: number, requestId: number, label: string, now = Date.now()): void {
    this.documentId = documentId;
    this.requestId = requestId;
    this.label = label;
    this.phase = 'Queued';
    this.state = 'queued';
    this.percent = null;
    this.startedAt = now;
    this.active = true;
  }

  ingest(progress: MaskProgress, now = Date.now()): MaskProgressView | null {
    if (!this.matches(progress.documentId, progress.requestId)) return this.view(now);
    // Keep the user-facing label supplied at begin(); backend operation IDs are
    // stable machine identifiers such as `remap_geometry`, not display copy.
    if (!this.label) this.label = progress.operation;
    this.phase = progress.phase;
    this.state = progress.state;
    if (
      Number.isFinite(progress.completedUnits) &&
      Number.isFinite(progress.totalUnits) &&
      progress.completedUnits >= 0 &&
      progress.totalUnits > 0
    ) {
      const candidate = Math.round(
        Math.max(0, Math.min(1, progress.completedUnits / progress.totalUnits)) * 100
      );
      this.percent = Math.max(this.percent ?? 0, candidate);
    }
    const current = this.view(now);
    if (isTerminal(this.state)) this.reset();
    return current;
  }

  markCancelling(now = Date.now()): MaskProgressView | null {
    if (!this.active) return null;
    this.state = 'cancelling';
    this.phase = 'Cancelling';
    return this.view(now);
  }

  view(now = Date.now()): MaskProgressView | null {
    if (!this.active) return null;
    const terminal = isTerminal(this.state);
    return {
      documentId: this.documentId,
      requestId: this.requestId,
      label: this.label,
      phase: this.phase,
      percent: this.percent,
      state: this.state,
      visible: !terminal && now - this.startedAt >= this.revealDelayMs
    };
  }

  finish(documentId: number, requestId: number): void {
    if (this.matches(documentId, requestId)) this.reset();
  }

  reset(): void {
    this.active = false;
    this.percent = null;
    this.startedAt = 0;
  }

  private matches(documentId: number, requestId: number): boolean {
    return this.active && this.documentId === documentId && this.requestId === requestId;
  }
}

function isTerminal(state: MaskProgressState): boolean {
  return state === 'completed' || state === 'cancelled' || state === 'failed';
}
