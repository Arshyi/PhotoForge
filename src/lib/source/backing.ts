/**
 * Finding the file a document's pixels are a region of.
 *
 * A region opened from an oversized source keeps its relationship to the file on
 * its layer: a `SourceOrigin` for a raster format, and the `view` of the RAW
 * source for a camera file. "Change Source Region" starts from whichever the
 * document has, so it reads both here, in one place, rather than the interface
 * knowing two shapes.
 *
 * This only *reads* the relationship. Whether the file is still the one recorded
 * is the backend's question (it hashes the file), and it is asked before anything
 * is offered.
 */
import type { LayerDocument } from '../layers/types';
import { activeLayer, eachLayer } from '../layers/tree';
import type { RawLayerSource } from '../types/editor';
import type { Rect, SourceOrigin } from './types';

export type SourceBacking =
  | {
      kind: 'file';
      path: string;
      filename: string;
      rect: Rect;
      sourceWidth: number;
      sourceHeight: number;
      origin: SourceOrigin;
    }
  | {
      kind: 'raw';
      path: string;
      filename: string;
      rect: Rect;
      sourceWidth: number;
      sourceHeight: number;
      source: RawLayerSource;
    };

function backingOfLayer(layer: ReturnType<typeof activeLayer>): SourceBacking | null {
  if (!layer) return null;
  const origin = layer.origin ?? null;
  if (origin && origin.view.view === 'region') {
    const name = origin.source.path.split(/[\/]/).pop() ?? origin.source.path;
    return {
      kind: 'file',
      path: origin.source.path,
      filename: name,
      rect: origin.view.rect,
      sourceWidth: origin.source.width,
      sourceHeight: origin.source.height,
      origin
    };
  }
  const raw = layer.raw ?? null;
  if (raw && raw.view && raw.mode.mode === 'linked') {
    return {
      kind: 'raw',
      path: raw.mode.path,
      filename: raw.reference.filename,
      rect: raw.view,
      sourceWidth: raw.reference.width,
      sourceHeight: raw.reference.height,
      source: raw
    };
  }
  return null;
}

/**
 * The region-of-a-file relationship of a document, if it has one.
 *
 * The active layer decides when it has one; otherwise the first layer that has
 * one does. A reduced copy is not a region and is not offered a region change:
 * its pixels are the whole file at lower resolution, and the way to change that
 * is to open the file again.
 */
export function sourceBackingOf(document: LayerDocument | null): SourceBacking | null {
  if (!document) return null;
  const active = backingOfLayer(activeLayer(document));
  if (active) return active;
  for (const layer of eachLayer(document)) {
    const found = backingOfLayer(layer);
    if (found) return found;
  }
  return null;
}
