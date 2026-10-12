import { childrenOf, layerAt, pathTo } from './tree';
import type { Layer, LayerDocument } from './types';

/**
 * Returns a user-facing reason when a merge request cannot be represented by
 * one ordinary pixel layer without changing the current composite.
 *
 * The renderer can flatten a contiguous sibling range exactly when its output
 * is independent of an omitted backdrop. Normal source-over layers have that
 * associativity; blend-dependent layers, adjustment layers, and pass-through
 * groups do not. Refusing the latter is safer than silently baking a different
 * image, especially for workflows replayed against a changed stack.
 *
 * Clipping is part of the image as well. See `clippingProblem`; the Rust engine
 * (`range_clipping` in `layers/clip.rs`) applies the same rule and is the authority.
 */
export function mergeSafetyProblem(document: LayerDocument, layerIds: string[]): string | null {
  if (layerIds.length === 0) return 'Merging requires at least one layer.';
  const paths = layerIds.map((id) => pathTo(document, id));
  if (paths.some((path) => path === null)) {
    const missing = layerIds.find((id, index) => paths[index] === null) ?? '';
    return `Layer ${missing} is missing.`;
  }
  const resolved = paths as number[][];
  const parentPath = resolved[0].slice(0, -1);
  if (resolved.some((path) => path.slice(0, -1).some((value, index) => value !== parentPath[index]) ||
    path.length !== parentPath.length + 1)) {
    return 'Merging requires layers from one sibling stack.';
  }
  const indices = resolved.map((path) => path[path.length - 1]).sort((a, b) => a - b);
  if (new Set(indices).size !== indices.length || indices.some((value, index) => index > 0 && value !== indices[index - 1] + 1)) {
    return 'Merging requires a contiguous sibling range.';
  }

  const parent = parentPath.length ? layerAt(document, parentPath) : null;
  const siblings = parent ? childrenOf(parent) : document.layers;
  const selected = indices.map((index) => siblings[index]).filter(Boolean) as Layer[];
  const clipProblem = clippingProblem(siblings, indices[0], indices[indices.length - 1]);
  if (clipProblem) return clipProblem;
  const omittedBackdrop = indices[0] > 0 || Boolean(parent?.content.type === 'group' && !parent.content.isolated);
  if (!omittedBackdrop) return null;

  const dependsOnBackdrop = selected.some((layer) =>
    layer.content.type === 'adjustment' ||
    layer.blendMode !== 'normal' ||
    (layer.content.type === 'group' && !layer.content.isolated)
  );
  return dependsOnBackdrop
    ? 'This merge depends on layers beneath the selection because of a blend mode, adjustment layer, or pass-through group. Include the full affected stack or switch those layers to Normal before merging.'
    : null;
}

/** Whether the layer at `index` is clipped to something beneath it. The first layer of a stack never is. */
function isClippedMember(siblings: Layer[], index: number): boolean {
  return index > 0 && siblings[index]?.clipped === true;
}

/**
 * Whether replacing the contiguous range `start..=end` of one stack with a single
 * layer keeps the clipping that is drawn. Merge Down and Convert to Smart Object
 * replace a range that way.
 *
 * A range inside a clipping group, every layer of it clipped, is replaced by a layer
 * clipped to the same base. A range that starts on a base is replaced by a base, and
 * the layers clipped above it follow it only if they were clipped to that same base.
 * Anything that would split a group is refused, as is leaving layers clipped to an
 * adjustment layer, which has no shape of its own, for a pixel layer.
 */
function clippingProblem(siblings: Layer[], start: number, end: number): string | null {
  const joinsGroup = isClippedMember(siblings, start);
  let oneGroup = true;
  for (let index = start + 1; index <= end; index += 1) {
    if (!isClippedMember(siblings, index)) oneGroup = false;
  }
  const leavesFollowers = isClippedMember(siblings, end + 1);
  if (!oneGroup && (joinsGroup || leavesFollowers)) {
    return 'This merge would split a clipping group (a layer and the layers clipped to it): work from the top of the group down, or release the clipping first.';
  }
  if (oneGroup && leavesFollowers && !joinsGroup && siblings[start]?.content.type === 'adjustment') {
    return 'Layers are clipped to an adjustment layer in this merge, and would be clipped to the merged layer instead: release the clipping first.';
  }
  return null;
}
