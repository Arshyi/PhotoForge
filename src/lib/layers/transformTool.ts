import { identityTransform, type LayerInterpolation, type LayerTransform } from './types';

/**
 * Interactive layer transform arithmetic.
 *
 * Every function here is pure: it maps a transform plus a pointer position to a
 * new transform, and never touches the DOM. The overlay component supplies
 * document-space coordinates and renders the result, so the geometry that is
 * easy to get wrong can be tested exhaustively without a browser.
 *
 * The forward mapping deliberately mirrors `layers::LayerTransform::forward` on
 * the Rust side — flip, then scale, then rotate about the layer's centre, then
 * translate. If the two ever diverge the handles would sit somewhere other than
 * the pixels the compositor draws, so `transformPoint` is the single definition
 * the whole UI shares.
 */

/** Mirrors `layers::transform::MIN_LAYER_SCALE`. */
export const MIN_LAYER_SCALE = 1 / 64;
/** Mirrors `layers::transform::MAX_LAYER_SCALE`. */
export const MAX_LAYER_SCALE = 64;
/** Mirrors `layers::transform::MAX_LAYER_TRANSLATION`. */
export const MAX_LAYER_TRANSLATION = 1_000_000;

/** Nudge distances, in document pixels, for the arrow keys. */
export const NUDGE_STEP = 1;
export const NUDGE_STEP_LARGE = 10;
/** Rotation snap applied while Shift is held. */
export const ROTATION_SNAP_DEGREES = 15;

export interface Point {
  x: number;
  y: number;
}

export type ScaleHandle = 'nw' | 'n' | 'ne' | 'e' | 'se' | 's' | 'sw' | 'w';
export type TransformHandle = ScaleHandle | 'move' | 'rotate';

export const scaleHandles: ScaleHandle[] = ['nw', 'n', 'ne', 'e', 'se', 's', 'sw', 'w'];

export interface GestureModifiers {
  shift: boolean;
  alt: boolean;
}

export interface TransformFrame {
  /** The layer's own corners in document space: TL, TR, BR, BL. */
  corners: [Point, Point, Point, Point];
  /** The layer's centre in document space, which is also the rotation pivot. */
  centre: Point;
  handles: { id: ScaleHandle; point: Point }[];
  /** The rotation grip, pushed out beyond the top edge. */
  rotate: Point;
  /** Where the rotation grip's tether meets the box. */
  rotateAnchor: Point;
}

export interface TransformGesture {
  handle: TransformHandle;
  start: LayerTransform;
  width: number;
  height: number;
  /** Pointer position, in document space, when the gesture began. */
  origin: Point;
  /** Layer-space point that must not move (scale gestures). */
  anchorLayer: Point;
  /** Layer-space point being dragged (scale gestures). */
  gripLayer: Point;
  /** Document-space position of `anchorLayer` at gesture start. */
  anchorDoc: Point;
  /** Document-space centre at gesture start (rotation pivot). */
  centreDoc: Point;
  /** Pointer bearing from the centre when a rotation began, in radians. */
  startAngle: number;
  /** Whether the transform panel's aspect lock was on when the drag started. */
  aspectLocked: boolean;
}

/** The numbers the transform options panel edits. */
export interface TransformMetrics {
  centerX: number;
  centerY: number;
  width: number;
  height: number;
  rotationDegrees: number;
  scaleX: number;
  scaleY: number;
}

export type TransformMetric = keyof TransformMetrics;

function isFiniteNumber(value: unknown): value is number {
  return typeof value === 'number' && Number.isFinite(value);
}

/**
 * Clamps one axis of scale into the range the renderer can invert.
 *
 * A sign is meaningful — dragging a handle past its anchor mirrors the layer —
 * so the magnitude is clamped and the sign kept. A zero or non-finite value has
 * no usable sign, so it falls back to the sign of the value it replaces.
 */
