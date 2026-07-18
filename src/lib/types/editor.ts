export type EditOperation =
  | { type: 'brightness'; amount: number }
  | { type: 'contrast'; amount: number }
  | { type: 'saturation'; amount: number }
  | { type: 'gamma'; value: number }
  | { type: 'grayscale' }
  | { type: 'sepia' }
  | { type: 'reflect_horizontal' }
  | { type: 'rotate'; degrees: number }
  | { type: 'gaussian_blur'; radius: number }
  | { type: 'sharpen'; strength: number }
  | { type: 'auto_white_balance'; strength: number }
  | { type: 'local_contrast'; strength: number; tile_size: number; clip_limit: number }
  | { type: 'denoise'; strength: number; preserve_edges: number }
  | { type: 'deblock'; strength: number }
  | { type: 'edge_aware_sharpen'; strength: number; radius: number; threshold: number }
  | { type: 'mild_deblur'; strength: number; radius: number }
  | { type: 'document_enhance'; strength: number; grayscale: boolean }
  | { type: 'uneven_lighting_correction'; strength: number; radius: number };

export type OperationType = EditOperation['type'];

export interface ImageMetadata {
  filename: string;
  width: number;
  height: number;
  format: string;
  fileSize: number;
}

export interface OpenImageResult {
  metadata: ImageMetadata;
  originalPreviewDataUrl: string;
  previewDataUrl: string;
  processingTimeMs: number;
  documentId: number;
  isCurrent: boolean;
}

export interface PreviewResult {
  previewDataUrl: string;
  requestId: number;
  processingTimeMs: number;
  isCurrent: boolean;
  operationCount: number;
}

export interface ExportResult {
  outputPath: string;
  width: number;
  height: number;
  processingTimeMs: number;
}

export interface ColorCastEstimate {
  dominant: 'neutral' | 'warm' | 'cool' | 'green' | 'mixed';
  redBias: number;
  greenBias: number;
  blueBias: number;
}

export interface ImageQualityAnalysis {
  averageLuminance: number;
  luminanceSpread: number;
  estimatedColorCast: ColorCastEstimate;
  estimatedNoise: number;
  estimatedSharpness: number;
  estimatedLocalContrast: number;
  edgeDensity: number;
  whiteBackgroundRatio: number;
  likelyDocument: boolean;
}

export interface AnalysisResult {
  analysis: ImageQualityAnalysis | null;
  documentId: number;
  requestId: number;
  processingTimeMs: number;
  isCurrent: boolean;
}

export interface AppErrorPayload {
  code?: string;
  message?: string;
}

export interface Preset {
  id: string;
  name: string;
  description: string;
  operations: EditOperation[];
}
