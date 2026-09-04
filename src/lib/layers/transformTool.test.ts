import { describe, expect, it } from 'vitest';
import {
  applyTransformGesture,
  beginTransformGesture,
  clampScale,
  documentBounds,
  effectiveScale,
  flipTransform,
  handleCursor,
  handleLayerPoint,
  hitTestHandle,
  intersectsCanvas,
  inverseTransformPoint,
  isIdentityTransform,
  nudgeTransform,
  oppositeHandle,
  pointInQuad,
  resetTransform,
  sanitizeTransform,
  transformFrame,
  transformMetrics,
  transformPoint,
  withMetric,
  wrapRotation,
  MAX_LAYER_SCALE,
  MAX_LAYER_TRANSLATION,
  MIN_LAYER_SCALE,
  type Point,
  type ScaleHandle
} from './transformTool';
import { identityTransform, type LayerTransform } from './types';

const W = 100;
const H = 60;

function make(overrides: Partial<LayerTransform> = {}): LayerTransform {
  return { ...identityTransform, ...overrides };
}

function close(actual: number, expected: number, tolerance = 1e-6) {
  expect(Math.abs(actual - expected)).toBeLessThanOrEqual(tolerance);
}

function closePoint(actual: Point, expected: Point, tolerance = 1e-6) {
  close(actual.x, expected.x, tolerance);
  close(actual.y, expected.y, tolerance);
}

describe('transformPoint', () => {
  it('leaves a point alone under the identity transform', () => {
    closePoint(transformPoint(make(), W, H, { x: 17, y: 23 }), { x: 17, y: 23 });
  });

  it('translates by whole document pixels', () => {
    const moved = transformPoint(make({ translateX: 12, translateY: -5 }), W, H, { x: 0, y: 0 });
    closePoint(moved, { x: 12, y: -5 });
  });

  it('scales about the layer centre, not its corner', () => {
    const scaled = make({ scaleX: 2, scaleY: 2 });
    closePoint(transformPoint(scaled, W, H, { x: W / 2, y: H / 2 }), { x: W / 2, y: H / 2 });
    closePoint(transformPoint(scaled, W, H, { x: 0, y: 0 }), { x: -W / 2, y: -H / 2 });
  });

  it('rotates about the layer centre', () => {
    const spun = make({ rotationDegrees: 90 });
    // The top-left corner lands where the bottom-left corner was.
    closePoint(transformPoint(spun, W, H, { x: 0, y: 0 }), { x: 80, y: -20 }, 1e-4);
    closePoint(transformPoint(spun, W, H, { x: W / 2, y: H / 2 }), { x: W / 2, y: H / 2 }, 1e-4);
  });

  it('treats a flip as a negative effective scale', () => {
    const flipped = make({ flipHorizontal: true });
    expect(effectiveScale(flipped)).toEqual({ x: -1, y: 1 });
    closePoint(transformPoint(flipped, W, H, { x: 0, y: 0 }), { x: W, y: 0 });
  });

  /**
   * The renderer maps document pixels back through `InverseTransform`. If the
   * two ever disagreed the handles would sit somewhere other than the pixels,
   * so every component is round-tripped together.
   */
  it('round-trips through its own inverse for every component at once', () => {
    const transform = make({
      translateX: 12.5,
      translateY: -7.25,
      scaleX: 1.75,
      scaleY: 0.6,
      rotationDegrees: 33,
      flipHorizontal: true
    });
    for (const point of [
      { x: 0, y: 0 },
      { x: 12, y: 5 },
      { x: W, y: H },
      { x: 3.5, y: 21.75 }
    ]) {
      const forward = transformPoint(transform, W, H, point);
      closePoint(inverseTransformPoint(transform, W, H, forward), point, 1e-4);
    }
  });
});

