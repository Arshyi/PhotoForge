import type { LayerDocument } from '../layers/types';
import { activeLayer, eachLayer } from '../layers/tree';
import type { HostFact } from './types';

const KIND_NAMES: Record<string, string> = {
  pixel: 'Pixel',
  group: 'Group',
  adjustment: 'Adjustment',
  shape: 'Shape',
  text: 'Text',
  smart_object: 'Smart object'
};

/**
 * The value of one of the facts a plugin panel may show.
 *
 * The list of facts is closed (see `HostFact`): a panel names one of these and
 * nothing else, so a plugin that was granted `document.read` can learn only what is
 * written here. The values are computed in the interface, from the document it
 * already holds; no plugin code is involved.
 */
export function factValue(document: LayerDocument | null, fact: HostFact): string {
  if (!document) return 'No document open';
  const active = activeLayer(document);
  switch (fact) {
    case 'layer_count':
      return String(eachLayer(document).length);
    case 'pixel_layer_count':
      return String(eachLayer(document).filter((layer) => layer.content.type === 'pixel').length);
    case 'canvas_width':
      return `${document.canvasWidth} px`;
    case 'canvas_height':
      return `${document.canvasHeight} px`;
    case 'precision':
      return document.precision === 'linear_srgb_f32' ? 'Linear float' : 'Legacy 8-bit sRGB';
    case 'active_layer_name':
      return active?.name ?? 'None selected';
    case 'active_layer_kind':
      return active ? (KIND_NAMES[active.content.type] ?? active.content.type) : 'None selected';
    case 'active_layer_opacity':
      return active ? `${Math.round(active.opacity * 100)}%` : 'None selected';
  }
}
