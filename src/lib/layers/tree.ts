import type { BaseEditOperation } from '../types/editor';
import {
  identityTransform,
  LAYER_SCHEMA_VERSION,
  MAX_GROUP_DEPTH,
  MAX_LAYERS,
  MAX_LAYER_NAME_CHARS,
  type Layer,
  type LayerContent,
  type LayerDocument,
  type LayerKind,
  type LayerRow,
  type LayerTransform
} from './types';

const ID_ALPHABET = 'abcdefghijklmnopqrstuvwxyz0123456789';

/**
 * Layer identifiers are random rather than sequential so they stay stable and
 * collision-free across undo, duplication, and project round trips without the
 * frontend having to track a counter that a loaded project could invalidate.
 */
export function newLayerId(prefix = 'l'): string {
  const values = new Uint8Array(12);
  if (typeof globalThis.crypto?.getRandomValues === 'function') {
    globalThis.crypto.getRandomValues(values);
  } else {
    for (let index = 0; index < values.length; index += 1) {
      values[index] = Math.floor(Math.random() * 256);
    }
  }
  let id = prefix;
  for (const value of values) id += ID_ALPHABET[value % ID_ALPHABET.length];
  return id;
}

export function timestamp(now: Date = new Date()): string {
  return now.toISOString();
}

function metadata(now?: Date) {
  const stamp = timestamp(now);
  return { createdAt: stamp, modifiedAt: stamp, custom: {} };
}

function baseLayer(name: string, content: LayerContent, now?: Date): Layer {
  return {
    id: newLayerId(),
    name,
    visible: true,
    locked: false,
    opacity: 1,
    blendMode: 'normal',
    transform: { ...identityTransform },
    mask: null,
    collapsed: false,
    metadata: metadata(now),
    content
  };
}

export function createPixelLayer(
  name: string,
  pixelId: string,
  width: number,
  height: number,
  now?: Date
): Layer {
  return baseLayer(name, { type: 'pixel', pixelId, width, height }, now);
}

export function createGroupLayer(
  name: string,
  children: Layer[] = [],
  now?: Date,
  isolated = true
): Layer {
  return baseLayer(name, { type: 'group', children, isolated }, now);
}

/** True for a group whose children composite against the backdrop beneath it. */
export function isPassThrough(layer: Layer): boolean {
  return layer.content.type === 'group' && !layer.content.isolated;
}

export function createAdjustmentLayer(
  name: string,
  operation: BaseEditOperation,
  now?: Date
): Layer {
  return baseLayer(name, { type: 'adjustment', operation }, now);
}

export function createDocument(
  canvasWidth: number,
  canvasHeight: number,
  layers: Layer[] = []
): LayerDocument {
  return {
    schemaVersion: LAYER_SCHEMA_VERSION,
    canvasWidth,
    canvasHeight,
    layers,
    activeLayerId: layers.at(-1)?.id ?? null
  };
}

export function layerKind(layer: Layer): LayerKind {
  return layer.content.type;
}

export function childrenOf(layer: Layer): Layer[] {
  return layer.content.type === 'group' ? layer.content.children : [];
}

/** Depth-first, bottom-to-top enumeration matching the Rust traversal order. */
export function eachLayer(document: LayerDocument): Layer[] {
  const ordered: Layer[] = [];
  const stack = [...document.layers].reverse();
  while (stack.length) {
    const layer = stack.pop() as Layer;
    ordered.push(layer);
    const children = childrenOf(layer);
    for (let index = children.length - 1; index >= 0; index -= 1) stack.push(children[index]);
  }
  return ordered;
}

export function findLayer(document: LayerDocument, id: string): Layer | null {
  return eachLayer(document).find((layer) => layer.id === id) ?? null;
}

