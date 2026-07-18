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