describe('sanitizeTransform', () => {
  it('accepts a well-formed transform unchanged', () => {
    const transform = make({ translateX: 4, scaleX: 2, rotationDegrees: 45 });
    expect(sanitizeTransform(transform)).toEqual(transform);
  });

  it.each([
    ['NaN', Number.NaN],
    ['positive infinity', Number.POSITIVE_INFINITY],
    ['negative infinity', Number.NEGATIVE_INFINITY]
  ])('replaces a %s translation with the previous value', (_label, value) => {
    const previous = make({ translateX: 9 });
    expect(sanitizeTransform({ ...previous, translateX: value }, previous).translateX).toBe(9);
  });

  it('replaces a zero scale with the smallest the renderer can invert', () => {
    const result = sanitizeTransform(make({ scaleX: 0, scaleY: 0 }));
    expect(result.scaleX).toBe(MIN_LAYER_SCALE);
    expect(result.scaleY).toBe(MIN_LAYER_SCALE);
  });

  it('clamps an enormous scale to the renderer ceiling', () => {
    const result = sanitizeTransform(make({ scaleX: 1e9, scaleY: -1e9 }));
    expect(result.scaleX).toBe(MAX_LAYER_SCALE);
    // A mirrored layer keeps its mirroring while the magnitude is clamped.
    expect(result.scaleY).toBe(-MAX_LAYER_SCALE);
  });

  it('clamps a runaway translation to the documented bound', () => {
    const result = sanitizeTransform(make({ translateX: 1e12, translateY: -1e12 }));
    expect(result.translateX).toBe(MAX_LAYER_TRANSLATION);
    expect(result.translateY).toBe(-MAX_LAYER_TRANSLATION);
  });

  it('wraps a rotation beyond a full turn back inside the accepted range', () => {
    expect(sanitizeTransform(make({ rotationDegrees: 725 })).rotationDegrees).toBeCloseTo(5, 6);
    expect(sanitizeTransform(make({ rotationDegrees: -725 })).rotationDegrees).toBeCloseTo(-5, 6);
    expect(Object.is(wrapRotation(-360), -0)).toBe(false);
  });

  it('coerces missing and non-boolean flip flags', () => {
    const result = sanitizeTransform({ flipHorizontal: 1 as unknown as boolean });
    expect(result.flipHorizontal).toBe(true);
    expect(result.flipVertical).toBe(false);
  });

  it('falls back to bilinear for an unknown interpolation mode', () => {
    expect(
      sanitizeTransform({ interpolation: 'lanczos' as unknown as 'nearest' }).interpolation
    ).toBe('bilinear');
    expect(sanitizeTransform({ interpolation: 'nearest' }).interpolation).toBe('nearest');
  });

  it('repairs a wholly malformed value into an identity placement', () => {
    expect(sanitizeTransform(null)).toEqual(identityTransform);
    expect(sanitizeTransform(undefined)).toEqual(identityTransform);
    expect(sanitizeTransform({} as LayerTransform)).toEqual(identityTransform);
  });

  it('never reports a repaired transform as invalid to the renderer', () => {
    const hostile = [
      { scaleX: Number.NaN, scaleY: 0, translateX: Number.POSITIVE_INFINITY },
      { rotationDegrees: Number.NaN, scaleX: -0 },
      { scaleX: 1e300, scaleY: -1e-300, translateY: Number.NaN }
    ];
    for (const input of hostile) {
      const result = sanitizeTransform(input as Partial<LayerTransform>);
      expect(Number.isFinite(result.translateX)).toBe(true);
      expect(Number.isFinite(result.translateY)).toBe(true);
      expect(Math.abs(result.scaleX)).toBeGreaterThanOrEqual(MIN_LAYER_SCALE);
      expect(Math.abs(result.scaleX)).toBeLessThanOrEqual(MAX_LAYER_SCALE);
      expect(Math.abs(result.scaleY)).toBeGreaterThanOrEqual(MIN_LAYER_SCALE);
      expect(Math.abs(result.scaleY)).toBeLessThanOrEqual(MAX_LAYER_SCALE);
      expect(Math.abs(result.rotationDegrees)).toBeLessThanOrEqual(360);
    }
  });

  it('keeps a sign when clamping a scale magnitude', () => {
    expect(clampScale(-1e9)).toBe(-MAX_LAYER_SCALE);
    expect(clampScale(0, -1)).toBe(-MIN_LAYER_SCALE);
    expect(clampScale(Number.NaN, 3)).toBe(3);
  });
});

