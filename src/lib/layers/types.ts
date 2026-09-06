import type { BaseEditOperation, EditOperation, ImageMetadata, RawLayerSource } from '../types/editor';
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

export type LayerKind = 'pixel' | 'group' | 'adjustment' | 'shape' | 'text';

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

/** A colour in the document's linear working space, with straight alpha. */
export interface ShapeColor {
  red: number;
  green: number;
  blue: number;
  alpha: number;
}

export type LineCap = 'butt' | 'round' | 'square';
export type LineJoin = 'round' | 'bevel' | 'miter';

export interface StrokeStyle {
  width: number;
  cap: LineCap;
  join: LineJoin;
  miterLimit: number;
}

/** One step of a path. Mirrors `vector::PathCommand`. */
export type PathCommand =
  | { type: 'moveTo'; x: number; y: number }
  | { type: 'lineTo'; x: number; y: number }
  | { type: 'cubicTo'; c1x: number; c1y: number; c2x: number; c2y: number; x: number; y: number }
  | { type: 'close' };

/**
 * A shape that keeps its meaning. A rectangle stays a rectangle rather than
 * four points, so changing its corner radius later edits a parameter instead of
 * rebuilding geometry from its remains.
 */
export type ShapeGeometry =
  | { type: 'rectangle'; x: number; y: number; width: number; height: number; cornerRadius: number }
  | { type: 'ellipse'; cx: number; cy: number; rx: number; ry: number }
  | { type: 'line'; x1: number; y1: number; x2: number; y2: number }
  | { type: 'polygon'; cx: number; cy: number; radius: number; sides: number; rotationDegrees: number }
  | {
      type: 'star';
      cx: number;
      cy: number;
      outerRadius: number;
      innerRadius: number;
      points: number;
      rotationDegrees: number;
    }
  | { type: 'path'; path: { commands: PathCommand[] } };

export type FillRule = 'nonZero' | 'evenOdd';

/** Mirrors `layers::shape::ShapeContent`, flattened into the layer content. */
export interface ShapeContent {
  geometry: ShapeGeometry;
  fill?: ShapeColor | null;
  stroke?: ShapeColor | null;
  strokeStyle?: StrokeStyle | null;
  fillRule: FillRule;
}

export type TextAlign = 'start' | 'center' | 'end' | 'justified';

/**
 * Mirrors `layers::text::TextContent`, flattened into the layer content.
 *
 * `fontFamily` is what the user asked for and stays what they asked for. When
 * the machine lacks it the text draws in a substitute and is reported as
 * missing; the request is never rewritten, so opening the project somewhere
 * that has the font restores the intended setting.
 */
export interface TextContent {
  text: string;
  fontFamily: string;
  fontSize: number;
  fontWeight: number;
  italic: boolean;
  align: TextAlign;
  /** Line advance as a multiple of the font size. */
  lineHeight: number;
  letterSpacing: number;
  originX: number;
  originY: number;
  /** Present for area text; absent for point text on a single line. */
  wrapWidth?: number | null;
  fill: ShapeColor;
  stroke?: ShapeColor | null;
  strokeStyle?: StrokeStyle | null;
}

export type LayerContent =
  | { type: 'pixel'; pixelId: string; width: number; height: number }
  | { type: 'group'; children: Layer[]; isolated: boolean }
  | { type: 'adjustment'; operation: BaseEditOperation }
  | ({ type: 'shape' } & ShapeContent)
  | ({ type: 'text' } & TextContent);

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
  /**
   * The camera file this layer was developed from, when it came from one.
   * Absent on every other layer and in projects written before 0.9.0.
   */
  raw?: RawLayerSource | null;
  content: LayerContent;
}

export interface LayerDocument {
  schemaVersion: typeof LAYER_SCHEMA_VERSION;
  /** Missing metadata denotes the original encoded-sRGB compositor. */
  precision?: 'legacy_srgb8' | 'linear_srgb_f32';
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
  raw?: RawLayerSource | null;
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
  adjustment: 'Adjustment layer',
  shape: 'Shape layer',
  text: 'Text layer'
};

export const layerKindIcons: Record<LayerKind, string> = {
  pixel: '▣',
  group: '▤',
  adjustment: '◐',
  shape: '◇',
  text: 'T'
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
  | 'edit_text'
  | 'rasterize_semantic'
  | 'reset_transform'
  | 'rasterize_transform'
  | 'toggle_pass_through'
  | 'flip_horizontal'
  | 'flip_vertical'
  | 'transform_mode';