export function clampScale(value: number, fallback = 1): number {
  const sign = Math.sign(isFiniteNumber(value) && value !== 0 ? value : fallback) || 1;
  const magnitude = isFiniteNumber(value) ? Math.abs(value) : Math.abs(fallback);
  const bounded = Math.min(MAX_LAYER_SCALE, Math.max(MIN_LAYER_SCALE, magnitude || MIN_LAYER_SCALE));
  return sign * bounded;
}

function clampTranslation(value: number, fallback = 0): number {
  const base = isFiniteNumber(value) ? value : fallback;
  return Math.min(MAX_LAYER_TRANSLATION, Math.max(-MAX_LAYER_TRANSLATION, base));
}

/** Wraps a rotation into the +/-360 degree window the document model accepts. */
export function wrapRotation(value: number, fallback = 0): number {
  const base = isFiniteNumber(value) ? value : fallback;
  const wrapped = base % 360;
  // -0 renders as "-0" in the angle field, which reads like a bug.
  return Object.is(wrapped, -0) ? 0 : wrapped;
}

/**
 * Repairs any transform-shaped value into one the renderer and the Rust
 * validator both accept.
 *
 * This is the single gate every transform passes through — pointer gestures,
 * typed fields, and transforms read back from a project file — so a NaN, an
 * infinity, a zero scale, or a wildly out-of-range value can never reach the
 * document model.
 */
export function sanitizeTransform(
  input: Partial<LayerTransform> | null | undefined,
  fallback: LayerTransform = identityTransform
): LayerTransform {
  const source = (input ?? {}) as Partial<LayerTransform>;
  const interpolation: LayerInterpolation =
    source.interpolation === 'nearest' || source.interpolation === 'bilinear'
      ? source.interpolation
      : fallback.interpolation;
  return {
    translateX: clampTranslation(source.translateX as number, fallback.translateX),
    translateY: clampTranslation(source.translateY as number, fallback.translateY),
    scaleX: clampScale(source.scaleX as number, fallback.scaleX),
    scaleY: clampScale(source.scaleY as number, fallback.scaleY),
    rotationDegrees: wrapRotation(source.rotationDegrees as number, fallback.rotationDegrees),
    flipHorizontal: Boolean(source.flipHorizontal),
    flipVertical: Boolean(source.flipVertical),
    interpolation
  };
}

export function resetTransform(interpolation: LayerInterpolation = 'bilinear'): LayerTransform {
  return { ...identityTransform, interpolation };
}

export function isIdentityTransform(transform: LayerTransform): boolean {
  return (
    transform.translateX === 0 &&
    transform.translateY === 0 &&
    transform.scaleX === 1 &&
    transform.scaleY === 1 &&
    transform.rotationDegrees === 0 &&
    !transform.flipHorizontal &&
    !transform.flipVertical
  );
}

/** The scale actually applied on each axis, with the flip flags folded in. */
export function effectiveScale(transform: LayerTransform): Point {
  return {
    x: transform.flipHorizontal ? -transform.scaleX : transform.scaleX,
    y: transform.flipVertical ? -transform.scaleY : transform.scaleY
  };
}

function rotate(point: Point, sin: number, cos: number): Point {
  return { x: point.x * cos - point.y * sin, y: point.x * sin + point.y * cos };
}

/**
 * Maps a point from the layer's own pixel grid into document space.
 *
 * This is the TypeScript twin of `LayerTransform::forward`.
 */
export function transformPoint(
  transform: LayerTransform,
  width: number,
  height: number,
  point: Point
): Point {
  const centreX = width / 2;
  const centreY = height / 2;
  const scale = effectiveScale(transform);
  const radians = (transform.rotationDegrees * Math.PI) / 180;
  const sin = Math.sin(radians);
  const cos = Math.cos(radians);
  const local = { x: (point.x - centreX) * scale.x, y: (point.y - centreY) * scale.y };
  const spun = rotate(local, sin, cos);
  return {
    x: spun.x + centreX + transform.translateX,
    y: spun.y + centreY + transform.translateY
  };
}

