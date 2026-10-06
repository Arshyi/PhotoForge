/**
 * Rectangle geometry for choosing a region of a source.
 *
 * Everything here is a pure function on integer rectangles in *source pixels*.
 * The component converts pointer positions to source pixels and calls these; it
 * does no geometry of its own, so the rules that matter — a region never leaves
 * its source, never collapses, and keeps its aspect when locked — hold for
 * every input rather than only the ones a person happens to drag.
 */
import type { AdmissionReport, Rect, RegionCost } from './types';
import { optionOf } from './types';

export type Handle = 'n' | 's' | 'e' | 'w' | 'ne' | 'nw' | 'se' | 'sw';

export type AspectPreset = 'free' | 'original' | '1:1' | '3:2' | '4:3' | '5:4' | '16:9';

export const aspectPresets: { id: AspectPreset; label: string }[] = [
  { id: 'free', label: 'Free' },
  { id: 'original', label: 'Original' },
  { id: '1:1', label: '1:1' },
  { id: '3:2', label: '3:2' },
  { id: '4:3', label: '4:3' },
  { id: '5:4', label: '5:4' },
  { id: '16:9', label: '16:9' }
];

/** Width divided by height, or null for no constraint. */
export function aspectRatio(preset: AspectPreset, sourceWidth: number, sourceHeight: number): number | null {
  switch (preset) {
    case 'free':
      return null;
    case 'original':
      return sourceHeight > 0 ? sourceWidth / sourceHeight : null;
    default: {
      const [w, h] = preset.split(':').map(Number);
      return h > 0 ? w / h : null;
    }
  }
}

const MIN_SIZE = 1;

function clamp(value: number, low: number, high: number): number {
  return Math.min(Math.max(value, low), high);
}

/** Forces a rectangle inside the source, at least one pixel each way. */
export function clampRect(rect: Rect, sourceWidth: number, sourceHeight: number): Rect {
  const width = clamp(Math.round(rect.width), MIN_SIZE, sourceWidth);
  const height = clamp(Math.round(rect.height), MIN_SIZE, sourceHeight);
  return {
    x: clamp(Math.round(rect.x), 0, sourceWidth - width),
    y: clamp(Math.round(rect.y), 0, sourceHeight - height),
    width,
    height
  };
}

export function moveRect(rect: Rect, dx: number, dy: number, sourceWidth: number, sourceHeight: number): Rect {
  return clampRect({ ...rect, x: rect.x + dx, y: rect.y + dy }, sourceWidth, sourceHeight);
}

export function centerRect(rect: Rect, sourceWidth: number, sourceHeight: number): Rect {
  return clampRect(
    {
      ...rect,
      x: Math.round((sourceWidth - rect.width) / 2),
      y: Math.round((sourceHeight - rect.height) / 2)
    },
    sourceWidth,
    sourceHeight
  );
}

export function megapixels(rect: Rect): number {
  return (rect.width * rect.height) / 1_000_000;
}

/**
 * Resizes by dragging one handle by (dx, dy) source pixels.
 *
 * The opposite edge or corner stays fixed. With a ratio, the rectangle keeps it
 * to within one pixel of rounding. The result is always inside the source and at
 * least one pixel in each direction, however far the drag went.
 */
export function resizeRect(
  rect: Rect,
  handle: Handle,
  dx: number,
  dy: number,
  sourceWidth: number,
  sourceHeight: number,
  ratio: number | null = null
): Rect {
  let left = rect.x;
  let top = rect.y;
  let right = rect.x + rect.width;
  let bottom = rect.y + rect.height;

  if (handle.includes('w')) left = clamp(left + dx, 0, right - MIN_SIZE);
  if (handle.includes('e')) right = clamp(right + dx, left + MIN_SIZE, sourceWidth);
  if (handle.includes('n')) top = clamp(top + dy, 0, bottom - MIN_SIZE);
  if (handle.includes('s')) bottom = clamp(bottom + dy, top + MIN_SIZE, sourceHeight);

  let width = right - left;
  let height = bottom - top;

  if (ratio !== null && ratio > 0) {
    const horizontal = handle.includes('e') || handle.includes('w');
    const vertical = handle.includes('n') || handle.includes('s');

    if (horizontal && vertical) {
      // A corner: whichever side was dragged proportionally further decides.
      const scale = Math.max(width / rect.width, height / rect.height);
      width = Math.max(MIN_SIZE, Math.round(rect.width * scale));
      height = Math.max(MIN_SIZE, Math.round(width / ratio));
      // The fixed corner limits how far each side can grow.
      const room = {
        width: handle.includes('e') ? sourceWidth - rect.x : rect.x + rect.width,
        height: handle.includes('s') ? sourceHeight - rect.y : rect.y + rect.height
      };
      if (width > room.width) {
        width = room.width;
        height = Math.max(MIN_SIZE, Math.round(width / ratio));
      }
      if (height > room.height) {
        height = room.height;
        width = Math.max(MIN_SIZE, Math.round(height * ratio));
      }
      const anchorX = handle.includes('e') ? rect.x : rect.x + rect.width;
      const anchorY = handle.includes('s') ? rect.y : rect.y + rect.height;
      left = handle.includes('e') ? anchorX : anchorX - width;
      top = handle.includes('s') ? anchorY : anchorY - height;
      right = left + width;
      bottom = top + height;
    } else if (horizontal) {
      // An edge: the other dimension follows, centred on where it was.
      height = Math.max(MIN_SIZE, Math.round(width / ratio));
      if (height > sourceHeight) {
        height = sourceHeight;
        width = Math.max(MIN_SIZE, Math.round(height * ratio));
        if (handle.includes('w')) left = right - width;
        else right = left + width;
      }
      const centre = rect.y + rect.height / 2;
      top = clamp(Math.round(centre - height / 2), 0, sourceHeight - height);
      bottom = top + height;
    } else {
      width = Math.max(MIN_SIZE, Math.round(height * ratio));
      if (width > sourceWidth) {
        width = sourceWidth;
        height = Math.max(MIN_SIZE, Math.round(width / ratio));
        if (handle.includes('n')) top = bottom - height;
        else bottom = top + height;
      }
      const centre = rect.x + rect.width / 2;
      left = clamp(Math.round(centre - width / 2), 0, sourceWidth - width);
      right = left + width;
    }
    return clampRect({ x: left, y: top, width: right - left, height: bottom - top }, sourceWidth, sourceHeight);
  }

  return clampRect({ x: left, y: top, width, height }, sourceWidth, sourceHeight);
}

