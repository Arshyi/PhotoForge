import type { BaseEditOperation, EditOperation, ImageMetadata } from '../types/editor';
import type { MaskSnapshot } from '../selections/types';

/// Mirrors `layers::LAYER_SCHEMA_VERSION`.
export const LAYER_SCHEMA_VERSION = 1;
/// Mirrors `layers::MAX_LAYERS`.
export const MAX_LAYERS = 512;
/// Mirrors `layers::MAX_GROUP_DEPTH`.
export const MAX_GROUP_DEPTH = 16;
export const MAX_LAYER_NAME_CHARS = 120;

export type BlendMode =
  | 'normal'
  | 'multiply'
  | 'screen'
  | 'overlay'
  | 'darken'
  | 'lighten'
  | 'color_dodge'
  | 'color_burn'
  | 'soft_light'
  | 'hard_light'
  | 'difference'
  | 'exclusion'
  | 'hue'
  | 'saturation'
  | 'color'
  | 'luminosity';

export const blendModes: { id: BlendMode; label: string; group: string }[] = [
  { id: 'normal', label: 'Normal', group: 'Normal' },
  { id: 'multiply', label: 'Multiply', group: 'Darken' },
  { id: 'darken', label: 'Darken', group: 'Darken' },
  { id: 'color_burn', label: 'Color Burn', group: 'Darken' },
  { id: 'screen', label: 'Screen', group: 'Lighten' },
  { id: 'lighten', label: 'Lighten', group: 'Lighten' },
  { id: 'color_dodge', label: 'Color Dodge', group: 'Lighten' },
  { id: 'overlay', label: 'Overlay', group: 'Contrast' },
  { id: 'soft_light', label: 'Soft Light', group: 'Contrast' },
  { id: 'hard_light', label: 'Hard Light', group: 'Contrast' },
  { id: 'difference', label: 'Difference', group: 'Comparative' },
  { id: 'exclusion', label: 'Exclusion', group: 'Comparative' },
  { id: 'hue', label: 'Hue', group: 'Component' },
  { id: 'saturation', label: 'Saturation', group: 'Component' },
  { id: 'color', label: 'Color', group: 'Component' },
  { id: 'luminosity', label: 'Luminosity', group: 'Component' }
];

export type LayerKind = 'pixel' | 'group' | 'adjustment';

/** Mirrors `layers::LayerInterpolation`. */
export type LayerInterpolation = 'bilinear' | 'nearest';

export const interpolationModes: { id: LayerInterpolation; label: string; hint: string }[] = [
  { id: 'bilinear', label: 'Smooth', hint: 'Blends neighbouring pixels. Best for photographs.' },
  { id: 'nearest', label: 'Hard edge', hint: 'Copies the nearest pixel. Best for pixel art and screenshots.' }
];

export interface LayerTransform {
  translateX: number;
  translateY: number;
  scaleX: number;
  scaleY: number;
  rotationDegrees: number;
  flipHorizontal: boolean;
  flipVertical: boolean;
  /** Absent in projects written before 0.8.2, which always sampled bilinearly. */
  interpolation: LayerInterpolation;
}

export const identityTransform: LayerTransform = {
  translateX: 0,
  translateY: 0,
  scaleX: 1,
  scaleY: 1,
  rotationDegrees: 0,
  flipHorizontal: false,
  flipVertical: false,
  interpolation: 'bilinear'
};

export interface LayerMask {
  snapshot: MaskSnapshot;
  enabled: boolean;
  inverted: boolean;
}

export interface LayerMetadata {
  createdAt: string;
  modifiedAt: string;
  custom: Record<string, string>;
}

export type LayerContent =
  | { type: 'pixel'; pixelId: string; width: number; height: number }
  | { type: 'group'; children: Layer[]; isolated: boolean }
  | { type: 'adjustment'; operation: BaseEditOperation };

export interface Layer {
  id: string;
  name: string;
  visible: boolean;
  locked: boolean;
  opacity: number;
  blendMode: BlendMode;
  transform: LayerTransform;
  mask: LayerMask | null;
  collapsed: boolean;
  metadata: LayerMetadata;
  content: LayerContent;
}

export interface LayerDocument {
  schemaVersion: typeof LAYER_SCHEMA_VERSION;
  canvasWidth: number;
  canvasHeight: number;
  /** Index 0 is the bottom of the stack; the panel displays this reversed. */
  layers: Layer[];
  activeLayerId: string | null;
}

/** What a keyboard or tool gesture currently edits. */
export type EditTarget = 'layer' | 'mask' | 'selection';

/** Where a global adjustment goes when the user changes a slider. */
export type AdjustmentTarget = 'document' | 'layer' | 'adjustmentLayer';

export interface LayerPixelsResult {
  pixelId: string;
  width: number;
  height: number;
  filename: string | null;
}

export interface LayerThumbnailResult {
  layerId: string;
  dataUrl: string;
  width: number;
  height: number;
}

export interface LayerMaskResult {
  snapshot: MaskSnapshot;
  width: number;
  height: number;
}

export interface LayerStoreReport {
  buffers: number;
  released: number;
  bytes: number;
}

export interface ProjectSaveResult {
  outputPath: string;
  bytes: number;
  processingTimeMs: number;
}

export interface ProjectLoadResult {
  documentId: number;
  isCurrent: boolean;
  metadata: ImageMetadata;
  originalPreviewDataUrl: string;
  previewDataUrl: string;
  document: LayerDocument;
  operations: EditOperation[];
  canvasWidth: number;
  canvasHeight: number;
  applicationVersion: string;
  createdAt: string;
  modifiedAt: string;
  processingTimeMs: number;
}

/** A row rendered by the Layers panel, flattened top-first with indentation. */
export interface LayerRow {
  layer: Layer;
  depth: number;
  parentId: string | null;
  index: number;
  /** Hidden because an ancestor group is collapsed or hidden. */
  hiddenByAncestor: boolean;
}

export const layerKindLabels: Record<LayerKind, string> = {
  pixel: 'Pixel layer',
  group: 'Group',
  adjustment: 'Adjustment layer'
};

export const layerKindIcons: Record<LayerKind, string> = {
  pixel: '▣',
  group: '▤',
  adjustment: '◐'
};

/** Actions the Layers panel raises for the host application to carry out. */
export type LayerPanelAction =
  | 'duplicate'
  | 'delete'
  | 'group'
  | 'ungroup'
  | 'merge_down'
  | 'flatten'
  | 'mask_white'
  | 'mask_black'
  | 'mask_from_selection'
  | 'mask_invert'
  | 'mask_toggle'
  | 'mask_delete'
  | 'mask_apply'
  | 'mask_load_selection'
  | 'edit_adjustment'
  | 'reset_transform'
  | 'rasterize_transform'
  | 'toggle_pass_through'
  | 'flip_horizontal'
  | 'flip_vertical'
  | 'transform_mode';