export function pathTo(document: LayerDocument, id: string): number[] | null {
  const stack: number[][] = document.layers.map((_, index) => [index]).reverse();
  while (stack.length) {
    const path = stack.pop() as number[];
    const layer = layerAt(document, path);
    if (!layer) continue;
    if (layer.id === id) return path;
    const children = childrenOf(layer);
    for (let index = children.length - 1; index >= 0; index -= 1) stack.push([...path, index]);
  }
  return null;
}

export function layerAt(document: LayerDocument, path: number[]): Layer | null {
  let layers = document.layers;
  let current: Layer | null = null;
  for (const step of path) {
    current = layers[step] ?? null;
    if (!current) return null;
    layers = childrenOf(current);
  }
  return current;
}

export function parentOf(document: LayerDocument, id: string): string | null {
  const path = pathTo(document, id);
  if (!path || path.length < 2) return null;
  return layerAt(document, path.slice(0, -1))?.id ?? null;
}

export function referencedPixelIds(document: LayerDocument): string[] {
  const ids = new Set<string>();
  for (const layer of eachLayer(document)) {
    if (layer.content.type === 'pixel') ids.add(layer.content.pixelId);
  }
  return [...ids].sort();
}

export function isSelfOrDescendant(
  document: LayerDocument,
  ancestorId: string,
  candidateId: string
): boolean {
  if (ancestorId === candidateId) return true;
  const ancestor = findLayer(document, ancestorId);
  if (!ancestor) return false;
  const stack = [...childrenOf(ancestor)];
  while (stack.length) {
    const layer = stack.pop() as Layer;
    if (layer.id === candidateId) return true;
    stack.push(...childrenOf(layer));
  }
  return false;
}

/**
 * Rebuilds the branch that leads to `path` and reuses every untouched subtree
 * by reference, so an edit to one layer never deep-copies the rest of the tree.
 */
function replaceAt(layers: Layer[], path: number[], replacement: Layer | null): Layer[] {
  const [index, ...rest] = path;
  const next = [...layers];
  if (rest.length === 0) {
    if (replacement === null) next.splice(index, 1);
    else next[index] = replacement;
    return next;
  }
  const parent = next[index];
  if (!parent || parent.content.type !== 'group') return layers;
  next[index] = {
    ...parent,
    content: {
      type: 'group',
      children: replaceAt(parent.content.children, rest, replacement),
      isolated: parent.content.isolated
    }
  };
  return next;
}

function insertAt(layers: Layer[], path: number[], index: number, layer: Layer): Layer[] {
  if (path.length === 0) {
    const next = [...layers];
    next.splice(Math.max(0, Math.min(index, next.length)), 0, layer);
    return next;
  }
  const [step, ...rest] = path;
  const next = [...layers];
  const parent = next[step];
  if (!parent || parent.content.type !== 'group') return layers;
  next[step] = {
    ...parent,
    content: {
      type: 'group',
      children: insertAt(parent.content.children, rest, index, layer),
      isolated: parent.content.isolated
    }
  };
  return next;
}

export function updateLayer(
  document: LayerDocument,
  id: string,
  updater: (layer: Layer) => Layer,
  now?: Date
): LayerDocument {
  const path = pathTo(document, id);
  if (!path) return document;
  const existing = layerAt(document, path);
  if (!existing) return document;
  const updated = updater(existing);
  const stamped: Layer = {
    ...updated,
    metadata: { ...updated.metadata, modifiedAt: timestamp(now) }
  };
  return { ...document, layers: replaceAt(document.layers, path, stamped) };
}

export function insertLayer(
  document: LayerDocument,
  layer: Layer,
  parentId: string | null,
  index: number
): LayerDocument {
  if (findLayer(document, layer.id)) return document;
  const parentPath = parentId ? pathTo(document, parentId) : [];
  if (parentPath === null) return document;
  if (parentId) {
    const parent = findLayer(document, parentId);
    if (!parent || parent.content.type !== 'group') return document;
  }
  return {
    ...document,
    layers: insertAt(document.layers, parentPath, index, layer),
    activeLayerId: layer.id
  };
}