/** Applies an aspect preset to the current rectangle, keeping its centre. */
export function applyAspect(
  rect: Rect,
  ratio: number | null,
  sourceWidth: number,
  sourceHeight: number
): Rect {
  if (ratio === null || ratio <= 0) return rect;
  const area = rect.width * rect.height;
  let width = Math.round(Math.sqrt(area * ratio));
  let height = Math.round(width / ratio);
  if (width > sourceWidth) {
    width = sourceWidth;
    height = Math.round(width / ratio);
  }
  if (height > sourceHeight) {
    height = sourceHeight;
    width = Math.round(height * ratio);
  }
  const centreX = rect.x + rect.width / 2;
  const centreY = rect.y + rect.height / 2;
  return clampRect(
    { x: Math.round(centreX - width / 2), y: Math.round(centreY - height / 2), width, height },
    sourceWidth,
    sourceHeight
  );
}

/**
 * Peak bytes for a region of this many pixels, from the backend's coefficients.
 *
 * Evaluates the model; it does not own it. The coefficients come from the same
 * planner that decides whether the region will open, so what is shown while
 * dragging cannot disagree with what happens on Open.
 */
export function regionPeakBytes(cost: RegionCost, pixels: number): number {
  return Math.max(
    cost.openingFixed + cost.openingPerPixel * pixels,
    cost.editingFixed + cost.editingPerPixel * pixels
  );
}

export interface RegionStatus {
  pixels: number;
  peakBytes: number | null;
  supported: boolean;
  /** Why not, in words a person can act on. Empty when supported. */
  reason: string;
}

/** Whether a rectangle can be opened, and what it would cost. */
export function regionStatus(rect: Rect, report: AdmissionReport): RegionStatus {
  const pixels = rect.width * rect.height;
  const option = optionOf(report, 'openRegion');
  const peakBytes = report.regionCost ? regionPeakBytes(report.regionCost, pixels) : null;
  if (!option || !report.regionCost) {
    return {
      pixels,
      peakBytes,
      supported: false,
      reason: 'This file cannot be opened a region at a time.'
    };
  }
  if (pixels > option.maxRegionPixels) {
    return {
      pixels,
      peakBytes,
      supported: false,
      reason: `Too large: at most ${option.maxRegionPixels.toLocaleString('en-US')} pixels can be opened on this machine.`
    };
  }
  return { pixels, peakBytes, supported: true, reason: '' };
}

/**
 * The largest rectangle that fits the budget, in the chosen aspect, kept as close
 * to where the current one is as the source allows.
 *
 * With no aspect lock the source's own proportions are used, so "maximise" gives
 * the biggest undistorted piece rather than an arbitrary sliver.
 */
export function largestRegion(
  report: AdmissionReport,
  sourceWidth: number,
  sourceHeight: number,
  ratio: number | null,
  near: Rect | null
): Rect | null {
  const option = optionOf(report, 'openRegion');
  if (!option) return null;
  const cap = Math.min(option.maxRegionPixels, sourceWidth * sourceHeight);
  const target = ratio !== null && ratio > 0 ? ratio : sourceWidth / sourceHeight;

  let width = Math.floor(Math.sqrt(cap * target));
  let height = Math.floor(width / target);
  if (width > sourceWidth) {
    width = sourceWidth;
    height = Math.floor(width / target);
  }
  if (height > sourceHeight) {
    height = sourceHeight;
    width = Math.floor(height * target);
  }
  width = Math.max(1, width);
  height = Math.max(1, height);
  // Rounding can leave the product a hair over the cap.
  while (width * height > cap && (width > 1 || height > 1)) {
    if (width / height > target) width -= 1;
    else height -= 1;
  }

  const centreX = near ? near.x + near.width / 2 : sourceWidth / 2;
  const centreY = near ? near.y + near.height / 2 : sourceHeight / 2;
  return clampRect(
    { x: Math.round(centreX - width / 2), y: Math.round(centreY - height / 2), width, height },
    sourceWidth,
    sourceHeight
  );
}

/** A starting region: the largest allowed, centred. */
export function initialRegion(report: AdmissionReport, sourceWidth: number, sourceHeight: number): Rect {
  return (
    largestRegion(report, sourceWidth, sourceHeight, null, null) ??
    clampRect({ x: 0, y: 0, width: Math.min(sourceWidth, 1024), height: Math.min(sourceHeight, 1024) }, sourceWidth, sourceHeight)
  );
}

/** The size of a reduced copy at a scale: each side rounded, never below one. */
export function reducedSize(sourceWidth: number, sourceHeight: number, scale: number): { width: number; height: number } {
  const side = (length: number) => Math.min(length, Math.max(1, Math.round(length * scale)));
  return { width: side(sourceWidth), height: side(sourceHeight) };
}