describe('frame geometry', () => {
  it('places the eight handles on the untransformed box', () => {
    const frame = transformFrame(make(), W, H, 10);
    const byId = Object.fromEntries(frame.handles.map((handle) => [handle.id, handle.point]));
    closePoint(byId.nw, { x: 0, y: 0 });
    closePoint(byId.se, { x: W, y: H });
    closePoint(byId.n, { x: W / 2, y: 0 });
    closePoint(byId.w, { x: 0, y: H / 2 });
    closePoint(frame.centre, { x: W / 2, y: H / 2 });
  });

  it('pushes the rotation grip beyond the top edge and follows the rotation', () => {
    const upright = transformFrame(make(), W, H, 10);
    expect(upright.rotate.y).toBeLessThan(upright.rotateAnchor.y);
    const spun = transformFrame(make({ rotationDegrees: 180 }), W, H, 10);
    // Turned upside down, the grip must end up below the box, not above it.
    expect(spun.rotate.y).toBeGreaterThan(spun.centre.y);
  });

  it('reports the axis-aligned bounds of a rotated layer', () => {
    const bounds = documentBounds(make({ rotationDegrees: 90 }), W, H);
    close(bounds.left, 20, 1e-4);
    close(bounds.right, 80, 1e-4);
    close(bounds.top, -20, 1e-4);
    close(bounds.bottom, 80, 1e-4);
  });

  it('knows when a layer has been dragged entirely off the canvas', () => {
    expect(intersectsCanvas(make(), W, H, 200, 200)).toBe(true);
    expect(intersectsCanvas(make({ translateX: -500 }), W, H, 200, 200)).toBe(false);
    // Touching by a single pixel still counts as on-canvas.
    expect(intersectsCanvas(make({ translateX: -99 }), W, H, 200, 200)).toBe(true);
  });

  it('opposes each handle with the one that stays put while it is dragged', () => {
    const pairs: [ScaleHandle, ScaleHandle][] = [
      ['nw', 'se'],
      ['n', 's'],
      ['ne', 'sw'],
      ['e', 'w']
    ];
    for (const [handle, expected] of pairs) {
      expect(oppositeHandle(handle)).toBe(expected);
      expect(oppositeHandle(expected)).toBe(handle);
    }
  });
});

describe('hit testing', () => {
  const frame = transformFrame(make(), W, H, 20);

  it('grabs a handle the pointer is near', () => {
    expect(hitTestHandle(frame, { x: 1, y: 1 }, 6)).toBe('nw');
    expect(hitTestHandle(frame, { x: W - 1, y: H - 1 }, 6)).toBe('se');
  });

  it('grabs the rotation grip above the top edge', () => {
    expect(hitTestHandle(frame, frame.rotate, 6)).toBe('rotate');
  });

  it('grabs the body for a point inside the box but away from a handle', () => {
    expect(hitTestHandle(frame, { x: W / 2, y: H / 2 + 8 }, 6)).toBe('move');
  });

  it('grabs nothing outside the box', () => {
    expect(hitTestHandle(frame, { x: -50, y: -50 }, 6)).toBeNull();
  });

  /**
   * With a naive first-match search, a layer scaled down until its handles
   * overlap would answer with whichever handle happened to be listed first, so
   * some corners could never be grabbed. The nearest handle must win.
   */
  it('prefers the nearest handle when several are within reach', () => {
    const tiny = transformFrame(make({ scaleX: 1 / 32, scaleY: 1 / 32 }), W, H, 20);
    const byId = Object.fromEntries(tiny.handles.map((handle) => [handle.id, handle.point]));
    expect(hitTestHandle(tiny, byId.nw, 40)).toBe('nw');
    expect(hitTestHandle(tiny, byId.se, 40)).toBe('se');
  });

  it('follows a rotated box rather than its axis-aligned bounds', () => {
    const spun = transformFrame(make({ rotationDegrees: 45, scaleX: 0.5, scaleY: 0.5 }), W, H, 20);
    expect(pointInQuad(spun.corners, spun.centre)).toBe(true);
    // A corner of the axis-aligned bounding box falls outside the turned box.
    const bounds = documentBounds(make({ rotationDegrees: 45, scaleX: 0.5, scaleY: 0.5 }), W, H);
    expect(pointInQuad(spun.corners, { x: bounds.left, y: bounds.top })).toBe(false);
  });
});