/** Maps a document-space point back into the layer's pixel grid. */
export function inverseTransformPoint(
  transform: LayerTransform,
  width: number,
  height: number,
  point: Point
): Point {
  const centreX = width / 2;
  const centreY = height / 2;
  const scale = effectiveScale(transform);
  const radians = (transform.rotationDegrees * Math.PI) / 180;
  const sin = Math.sin(radians);
  const cos = Math.cos(radians);
  const shifted = {
    x: point.x - centreX - transform.translateX,
    y: point.y - centreY - transform.translateY
  };
  const unspun = { x: shifted.x * cos + shifted.y * sin, y: -shifted.x * sin + shifted.y * cos };
  return { x: unspun.x / scale.x + centreX, y: unspun.y / scale.y + centreY };
}

/** The layer-space position of each scale handle, before any transform. */
export function handleLayerPoint(handle: ScaleHandle, width: number, height: number): Point {
  const left = 0;
  const right = width;
  const top = 0;
  const bottom = height;
  const midX = width / 2;
  const midY = height / 2;
  switch (handle) {
    case 'nw':
      return { x: left, y: top };
    case 'n':
      return { x: midX, y: top };
    case 'ne':
      return { x: right, y: top };
    case 'e':
      return { x: right, y: midY };
    case 'se':
      return { x: right, y: bottom };
    case 's':
      return { x: midX, y: bottom };
    case 'sw':
      return { x: left, y: bottom };
    case 'w':
      return { x: left, y: midY };
  }
}

/** The handle diagonally or directly opposite, which stays put while scaling. */
export function oppositeHandle(handle: ScaleHandle): ScaleHandle {
  const opposites: Record<ScaleHandle, ScaleHandle> = {
    nw: 'se',
    n: 's',
    ne: 'sw',
    e: 'w',
    se: 'nw',
    s: 'n',
    sw: 'ne',
    w: 'e'
  };
  return opposites[handle];
}

/**
 * Everything the overlay draws, in document coordinates.
 *
 * `gripDistance` is how far the rotation grip sits outside the box, expressed in
 * document pixels so the caller can keep it a constant size on screen whatever
 * the viewport zoom is.
 */
export function transformFrame(
  transform: LayerTransform,
  width: number,
  height: number,
  gripDistance = Math.max(width, height) * 0.08
): TransformFrame {
  const map = (point: Point) => transformPoint(transform, width, height, point);
  const corners: [Point, Point, Point, Point] = [
    map({ x: 0, y: 0 }),
    map({ x: width, y: 0 }),
    map({ x: width, y: height }),
    map({ x: 0, y: height })
  ];
  const centre = map({ x: width / 2, y: height / 2 });
  const handles = scaleHandles.map((id) => ({
    id,
    point: map(handleLayerPoint(id, width, height))
  }));
  const rotateAnchor = map({ x: width / 2, y: 0 });
  // Push the grip along the box's own outward normal so it stays above the top
  // edge however the layer is rotated or flipped.
  const inward = map({ x: width / 2, y: height / 2 });
  const dx = rotateAnchor.x - inward.x;
  const dy = rotateAnchor.y - inward.y;
  const length = Math.hypot(dx, dy) || 1;
  const rotatePoint = {
    x: rotateAnchor.x + (dx / length) * gripDistance,
    y: rotateAnchor.y + (dy / length) * gripDistance
  };
  return { corners, centre, handles, rotate: rotatePoint, rotateAnchor };
}

/** True when a document-space point lies inside the transformed layer box. */
export function pointInQuad(corners: Point[], point: Point): boolean {
  let inside = false;
  for (let index = 0, previous = corners.length - 1; index < corners.length; previous = index++) {
    const a = corners[index];
    const b = corners[previous];
    const straddles = a.y > point.y !== b.y > point.y;
    if (!straddles) continue;
    const crossingX = ((b.x - a.x) * (point.y - a.y)) / (b.y - a.y) + a.x;
    if (point.x < crossingX) inside = !inside;
  }
  return inside;
}