export function removeLayer(document: LayerDocument, id: string): LayerDocument {
  const path = pathTo(document, id);
  if (!path) return document;
  const layers = replaceAt(document.layers, path, null);
  const removedIds = new Set(collectIds(findLayer(document, id)));
  const activeLayerId =
    document.activeLayerId && removedIds.has(document.activeLayerId)
      ? null
      : document.activeLayerId;
  return { ...document, layers, activeLayerId };
}

function collectIds(layer: Layer | null): string[] {
  if (!layer) return [];
  const ids: string[] = [];
  const stack = [layer];
  while (stack.length) {
    const current = stack.pop() as Layer;
    ids.push(current.id);
    stack.push(...childrenOf(current));
  }
  return ids;
}

/**
 * Moves a layer under a new parent. A group may never be moved into itself or
 * one of its own descendants; that check runs before anything is detached, so a
 * rejected move leaves the tree exactly as it was.
 */
export function moveLayer(
  document: LayerDocument,
  id: string,
  parentId: string | null,
  index: number
): LayerDocument {
  const layer = findLayer(document, id);
  if (!layer) return document;
  if (parentId) {
    if (isSelfOrDescendant(document, id, parentId)) return document;
    const parent = findLayer(document, parentId);
    if (!parent || parent.content.type !== 'group') return document;
  }
  if (depthOf(document, parentId) + depthOfSubtree(layer) > MAX_GROUP_DEPTH) return document;

  const active = document.activeLayerId;
  const detached = removeLayer(document, id);
  const parentPath = parentId ? pathTo(detached, parentId) : [];
  if (parentPath === null) return document;
  const inserted = {
    ...detached,
    layers: insertAt(detached.layers, parentPath, index, layer)
  };
  return { ...inserted, activeLayerId: active };
}

function depthOf(document: LayerDocument, parentId: string | null): number {
  if (!parentId) return 0;
  return pathTo(document, parentId)?.length ?? 0;
}

function depthOfSubtree(layer: Layer): number {
  let deepest = 1;
  const stack: { layer: Layer; depth: number }[] = [{ layer, depth: 1 }];
  while (stack.length) {
    const current = stack.pop() as { layer: Layer; depth: number };
    deepest = Math.max(deepest, current.depth);
    for (const child of childrenOf(current.layer)) {
      stack.push({ layer: child, depth: current.depth + 1 });
    }
  }
  return deepest;
}

/**
 * Duplicates a layer and its whole subtree with fresh identifiers. Pixel buffer
 * references are reused deliberately: buffers are immutable, so a duplicate
 * costs no extra pixel memory until one of the copies is edited.
 */
export function duplicateLayer(
  document: LayerDocument,
  id: string,
  now?: Date
): { document: LayerDocument; layer: Layer | null } {
  const original = findLayer(document, id);
  if (!original) return { document, layer: null };
  const copy = withNewIds(original, `${original.name} copy`, now);
  const path = pathTo(document, id);
  if (!path) return { document, layer: null };
  const parentPath = path.slice(0, -1);
  const index = path[path.length - 1] + 1;
  return {
    document: {
      ...document,
      layers: insertAt(document.layers, parentPath, index, copy),
      activeLayerId: copy.id
    },
    layer: copy
  };
}

function withNewIds(layer: Layer, name: string | null, now?: Date): Layer {
  const stamp = timestamp(now);
  const content: LayerContent =
    layer.content.type === 'group'
      ? {
          type: 'group',
          children: layer.content.children.map((child) => withNewIds(child, null, now)),
          isolated: layer.content.isolated
        }
      : layer.content;
  return {
    ...layer,
    id: newLayerId(),
    name: name ?? layer.name,
    content,
    metadata: { ...layer.metadata, createdAt: stamp, modifiedAt: stamp }
  };
}