describe('move gestures', () => {
  it('follows the pointer one document pixel at a time', () => {
    const gesture = beginTransformGesture('move', make(), W, H, { x: 50, y: 30 });
    const moved = applyTransformGesture(gesture, { x: 62, y: 37 });
    expect(moved.translateX).toBe(12);
    expect(moved.translateY).toBe(7);
  });

  it('constrains to the dominant axis while Shift is held', () => {
    const gesture = beginTransformGesture('move', make(), W, H, { x: 50, y: 30 });
    const horizontal = applyTransformGesture(gesture, { x: 90, y: 33 }, { shift: true, alt: false });
    expect(horizontal.translateX).toBe(40);
    expect(horizontal.translateY).toBe(0);
    const vertical = applyTransformGesture(gesture, { x: 52, y: 90 }, { shift: true, alt: false });
    expect(vertical.translateX).toBe(0);
    expect(vertical.translateY).toBe(60);
  });

  it('is stateless, so every move is measured from where the drag began', () => {
    const gesture = beginTransformGesture('move', make({ translateX: 5 }), W, H, { x: 0, y: 0 });
    applyTransformGesture(gesture, { x: 100, y: 100 });
    applyTransformGesture(gesture, { x: 300, y: 300 });
    // Coming back to the start restores the transform exactly rather than
    // accumulating every intermediate move.
    expect(applyTransformGesture(gesture, { x: 0, y: 0 })).toEqual(make({ translateX: 5 }));
  });

  it('ignores a pointer position the browser could not resolve', () => {
    const gesture = beginTransformGesture('move', make(), W, H, { x: 0, y: 0 });
    expect(applyTransformGesture(gesture, { x: Number.NaN, y: 10 })).toEqual(gesture.start);
  });
});

describe('scale gestures', () => {
  it('keeps the opposite corner fixed while the dragged corner follows', () => {
    const anchor = { x: 0, y: 0 };
    const gesture = beginTransformGesture('se', make(), W, H, { x: W, y: H });
    const next = applyTransformGesture(gesture, { x: 2 * W, y: 2 * H });
    closePoint(transformPoint(next, W, H, { x: 0, y: 0 }), anchor, 1e-4);
    closePoint(transformPoint(next, W, H, { x: W, y: H }), { x: 2 * W, y: 2 * H }, 1e-4);
    close(next.scaleX, 2, 1e-6);
    close(next.scaleY, 2, 1e-6);
  });

  it('moves only one axis for an edge handle', () => {
    const gesture = beginTransformGesture('e', make(), W, H, { x: W, y: H / 2 });
    const next = applyTransformGesture(gesture, { x: 2 * W, y: H });
    close(next.scaleX, 2, 1e-6);
    close(next.scaleY, 1, 1e-6);
  });

  it('scales from the centre while Alt is held', () => {
    const gesture = beginTransformGesture('se', make(), W, H, { x: W, y: H });
    const next = applyTransformGesture(gesture, { x: W + 50, y: H + 30 }, { shift: false, alt: true });
    // The centre is what stays put, so the box grows in both directions.
    closePoint(transformPoint(next, W, H, { x: W / 2, y: H / 2 }), { x: W / 2, y: H / 2 }, 1e-4);
    close(next.scaleX, 2, 1e-6);
    close(next.scaleY, 2, 1e-6);
  });

  it('keeps proportions when the aspect lock is on', () => {
    const gesture = beginTransformGesture('se', make(), W, H, { x: W, y: H }, {
      aspectLocked: true
    });
    const next = applyTransformGesture(gesture, { x: 3 * W, y: H + 1 });
    close(next.scaleX, next.scaleY, 1e-6);
  });

  it('lets Shift free a locked aspect and lock a free one', () => {
    const locked = beginTransformGesture('se', make(), W, H, { x: W, y: H }, {
      aspectLocked: true
    });
    const freed = applyTransformGesture(locked, { x: 3 * W, y: H }, { shift: true, alt: false });
    expect(Math.abs(freed.scaleX - freed.scaleY)).toBeGreaterThan(0.5);

    const free = beginTransformGesture('se', make(), W, H, { x: W, y: H });
    const held = applyTransformGesture(free, { x: 3 * W, y: H }, { shift: true, alt: false });
    close(held.scaleX, held.scaleY, 1e-6);
  });

  it('scales along the layer axes rather than the screen axes when rotated', () => {
    const start = make({ rotationDegrees: 90 });
    const grip = transformPoint(start, W, H, handleLayerPoint('e', W, H));
    const anchorDoc = transformPoint(start, W, H, handleLayerPoint('w', W, H));
    const gesture = beginTransformGesture('e', start, W, H, grip);
    // Drag the handle twice as far from its anchor, along the direction the
    // rotation put it in.
    const pointer = {
      x: anchorDoc.x + (grip.x - anchorDoc.x) * 2,
      y: anchorDoc.y + (grip.y - anchorDoc.y) * 2
    };
    const next = applyTransformGesture(gesture, pointer);
    close(next.scaleX, 2, 1e-4);
    close(next.scaleY, 1, 1e-4);
    closePoint(transformPoint(next, W, H, handleLayerPoint('w', W, H)), anchorDoc, 1e-3);
  });

  it('mirrors rather than collapsing when a handle is dragged past its anchor', () => {
    const gesture = beginTransformGesture('e', make(), W, H, { x: W, y: H / 2 });
    const next = applyTransformGesture(gesture, { x: -W, y: H / 2 });
    expect(next.scaleX).toBeLessThan(0);
    // The layer is mirrored through the stored scale; the flip flag is the
    // user's own setting and is left alone.
    expect(next.flipHorizontal).toBe(false);
  });

  it('never produces a scale the renderer would refuse', () => {
    const gesture = beginTransformGesture('se', make(), W, H, { x: W, y: H });
    for (const pointer of [
      { x: 0, y: 0 },
      { x: 1e9, y: 1e9 },
      { x: -1e9, y: -1e9 },
      { x: 0.0001, y: 0.0001 }
    ]) {
      const next = applyTransformGesture(gesture, pointer);
      expect(Math.abs(next.scaleX)).toBeGreaterThanOrEqual(MIN_LAYER_SCALE);
      expect(Math.abs(next.scaleX)).toBeLessThanOrEqual(MAX_LAYER_SCALE);
      expect(Math.abs(next.scaleY)).toBeGreaterThanOrEqual(MIN_LAYER_SCALE);
      expect(Math.abs(next.scaleY)).toBeLessThanOrEqual(MAX_LAYER_SCALE);
      expect(Number.isFinite(next.translateX)).toBe(true);
      expect(Number.isFinite(next.translateY)).toBe(true);
    }
  });

  it('keeps an existing flip through a scale drag', () => {
    const start = make({ flipHorizontal: true });
    const gesture = beginTransformGesture('se', start, W, H, { x: 0, y: H });
    const next = applyTransformGesture(gesture, { x: -W, y: 2 * H });
    expect(next.flipHorizontal).toBe(true);
    close(Math.abs(next.scaleX), 2, 1e-6);
  });
});