/**
 * Which control a pointer at `point` grabs.
 *
 * Handles win over the body, and the nearest handle wins over a further one, so
 * the corners of a layer scaled down to nearly nothing stay individually
 * reachable instead of the first match swallowing the rest.
 */
export function hitTestHandle(
  frame: TransformFrame,
  point: Point,
  tolerance: number
): TransformHandle | null {
  let best: { id: TransformHandle; distance: number } | null = null;
  const consider = (id: TransformHandle, candidate: Point) => {
    const distance = Math.hypot(candidate.x - point.x, candidate.y - point.y);
    if (distance > tolerance) return;
    if (!best || distance < best.distance) best = { id, distance };
  };
  consider('rotate', frame.rotate);
  for (const handle of frame.handles) consider(handle.id, handle.point);
  if (best) return (best as { id: TransformHandle }).id;
  return pointInQuad(frame.corners, point) ? 'move' : null;
}

/**
 * The CSS cursor for a handle, turned to follow the layer's own rotation.
 *
 * A box rotated 90 degrees has its "north" handle pointing east, and a cursor
 * that still says "resize vertically" would be actively misleading.
 */
export function handleCursor(handle: TransformHandle, rotationDegrees = 0): string {
  if (handle === 'move') return 'move';
  if (handle === 'rotate') return 'grab';
  const bearings: Record<ScaleHandle, number> = {
    n: 0,
    ne: 45,
    e: 90,
    se: 135,
    s: 180,
    sw: 225,
    w: 270,
    nw: 315
  };
  const cursors = ['ns-resize', 'nesw-resize', 'ew-resize', 'nwse-resize'];
  const bearing = bearings[handle] + (isFiniteNumber(rotationDegrees) ? rotationDegrees : 0);
  const normalized = ((bearing % 180) + 180) % 180;
  return cursors[Math.round(normalized / 45) % 4];
}

/** Captures everything a drag needs so later moves depend only on the pointer. */
export function beginTransformGesture(
  handle: TransformHandle,
  transform: LayerTransform,
  width: number,
  height: number,
  origin: Point,
  options: { aspectLocked?: boolean; fromCentre?: boolean } = {}
): TransformGesture {
  const start = sanitizeTransform(transform);
  const centreLayer = { x: width / 2, y: height / 2 };
  const centreDoc = transformPoint(start, width, height, centreLayer);
  const scaling = handle !== 'move' && handle !== 'rotate';
  const gripLayer = scaling
    ? handleLayerPoint(handle as ScaleHandle, width, height)
    : { ...centreLayer };
  const anchorLayer =
    scaling && !options.fromCentre
      ? handleLayerPoint(oppositeHandle(handle as ScaleHandle), width, height)
      : { ...centreLayer };
  return {
    handle,
    start,
    width,
    height,
    origin,
    anchorLayer,
    gripLayer,
    anchorDoc: transformPoint(start, width, height, anchorLayer),
    centreDoc,
    startAngle: Math.atan2(origin.y - centreDoc.y, origin.x - centreDoc.x),
    aspectLocked: Boolean(options.aspectLocked)
  };
}

function moveGesture(gesture: TransformGesture, point: Point, shift: boolean): LayerTransform {
  let dx = point.x - gesture.origin.x;
  let dy = point.y - gesture.origin.y;
  // Shift constrains a move to the axis the pointer has travelled furthest on.
  if (shift) {
    if (Math.abs(dx) >= Math.abs(dy)) dy = 0;
    else dx = 0;
  }
  return sanitizeTransform(
    {
      ...gesture.start,
      translateX: gesture.start.translateX + dx,
      translateY: gesture.start.translateY + dy
    },
    gesture.start
  );
}

function rotateGesture(gesture: TransformGesture, point: Point, shift: boolean): LayerTransform {
  const angle = Math.atan2(point.y - gesture.centreDoc.y, point.x - gesture.centreDoc.x);
  const delta = ((angle - gesture.startAngle) * 180) / Math.PI;
  let rotation = gesture.start.rotationDegrees + delta;
  if (shift) rotation = Math.round(rotation / ROTATION_SNAP_DEGREES) * ROTATION_SNAP_DEGREES;
  return sanitizeTransform({ ...gesture.start, rotationDegrees: rotation }, gesture.start);
}