/**
 * Wraps the chosen layers in a new group, placed where the topmost of them sat.
 * Only siblings can be grouped, which keeps the operation unambiguous.
 */
export function groupLayers(
  document: LayerDocument,
  ids: string[],
  name = 'Group',
  now?: Date
): { document: LayerDocument; group: Layer | null } {
  if (ids.length === 0) return { document, group: null };
  const paths = ids.map((id) => pathTo(document, id));
  if (paths.some((path) => path === null)) return { document, group: null };
  const parents = new Set(paths.map((path) => (path as number[]).slice(0, -1).join('/')));
  if (parents.size !== 1) return { document, group: null };

  const parentPath = (paths[0] as number[]).slice(0, -1);
  const indices = (paths as number[][]).map((path) => path[path.length - 1]).sort((a, b) => a - b);
  const ordered = indices.map((index) => layerAt(document, [...parentPath, index])).filter(Boolean) as Layer[];
  const parentId = parentPath.length ? layerAt(document, parentPath)?.id ?? null : null;
  if (depthOf(document, parentId) + 1 + Math.max(...ordered.map(depthOfSubtree)) > MAX_GROUP_DEPTH) {
    return { document, group: null };
  }

  let next = document;
  for (const id of ids) next = removeLayer(next, id);
  const group = createGroupLayer(name, ordered, now);
  const insertionPath = parentId ? pathTo(next, parentId) : [];
  if (insertionPath === null) return { document, group: null };
  return {
    document: {
      ...next,
      layers: insertAt(next.layers, insertionPath, indices[0], group),
      activeLayerId: group.id
    },
    group
  };
}

/** Replaces a group with its children, in place and in order. */
export function ungroupLayer(document: LayerDocument, id: string): LayerDocument {
  const group = findLayer(document, id);
  if (!group || group.content.type !== 'group') return document;
  const path = pathTo(document, id);
  if (!path) return document;
  const children = group.content.children;
  const parentPath = path.slice(0, -1);
  const index = path[path.length - 1];

  let layers = replaceAt(document.layers, path, null);
  children.forEach((child, offset) => {
    layers = insertAt(layers, parentPath, index + offset, child);
  });
  return {
    ...document,
    layers,
    activeLayerId: children.at(-1)?.id ?? document.activeLayerId
  };
}

/**
 * Flattens the tree for display: top layer first, with indentation, skipping the
 * children of collapsed groups.
 */
export function displayRows(document: LayerDocument): LayerRow[] {
  const rows: LayerRow[] = [];
  const walk = (
    layers: Layer[],
    depth: number,
    parentId: string | null,
    hiddenByAncestor: boolean
  ) => {
    for (let index = layers.length - 1; index >= 0; index -= 1) {
      const layer = layers[index];
      rows.push({ layer, depth, parentId, index, hiddenByAncestor });
      if (layer.content.type === 'group' && !layer.collapsed) {
        walk(
          layer.content.children,
          depth + 1,
          layer.id,
          hiddenByAncestor || !layer.visible
        );
      }
    }
  };
  walk(document.layers, 0, null, false);
  return rows;
}

/** True when a transform places the layer exactly on its own grid. */
function isIdentityPlacement(transform: LayerTransform): boolean {
  return (
    transform.translateX === identityTransform.translateX &&
    transform.translateY === identityTransform.translateY &&
    transform.scaleX === identityTransform.scaleX &&
    transform.scaleY === identityTransform.scaleY &&
    transform.rotationDegrees === identityTransform.rotationDegrees &&
    !transform.flipHorizontal &&
    !transform.flipVertical
  );
}

/**
 * True when the document is a single ordinary full-canvas pixel layer with no
 * mask, transform, or blending. PhotoForge keeps using the original Phase 7.1
 * render path in that case, so opening and editing a normal photo behaves
 * exactly as it did before layers existed.
 */