describe('rotate gestures', () => {
  it('turns the layer by the angle the pointer swept around the centre', () => {
    const centre = { x: W / 2, y: H / 2 };
    const gesture = beginTransformGesture('rotate', make(), W, H, {
      x: centre.x,
      y: centre.y - 40
    });
    const next = applyTransformGesture(gesture, { x: centre.x + 40, y: centre.y });
    close(next.rotationDegrees, 90, 1e-4);
  });

  it('snaps to fifteen degree steps while Shift is held', () => {
    const centre = { x: W / 2, y: H / 2 };
    const gesture = beginTransformGesture('rotate', make(), W, H, {
      x: centre.x,
      y: centre.y - 40
    });
    const pointer = {
      x: centre.x + 40 * Math.cos((-38 * Math.PI) / 180),
      y: centre.y + 40 * Math.sin((-38 * Math.PI) / 180)
    };
    const next = applyTransformGesture(gesture, pointer, { shift: true, alt: false });
    expect(next.rotationDegrees % 15).toBeCloseTo(0, 6);
  });

  it('leaves the centre exactly where it was', () => {
    const start = make({ translateX: 11, translateY: -4 });
    const centre = transformPoint(start, W, H, { x: W / 2, y: H / 2 });
    const gesture = beginTransformGesture('rotate', start, W, H, { x: centre.x, y: centre.y - 40 });
    const next = applyTransformGesture(gesture, { x: centre.x + 13, y: centre.y + 29 });
    closePoint(transformPoint(next, W, H, { x: W / 2, y: H / 2 }), centre, 1e-6);
  });

  it('stays inside the range the renderer accepts across many turns', () => {
    const centre = { x: W / 2, y: H / 2 };
    const gesture = beginTransformGesture('rotate', make({ rotationDegrees: 350 }), W, H, {
      x: centre.x,
      y: centre.y - 40
    });
    const next = applyTransformGesture(gesture, { x: centre.x + 40, y: centre.y });
    expect(Math.abs(next.rotationDegrees)).toBeLessThanOrEqual(360);
  });
});