function scaleGesture(
  gesture: TransformGesture,
  point: Point,
  modifiers: GestureModifiers
): LayerTransform {
  const { start, width, height } = gesture;
  const startScale = effectiveScale(start);
  const radians = (start.rotationDegrees * Math.PI) / 180;
  const sin = Math.sin(radians);
  const cos = Math.cos(radians);

  // Alt re-anchors the drag on the centre, so the layer grows both ways.
  const anchorLayer = modifiers.alt ? { x: width / 2, y: height / 2 } : gesture.anchorLayer;
  const anchorDoc = modifiers.alt
    ? transformPoint(start, width, height, anchorLayer)
    : gesture.anchorDoc;

  // Undo the rotation so the remaining arithmetic happens on the layer's own
  // axes: the anchor is fixed, so the dragged handle's offset from it is
  // exactly the new scale times its unscaled offset.
  const offset = { x: point.x - anchorDoc.x, y: point.y - anchorDoc.y };
  const local = { x: offset.x * cos + offset.y * sin, y: -offset.x * sin + offset.y * cos };
  const span = { x: gesture.gripLayer.x - anchorLayer.x, y: gesture.gripLayer.y - anchorLayer.y };

  let scaleX = span.x !== 0 ? local.x / span.x : startScale.x;
  let scaleY = span.y !== 0 ? local.y / span.y : startScale.y;

  // Shift toggles the panel's aspect lock rather than only ever enabling it, so
  // the modifier is useful in both directions.
  const uniform = gesture.aspectLocked !== modifiers.shift;
  if (uniform) {
    const ratioX = span.x !== 0 ? Math.abs(scaleX / startScale.x) : 0;
    const ratioY = span.y !== 0 ? Math.abs(scaleY / startScale.y) : 0;
    const ratio = Math.max(ratioX, ratioY) || 1;
    scaleX = Math.sign(span.x !== 0 ? scaleX : startScale.x) * Math.abs(startScale.x) * ratio;
    scaleY = Math.sign(span.y !== 0 ? scaleY : startScale.y) * Math.abs(startScale.y) * ratio;
  }

  scaleX = clampScale(scaleX, startScale.x);
  scaleY = clampScale(scaleY, startScale.y);

  // Put the anchor back where it started, given the new scale.
  const anchorOffset = {
    x: (anchorLayer.x - width / 2) * scaleX,
    y: (anchorLayer.y - height / 2) * scaleY
  };
  const spun = rotate(anchorOffset, sin, cos);
  return sanitizeTransform(
    {
      ...start,
      // The flip flags are the user's, not the drag's, so the sign of the drag
      // lives in the scale and the flags survive untouched.
      scaleX: start.flipHorizontal ? -scaleX : scaleX,
      scaleY: start.flipVertical ? -scaleY : scaleY,
      translateX: anchorDoc.x - width / 2 - spun.x,
      translateY: anchorDoc.y - height / 2 - spun.y
    },
    start
  );
}

/** The transform a gesture produces for a pointer now at `point`. */
export function applyTransformGesture(
  gesture: TransformGesture,
  point: Point,
  modifiers: GestureModifiers = { shift: false, alt: false }
): LayerTransform {
  if (!isFiniteNumber(point.x) || !isFiniteNumber(point.y)) return gesture.start;
  if (gesture.handle === 'move') return moveGesture(gesture, point, modifiers.shift);
  if (gesture.handle === 'rotate') return rotateGesture(gesture, point, modifiers.shift);
  return scaleGesture(gesture, point, modifiers);
}

/** Moves a layer by whole document pixels, for the arrow keys. */
export function nudgeTransform(
  transform: LayerTransform,
  dx: number,
  dy: number
): LayerTransform {
  const base = sanitizeTransform(transform);
  return sanitizeTransform(
    { ...base, translateX: base.translateX + dx, translateY: base.translateY + dy },
    base
  );
}