export function isSimpleDocument(document: LayerDocument): boolean {
  if (document.layers.length !== 1) return false;
  const layer = document.layers[0];
  if (layer.content.type !== 'pixel') return false;
  if (!layer.visible || layer.opacity !== 1 || layer.blendMode !== 'normal') return false;
  if (layer.mask) return false;
  // Compared field by field rather than by serialising: the transform gained an
  // optional interpolation mode in 0.8.2, and a mode chosen on an untransformed
  // layer changes no pixels, so it must not force the layered render path.
  if (!isIdentityPlacement(layer.transform)) return false;
  return (
    layer.content.width === document.canvasWidth &&
    layer.content.height === document.canvasHeight
  );
}

export function countLayers(document: LayerDocument): number {
  return eachLayer(document).length;
}

export function activeLayer(document: LayerDocument): Layer | null {
  return document.activeLayerId ? findLayer(document, document.activeLayerId) : null;
}

/**
 * Client-side validation mirroring the Rust checks. It gives immediate feedback
 * and stops obviously broken trees from reaching the backend; Rust still
 * revalidates everything at the trust boundary.
 */
export function validateDocument(document: LayerDocument): string[] {
  const problems: string[] = [];
  if (document.schemaVersion !== LAYER_SCHEMA_VERSION) {
    problems.push(`Unsupported layer schema version ${document.schemaVersion}.`);
  }
  if (document.canvasWidth <= 0 || document.canvasHeight <= 0) {
    problems.push('The canvas must be larger than zero pixels.');
  }

  const seen = new Set<string>();
  const stack: { layer: Layer; depth: number }[] = document.layers
    .map((layer) => ({ layer, depth: 1 }))
    .reverse();
  let count = 0;
  while (stack.length) {
    const { layer, depth } = stack.pop() as { layer: Layer; depth: number };
    count += 1;
    if (depth > MAX_GROUP_DEPTH) {
      problems.push(`Groups are nested deeper than the limit of ${MAX_GROUP_DEPTH}.`);
      break;
    }
    if (count > MAX_LAYERS) {
      problems.push(`A document may contain at most ${MAX_LAYERS} layers.`);
      break;
    }
    if (seen.has(layer.id)) problems.push(`Two layers share the identifier ${layer.id}.`);
    seen.add(layer.id);
    if (!layer.name.trim() || layer.name.length > MAX_LAYER_NAME_CHARS) {
      problems.push(`${layer.id} has an invalid name.`);
    }
    if (!Number.isFinite(layer.opacity) || layer.opacity < 0 || layer.opacity > 1) {
      problems.push(`${layer.name} has an opacity outside 0 to 1.`);
    }
    if (!validTransform(layer.transform)) {
      problems.push(`${layer.name} has an invalid transform.`);
    }
    for (const child of childrenOf(layer)) stack.push({ layer: child, depth: depth + 1 });
  }

  if (document.activeLayerId && !seen.has(document.activeLayerId)) {
    problems.push('The selected layer no longer exists.');
  }
  return problems;
}

function validTransform(transform: LayerTransform): boolean {
  const numbers = [
    transform.translateX,
    transform.translateY,
    transform.scaleX,
    transform.scaleY,
    transform.rotationDegrees
  ];
  if (!numbers.every((value) => Number.isFinite(value))) return false;
  const scaleInRange = (value: number) => {
    const magnitude = Math.abs(value);
    return magnitude >= 1 / 64 && magnitude <= 64;
  };
  // Mirrors `layers::transform::MAX_LAYER_TRANSLATION`. Without this the panel
  // would happily commit a placement the renderer then refuses to draw.
  const translationInRange = (value: number) => Math.abs(value) <= 1_000_000;
  return (
    scaleInRange(transform.scaleX) &&
    scaleInRange(transform.scaleY) &&
    translationInRange(transform.translateX) &&
    translationInRange(transform.translateY) &&
    Math.abs(transform.rotationDegrees) <= 360
  );
}
