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
  | { type: 'sharpen'; strength: number };

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
