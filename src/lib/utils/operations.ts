import type { EditOperation, OperationType, Preset } from '../types/editor';

export const presets: Preset[] = [
  {
    id: 'fix-dark',
    name: 'Fix dark photo',
    description: 'Lift shadows and add gentle contrast.',
    operations: [
      { type: 'brightness', amount: 0.14 },
      { type: 'gamma', value: 1.18 },
      { type: 'contrast', amount: 0.08 }
    ]
  },
  {
    id: 'readability',
    name: 'Improve readability',
    description: 'High-contrast grayscale for documents.',
    operations: [
      { type: 'grayscale' },
      { type: 'contrast', amount: 0.28 },
      { type: 'sharpen', strength: 0.45 }
    ]
  },
  {
    id: 'mild-enhance',
    name: 'Mild photo enhancement',
    description: 'A restrained clarity and color lift.',
    operations: [
      { type: 'contrast', amount: 0.08 },
      { type: 'saturation', amount: 0.1 },
      { type: 'sharpen', strength: 0.24 }
    ]
  },
  {
    id: 'soft-contrast',
    name: 'Reduce harsh contrast',
    description: 'Soften extremes without blurring.',
    operations: [
      { type: 'contrast', amount: -0.18 },
      { type: 'brightness', amount: 0.03 }
    ]
  },
  {
    id: 'black-white',
    name: 'Black and white',
    description: 'Neutral grayscale conversion.',
    operations: [{ type: 'grayscale' }]
  },
  {
    id: 'indoor-lighting',
    name: 'Fix Indoor Lighting',
    description: 'Balance a warm cast and lift local contrast.',
    operations: [
      { type: 'auto_white_balance', strength: 0.72 },
      { type: 'local_contrast', strength: 0.32, tile_size: 32, clip_limit: 1.4 }
    ]
  },
  {
    id: 'old-scan',
    name: 'Improve Old Scan',
    description: 'Even lighting, reduce noise, and restore restrained clarity.',
    operations: [
      { type: 'uneven_lighting_correction', strength: 0.62, radius: 40 },
      { type: 'denoise', strength: 0.28, preserve_edges: 0.82 },
      { type: 'edge_aware_sharpen', strength: 0.46, radius: 1.2, threshold: 0.035 }
    ]
  },
  {
    id: 'jpeg-cleanup',
    name: 'Clean Up JPEG',
    description: 'Soften block boundaries without blurring major edges.',
    operations: [
      { type: 'deblock', strength: 0.58 },
      { type: 'denoise', strength: 0.18, preserve_edges: 0.88 }
    ]
  },
  {
    id: 'mild-detail',
    name: 'Mild Detail Recovery',
    description: 'Conservative clarity for slight softness.',
    operations: [
      { type: 'mild_deblur', strength: 0.38, radius: 1.2 },
      { type: 'edge_aware_sharpen', strength: 0.22, radius: 0.9, threshold: 0.045 }
    ]
  },
  {
    id: 'document-color',
    name: 'Enhance Document — Color',
    description: 'Even a photographed page while retaining colored marks.',
    operations: [{ type: 'document_enhance', strength: 0.72, grayscale: false }]
  },
  {
    id: 'document-grayscale',
    name: 'Enhance Document — Grayscale',
    description: 'Improve page readability in neutral grayscale.',
    operations: [{ type: 'document_enhance', strength: 0.76, grayscale: true }]
  },
  {
    id: 'uneven-lighting',
    name: 'Fix Uneven Lighting',
    description: 'Normalize broad illumination gradients.',
    operations: [{ type: 'uneven_lighting_correction', strength: 0.7, radius: 48 }]
  },
  {
    id: 'conservative-restore',
    name: 'Conservative Photo Restore',
    description: 'Balanced color, noise, contrast, and clarity corrections.',
    operations: [
      { type: 'auto_white_balance', strength: 0.42 },
      { type: 'local_contrast', strength: 0.26, tile_size: 40, clip_limit: 1.25 },
      { type: 'denoise', strength: 0.22, preserve_edges: 0.86 },
      { type: 'edge_aware_sharpen', strength: 0.3, radius: 1.1, threshold: 0.04 }
    ]
  }
];

export function operationType(operation: EditOperation): OperationType {
  return operation.type;
}

export function replaceOperation(
  operations: EditOperation[],
  next: EditOperation,
  enabled = true
): EditOperation[] {
  const index = operations.findIndex((operation) => operation.type === next.type);
  if (!enabled) {
    return index === -1
      ? operations.map((operation) => ({ ...operation }))
      : operations.filter((_, operationIndex) => operationIndex !== index);
  }

  const copy = operations.map((operation) => ({ ...operation })) as EditOperation[];
  if (index === -1) copy.push(next);
  else copy[index] = next;
  return copy;
}

export function valueFor(
  operations: EditOperation[],
  type: OperationType,
  fallback: number
): number {
  const operation = operations.find((candidate) => candidate.type === type);
  if (!operation) return fallback;
  if ('amount' in operation) return operation.amount;
  if ('value' in operation) return operation.value;
  if ('radius' in operation) return operation.radius;
  if ('strength' in operation) return operation.strength;
  if ('degrees' in operation) return operation.degrees;
  return fallback;
}

export function cloneOperations(operations: EditOperation[]): EditOperation[] {
  return operations.map((operation) => ({ ...operation })) as EditOperation[];
}

export const operationLabels: Record<OperationType, string> = {
  brightness: 'Brightness',
  contrast: 'Contrast',
  saturation: 'Saturation',
  gamma: 'Gamma',
  grayscale: 'Grayscale',
  sepia: 'Sepia',
  reflect_horizontal: 'Reflect',
  rotate: 'Rotate',
  gaussian_blur: 'Blur',
  sharpen: 'Sharpen',
  auto_white_balance: 'Auto White Balance',
  local_contrast: 'Local Contrast',
  denoise: 'Denoise',
  deblock: 'JPEG Cleanup',
  edge_aware_sharpen: 'Edge-Aware Sharpen',
  mild_deblur: 'Mild Deblur',
  document_enhance: 'Document Enhance',
  uneven_lighting_correction: 'Uneven Lighting'
  ,curves: 'Curves'
  ,levels: 'Levels'
  ,white_point: 'White Point'
  ,black_point: 'Black Point'
  ,crop: 'Crop'
  ,straighten: 'Straighten'
  ,perspective: 'Perspective'
  ,lens_correction: 'Lens Correction'
  ,hsl: 'HSL'
  ,temperature_tint: 'Temperature & Tint'
  ,selective_color: 'Selective Color'
};
