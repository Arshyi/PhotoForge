export const MASK_FORMAT_VERSION = 1 as const;

export interface Point {
  x: number;
  y: number;
}

export interface MaskSnapshot {
  version: number;
  width: number;
  height: number;
  encoding: string;
  data: string;
  checksum: string;
}

export interface MaskDiagnostics {
  width: number;
  height: number;
  selectedPixels: number;
  fullySelectedPixels: number;
  averageCoverage: number;
  bounds: [number, number, number, number] | null;
  memoryBytes: number;
}

export type CompositionMode = 'replace' | 'add' | 'subtract' | 'intersect';
export type SelectionTool =
  | 'none'
  | 'rectangle'
  | 'ellipse'
  | 'freehand'
  | 'polygon'
  | 'brush'
  | 'eraser'
  | 'magic_wand'
  | 'color_range';
export type ApplyScope = 'global' | 'inside' | 'outside';
export type OverlayMode =
  | 'marching_ants'
  | 'color'
  | 'grayscale'
  | 'black'
  | 'white'
  | 'mask_only';

export interface OverlaySettings {
  visible: boolean;
  mode: OverlayMode;
  opacity: number;
  color: string;
}

export interface SelectionSettings {
  brushDiameter: number;
  brushHardness: number;
  brushOpacity: number;
  wandTolerance: number;
  wandConnectivity: 'four' | 'eight';
  wandAntiAlias: boolean;
  wandContiguous: boolean;
  sampleMerged: boolean;
  colorTolerance: number;
  luminanceSensitivity: number;
  hueSensitivity: number;
  saturationSensitivity: number;
  fixedAspect: boolean;
  fromCenter: boolean;
}

export interface NamedMask {
  id: string;
  name: string;
  mask: MaskSnapshot;
  visible: boolean;
  locked: boolean;
  createdAt: string;
  modifiedAt: string;
  sourceTool?: SelectionTool;
}

export interface SelectionState {
  schemaVersion: 1;
  documentKey: string;
  activeMask: MaskSnapshot | null;
  activeDiagnostics: MaskDiagnostics | null;
  namedMasks: NamedMask[];
  tool: SelectionTool;
  mode: CompositionMode;
  applyScope: ApplyScope;
  overlay: OverlaySettings;
  settings: SelectionSettings;
  panelCollapsed: boolean;
  updatedAt: string;
}

export type SelectionShape =
  | { type: 'rectangle'; start: Point; end: Point }
  | { type: 'ellipse'; start: Point; end: Point }
  | { type: 'polygon'; points: Point[] }
  | { type: 'freehand'; points: Point[] }
  | {
      type: 'brush';
      points: Point[];
      diameter: number;
      hardness: number;
      opacity: number;
    };

export type MaskOperation =
  | { type: 'select_all' }
  | { type: 'deselect' }
  | { type: 'invert' }
  | { type: 'feather'; radius: number }
  | { type: 'expand'; radius: number }
  | { type: 'contract'; radius: number }
  | { type: 'smooth'; radius: number }
  | { type: 'fill_holes' }
  | { type: 'remove_small_islands'; minimum_pixels: number }
  | { type: 'border'; width: number }
  | {
      type: 'refine';
      smooth: number;
      feather: number;
      contrast: number;
      shift_edge: number;
    };

export interface SelectionGesture {
  tool: SelectionTool;
  points: Point[];
  shiftKey: boolean;
  altKey: boolean;
}

export interface MaskResult {
  mask: MaskSnapshot;
  diagnostics: MaskDiagnostics;
  documentId: number;
  requestId: number;
  processingTimeMs: number;
  isCurrent: boolean;
}

export interface ColorRangeOptions {
  tolerance: number;
  luminanceSensitivity: number;
  hueSensitivity: number;
  saturationSensitivity: number;
}

export interface MaskFile {
  format: 'photoforge-mask';
  version: 1;
  id: string;
  name: string;
  mask: MaskSnapshot;
  metadata: {
    createdAt: string;
    modifiedAt: string;
    sourceTool?: string;
  };
}