describe('keyboard and flips', () => {
  it('nudges by whole document pixels', () => {
    expect(nudgeTransform(make(), -1, 0).translateX).toBe(-1);
    expect(nudgeTransform(make({ translateY: 4 }), 0, 10).translateY).toBe(14);
  });

  it('toggles each flip independently', () => {
    const once = flipTransform(make(), 'horizontal');
    expect(once.flipHorizontal).toBe(true);
    expect(once.flipVertical).toBe(false);
    expect(flipTransform(once, 'horizontal').flipHorizontal).toBe(false);
    expect(flipTransform(once, 'vertical').flipVertical).toBe(true);
  });

  it('resets to an identity placement while keeping the sampling choice', () => {
    const reset = resetTransform('nearest');
    expect(isIdentityTransform(reset)).toBe(true);
    expect(reset.interpolation).toBe('nearest');
  });

  it('does not call a moved layer untransformed', () => {
    expect(isIdentityTransform(make())).toBe(true);
    expect(isIdentityTransform(make({ translateX: 1 }))).toBe(false);
    expect(isIdentityTransform(make({ flipVertical: true }))).toBe(false);
    // Sampling alone moves no pixels, so it is not part of the placement.
    expect(isIdentityTransform(make({ interpolation: 'nearest' }))).toBe(true);
  });

  it('turns the resize cursor to follow the layer', () => {
    expect(handleCursor('n')).toBe('ns-resize');
    expect(handleCursor('e')).toBe('ew-resize');
    expect(handleCursor('nw')).toBe('nwse-resize');
    // Quarter-turned, the north handle points east and must say so.
    expect(handleCursor('n', 90)).toBe('ew-resize');
    expect(handleCursor('nw', 90)).toBe('nesw-resize');
    expect(handleCursor('move')).toBe('move');
    expect(handleCursor('rotate')).toBe('grab');
    expect(handleCursor('n', Number.NaN)).toBe('ns-resize');
  });
});

describe('numeric fields', () => {
  it('reports position as the layer centre and size as the scaled extent', () => {
    const metrics = transformMetrics(make({ translateX: 10, scaleX: 2, scaleY: 0.5 }), W, H);
    expect(metrics.centerX).toBe(W / 2 + 10);
    expect(metrics.centerY).toBe(H / 2);
    expect(metrics.width).toBe(W * 2);
    expect(metrics.height).toBe(H * 0.5);
  });

  it('moves the layer when the centre is typed in', () => {
    const next = withMetric(make(), W, H, 'centerX', 200);
    expect(next.translateX).toBe(200 - W / 2);
    expect(transformMetrics(next, W, H).centerX).toBe(200);
  });

  it('rescales when a width is typed in', () => {
    const next = withMetric(make(), W, H, 'width', 250);
    close(next.scaleX, 2.5, 1e-9);
    close(next.scaleY, 1, 1e-9);
  });

  it('carries the ratio to the other axis when proportions are locked', () => {
    const next = withMetric(make(), W, H, 'width', 200, true);
    close(next.scaleX, 2, 1e-9);
    close(next.scaleY, 2, 1e-9);
  });

  it('refuses a value that is not a number', () => {
    const start = make({ scaleX: 1.5 });
    expect(withMetric(start, W, H, 'width', Number.NaN)).toEqual(start);
  });

  it('repairs a typed value the renderer could not use', () => {
    const huge = withMetric(make(), W, H, 'width', 1e12);
    expect(Math.abs(huge.scaleX)).toBeLessThanOrEqual(MAX_LAYER_SCALE);
    const zero = withMetric(make(), W, H, 'height', 0);
    expect(Math.abs(zero.scaleY)).toBeGreaterThanOrEqual(MIN_LAYER_SCALE);
    const spun = withMetric(make(), W, H, 'rotationDegrees', 1000);
    expect(Math.abs(spun.rotationDegrees)).toBeLessThanOrEqual(360);
  });

  it('round-trips every metric it reports', () => {
    const start = make({ translateX: -13, translateY: 7, scaleX: 1.4, scaleY: 0.8 });
    const metrics = transformMetrics(start, W, H);
    let rebuilt = make();
    rebuilt = withMetric(rebuilt, W, H, 'width', metrics.width);
    rebuilt = withMetric(rebuilt, W, H, 'height', metrics.height);
    rebuilt = withMetric(rebuilt, W, H, 'centerX', metrics.centerX);
    rebuilt = withMetric(rebuilt, W, H, 'centerY', metrics.centerY);
    const again = transformMetrics(rebuilt, W, H);
    close(again.centerX, metrics.centerX, 1e-9);
    close(again.centerY, metrics.centerY, 1e-9);
    close(again.width, metrics.width, 1e-9);
    close(again.height, metrics.height, 1e-9);
  });
});
