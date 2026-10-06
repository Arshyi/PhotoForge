/**
 * Types for opening a source that may be too large to open whole.
 *
 * These mirror `resources::admission` and `source::model` in the backend. The
 * backend is the only place that prices a decode; nothing here re-derives a
 * memory figure from a formula of its own. Where the interface needs to
 * recompute a cost as a rectangle is dragged, it evaluates coefficients the
 * backend supplied (`RegionCost`) rather than reimplementing the model.
 */

export type SourceKind = 'png' | 'jpeg' | 'webP' | 'dng';

export type RegionDecode =
  | { kind: 'rows' }
  | { kind: 'segments'; fixedBytes: number; bytesPerPixel: number }
  | { kind: 'transientFull'; bytesPerPixel: number }
  | { kind: 'none' };

export type ReducedDecode =
  | { kind: 'rows' }
  | { kind: 'transientFull'; bytesPerPixel: number }
  | { kind: 'dctScaled' }
  | { kind: 'none' };

/** Whether a verdict leans on the machine right now or on PhotoForge's budget. */
export type Shortfall = 'momentary' | 'budget';

export type Refusal =
  | { kind: 'empty' }
  | { kind: 'implausibleDimensions'; width: number; height: number }
  | { kind: 'impossibleExpansion'; claimedBytes: number; fileBytes: number };

export type Verdict =
  | { kind: 'fullResolution' }
  | { kind: 'fullResolutionWithWarning'; peakPercentOfBudget: number }
  | { kind: 'regionRequired' }
  | { kind: 'reducedCopyRecommended' }
  | { kind: 'insufficientResources'; shortfall: Shortfall }
  | { kind: 'unsafe'; refusal: Refusal };

export type AdmissionOption =
  | { kind: 'openFull'; peakBytes: number }
  | { kind: 'openRegion'; maxRegionPixels: number; decode: RegionDecode; peakBytesAtMax: number }
  | { kind: 'openReduced'; maxScale: number; decode: ReducedDecode };

/**
 * Coefficients of the region peak, so the interface can price a rectangle as it
 * is dragged. Peak is the larger of two affine functions of the region's pixel
 * count: building the working copy, and editing it.
 */
export interface RegionCost {
  openingFixed: number;
  openingPerPixel: number;
  editingFixed: number;
  editingPerPixel: number;
}

export interface AdmissionReport {
  verdict: Verdict;
  options: AdmissionOption[];
  fullPeakBytes: number;
  budgetBytes: number;
  availableBytes: number | null;
  regionCost: RegionCost | null;
  outOfCore: string;
}

/** What `probe_image_source` returns: the header, and what to do about it. */
export interface SourceAdmission {
  path: string;
  filename: string;
  kind: SourceKind;
  width: number;
  height: number;
  fileBytes: number;
  hasIcc: boolean;
  bitDepth: number;
  report: AdmissionReport;
}

export interface Rect {
  x: number;
  y: number;
  width: number;
  height: number;
}

export type OpenSelection =
  | { kind: 'region'; rect: Rect }
  | { kind: 'reduced'; width: number; height: number };

export interface SourceIdentity {
  path: string;
  sha256: string;
  bytes: number;
  kind: SourceKind;
  width: number;
  height: number;
}

export type SourceView =
  | { view: 'region'; rect: Rect }
  | { view: 'reduced'; width: number; height: number };

/** Provenance of a layer whose pixels are only part of a larger file. */
export interface SourceOrigin {
  source: SourceIdentity;
  view: SourceView;
}

export type OriginState = 'available' | 'missing' | 'changed';

export interface SourcePreview {
  dataUrl: string;
  width: number;
  height: number;
}

/** The kinds of verdict that mean the user has to choose before anything opens. */
export function needsDecision(verdict: Verdict): boolean {
  return verdict.kind !== 'fullResolution' && verdict.kind !== 'fullResolutionWithWarning';
}

export function optionOf<K extends AdmissionOption['kind']>(
  report: AdmissionReport,
  kind: K
): Extract<AdmissionOption, { kind: K }> | null {
  return (report.options.find((option) => option.kind === kind) as Extract<AdmissionOption, { kind: K }>) ?? null;
}