export function flipTransform(
  transform: LayerTransform,
  axis: 'horizontal' | 'vertical'
): LayerTransform {
  const base = sanitizeTransform(transform);
  return axis === 'horizontal'
    ? { ...base, flipHorizontal: !base.flipHorizontal }
    : { ...base, flipVertical: !base.flipVertical };
}

/**
 * The transform expressed the way the options panel shows it.
 *
 * Position is the layer centre rather than a corner: the centre is the rotation
 * pivot, so it is the one point that does not jump around while the angle
 * changes. Width and height are the layer's scaled size, independent of angle.
 */
export function transformMetrics(
  transform: LayerTransform,
  width: number,
  height: number
): TransformMetrics {
  const base = sanitizeTransform(transform);
  return {
    centerX: width / 2 + base.translateX,
    centerY: height / 2 + base.translateY,
    width: Math.abs(base.scaleX) * width,
    height: Math.abs(base.scaleY) * height,
    rotationDegrees: base.rotationDegrees,
    scaleX: base.scaleX,
    scaleY: base.scaleY
  };
}

/**
 * Applies one edited field from the options panel.
 *
 * Editing width or a scale with the aspect lock on carries the same ratio to the
 * other axis, so the layer keeps its proportions.
 */
export function withMetric(
  transform: LayerTransform,
  width: number,
  height: number,
  metric: TransformMetric,
  value: number,
  aspectLocked = false
): LayerTransform {
  const base = sanitizeTransform(transform);
  if (!isFiniteNumber(value)) return base;
  const next: LayerTransform = { ...base };
  const applyScale = (axis: 'x' | 'y', magnitude: number) => {
    const ratio = Math.abs(magnitude) / Math.abs(axis === 'x' ? base.scaleX : base.scaleY);
    if (axis === 'x') {
      next.scaleX = Math.sign(base.scaleX || 1) * Math.abs(magnitude);
      if (aspectLocked && Number.isFinite(ratio)) {
        next.scaleY = Math.sign(base.scaleY || 1) * Math.abs(base.scaleY) * ratio;
      }
    } else {
      next.scaleY = Math.sign(base.scaleY || 1) * Math.abs(magnitude);
      if (aspectLocked && Number.isFinite(ratio)) {
        next.scaleX = Math.sign(base.scaleX || 1) * Math.abs(base.scaleX) * ratio;
      }
    }
  };

  switch (metric) {
    case 'centerX':
      next.translateX = value - width / 2;
      break;
    case 'centerY':
      next.translateY = value - height / 2;
      break;
    case 'width':
      if (width > 0) applyScale('x', value / width);
      break;
    case 'height':
      if (height > 0) applyScale('y', value / height);
      break;
    case 'rotationDegrees':
      next.rotationDegrees = value;
      break;
    case 'scaleX':
      applyScale('x', value);
      break;
    case 'scaleY':
      applyScale('y', value);
      break;
  }
  return sanitizeTransform(next, base);
}

/**
 * The transformed layer's axis-aligned document bounds, which the panel shows
 * so a layer dragged off-canvas can still be found.
 */
export function documentBounds(
  transform: LayerTransform,
  width: number,
  height: number
): { left: number; top: number; right: number; bottom: number } {
  const { corners } = transformFrame(transform, width, height, 0);
  const xs = corners.map((corner) => corner.x);
  const ys = corners.map((corner) => corner.y);
  return {
    left: Math.min(...xs),
    top: Math.min(...ys),
    right: Math.max(...xs),
    bottom: Math.max(...ys)
  };
}

/** Whether any part of the transformed layer still overlaps the canvas. */
export function intersectsCanvas(
  transform: LayerTransform,
  width: number,
  height: number,
  canvasWidth: number,
  canvasHeight: number
): boolean {
  const bounds = documentBounds(transform, width, height);
  return (
    bounds.right > 0 &&
    bounds.bottom > 0 &&
    bounds.left < canvasWidth &&
    bounds.top < canvasHeight
  );
}
