import { describe, expect, it } from 'vitest';
import type { EditOperation, Workflow } from '../types/editor';
import {
  createWorkflow,
  duplicateOperationAt,
  duplicateWorkflow,
  loadWorkflows,
  MAX_WORKFLOW_JSON_CHARACTERS,
  moveOperation,
  parseWorkflowDocument,
  removeOperationAt,
  removeWorkflow,
  saveWorkflows,
  searchWorkflows,
  toggleFavorite,
  upsertWorkflow,
  validateEditOperation,
  validateWorkflow,
  workflowDocument
} from './workflows';

const operations: EditOperation[] = [
  { type: 'brightness', amount: 0.1 },
  { type: 'levels', input_black: 5, input_white: 245, gamma: 1, output_black: 0, output_white: 255 },
  { type: 'sharpen', strength: 0.2 }
];

function workflow(name = 'Restore Scan', folder = 'Restoration'): Workflow {
  return createWorkflow(name, operations, folder, new Date('2026-01-02T03:04:05Z'));
}

const validMask = {
  version: 1,
  width: 2,
  height: 1,
  encoding: 'base64_u8',
  data: 'AP8',
  checksum: 'fnv1a64:0831c907b4ea2b60'
};

const maskedOperation: EditOperation = {
  type: 'masked',
  operation: { type: 'brightness', amount: 0.25 },
  mask: validMask,
  invert: false,
  mask_id: 'subject'
};

const identityCurve = [{ input: 0, output: 0 }, { input: 1, output: 1 }];
const hslAdjustment = { hue: 0, saturation: 0, lightness: 0 };
const everyBaseOperation: EditOperation[] = [
  { type: 'brightness', amount: 0 }, { type: 'contrast', amount: 0 }, { type: 'saturation', amount: 0 },
  { type: 'gamma', value: 1 }, { type: 'grayscale' }, { type: 'sepia' }, { type: 'reflect_horizontal' },
  { type: 'rotate', degrees: -90 }, { type: 'gaussian_blur', radius: 1 }, { type: 'sharpen', strength: 1 },
  { type: 'auto_white_balance', strength: 1 }, { type: 'local_contrast', strength: 1, tile_size: 32, clip_limit: 2 },
  { type: 'denoise', strength: 1, preserve_edges: 1 }, { type: 'deblock', strength: 1 },
  { type: 'edge_aware_sharpen', strength: 1, radius: 1, threshold: 0.1 },
  { type: 'mild_deblur', strength: 1, radius: 1 }, { type: 'document_enhance', strength: 1, grayscale: false },
  { type: 'uneven_lighting_correction', strength: 1, radius: 16 },
  { type: 'curves', curves: { rgb: identityCurve, red: identityCurve, green: identityCurve, blue: identityCurve } },
  { type: 'levels', input_black: 0, input_white: 255, gamma: 1, output_black: 0, output_white: 255 },
  { type: 'white_point', red: 255, green: 255, blue: 255 }, { type: 'black_point', red: 0, green: 0, blue: 0 },
  { type: 'crop', x: 0, y: 0, width: 1, height: 1, aspect_ratio: null, overlay: 'none' },
  { type: 'straighten', degrees: 0 },
  { type: 'perspective', corners: { topLeft: [0, 0], topRight: [1, 0], bottomRight: [1, 1], bottomLeft: [0, 1] } },
  { type: 'lens_correction', distortion: 0, vignetting: 0, chromatic_aberration: 0 },
  { type: 'hsl', settings: { master: hslAdjustment, red: hslAdjustment, yellow: hslAdjustment, green: hslAdjustment, cyan: hslAdjustment, blue: hslAdjustment, magenta: hslAdjustment } },
  { type: 'temperature_tint', temperature: 0, tint: 0 },
  { type: 'selective_color', target_hue: 180, width: 45, adjustment: { cyan: 0, magenta: 0, yellow: 0, black: 0 } }
];

describe('workflow system', () => {
  it('creates stable local workflow metadata', () => {
    const value = workflow();
    expect(value.id).toMatch(/^restore-scan-[a-z0-9]+$/);
    expect(value.createdAt).toBe('2026-01-02T03:04:05.000Z');
    expect(value.operations).not.toBe(operations);
  });

  it('wraps workflow in versioned schema', () => {
    expect(workflowDocument(workflow()).schemaVersion).toBe(1);
  });

  it('round trips workflow JSON', () => {
    const document = workflowDocument(workflow());
    expect(parseWorkflowDocument(JSON.stringify(document))).toEqual(document);
  });

  it('keeps valid Phase 6 documents compatible when optional metadata is absent', () => {
    const legacy = { id: 'phase-6', name: 'Phase 6', operations: [{ type: 'brightness', amount: 0.2 }] };
    const parsed = parseWorkflowDocument(JSON.stringify({ schemaVersion: 1, workflow: legacy }));
    expect(parsed.workflow).toMatchObject({ ...legacy, description: '', folder: '', favorite: false });
    expect(loadWorkflows({ getItem: () => JSON.stringify([legacy]) })).toEqual([parsed.workflow]);
  });

  it('round trips a valid workflow with an integrity-checked embedded mask', () => {
    const value = createWorkflow('Masked', [maskedOperation]);
    expect(validateEditOperation(maskedOperation)).toBeNull();
    expect(parseWorkflowDocument(JSON.stringify(workflowDocument(value)))).toEqual(workflowDocument(value));
  });

  it('accepts every supported base operation discriminant with valid required parameters', () => {
    expect(everyBaseOperation.map(validateEditOperation)).toEqual(everyBaseOperation.map(() => null));
  });

  it.each([
    [{ type: 'unknown' }, /unsupported operation type/],
    [{ type: 'brightness', amount: Number.NaN }, /parameters are invalid/],
    [{ type: 'local_contrast', strength: 0.5, tile_size: 129, clip_limit: 1 }, /parameters are invalid/],
    [{ type: 'perspective', corners: { topLeft: [1, 0], topRight: [0, 0], bottomRight: [1, 1], bottomLeft: [0, 1] } }, /parameters are invalid/],
    [{ type: 'masked', operation: { type: 'crop', x: 0, y: 0, width: 1, height: 1, aspect_ratio: null, overlay: 'none' }, mask: validMask, invert: false, mask_id: null }, /cannot be masked/]
  ])('rejects malformed operation %#', (operation, message) => {
    expect(validateEditOperation(operation)).toMatch(message);
  });

  it.each([
    [{ ...validMask, checksum: 'fnv1a64:0000000000000000' }, /checksum/],
    [{ ...validMask, data: 'AP8=' }, /base64/],
    [{ ...validMask, encoding: 'base64_rle_u8', data: 'AAAA' }, /malformed/],
    [{ ...validMask, width: 100_000, height: 100_000 }, /bounded limit/]
  ])('rejects malformed or oversized embedded mask %#', (mask, message) => {
    expect(validateEditOperation({ ...maskedOperation, mask })).toMatch(message);
  });

  it.each([0, 2, 99, -1])('rejects unsupported schema version %s', (schemaVersion) => {
    expect(() => parseWorkflowDocument(JSON.stringify({ schemaVersion, workflow: workflow() }))).toThrow(/Unsupported/);
  });

  it('reports missing workflow names', () => {
    expect(validateWorkflow({ ...workflow(), name: '' })).toContain('Workflow name is required.');
  });

  it('reports empty workflows', () => {
    expect(validateWorkflow({ ...workflow(), operations: [] })).toContain('Add at least one operation.');
  });

  it('inserts and replaces by id', () => {
    const first = workflow();
    const inserted = upsertWorkflow([], first);
    expect(inserted).toHaveLength(1);
    expect(upsertWorkflow(inserted, { ...first, name: 'Renamed' })[0].name).toBe('Renamed');
  });

  it('duplicates workflow with independent operations', () => {
    const values = duplicateWorkflow([workflow()], workflow().id, new Date('2026-01-03T00:00:00Z'));
    expect(values).toHaveLength(2);
    expect(values[0].name).toBe('Restore Scan Copy');
    expect(values[0].operations).not.toBe(values[1].operations);
  });

  it('removes only selected workflow', () => {
    const first = workflow('One'); const second = workflow('Two');
    expect(removeWorkflow([first, second], first.id)).toEqual([second]);
  });

  it('toggles favorites immutably', () => {
    const value = workflow();
    const toggled = toggleFavorite([value], value.id);
    expect(toggled[0].favorite).toBe(true);
    expect(value.favorite).toBe(false);
  });

  it('searches names, folders, and descriptions', () => {
    const scan = { ...workflow('Restore Scan', 'Archive'), description: 'Faded family photo' };
    expect(searchWorkflows([scan], 'family')).toHaveLength(1);
    expect(searchWorkflows([scan], 'archive')).toHaveLength(1);
    expect(searchWorkflows([scan], 'missing')).toHaveLength(0);
  });

  it('sorts favorites first', () => {
    const normal = workflow('A'); const favorite = { ...workflow('Z'), favorite: true };
    expect(searchWorkflows([normal, favorite], '')[0].name).toBe('Z');
  });

  it.each([
    [0, 1, ['levels', 'brightness', 'sharpen']],
    [1, -1, ['levels', 'brightness', 'sharpen']],
    [1, 1, ['brightness', 'sharpen', 'levels']],
    [2, -1, ['brightness', 'sharpen', 'levels']]
  ] as const)('moves operation %s by %s', (index, delta, types) => {
    expect(moveOperation(operations, index, delta).map((operation) => operation.type)).toEqual(types);
  });

  it.each([[-1, -1], [0, -1], [2, 1], [9, 1]] as const)('bounds invalid move %s/%s', (index, delta) => {
    expect(moveOperation(operations, index, delta)).toEqual(operations);
  });

  it.each([0, 1, 2])('removes operation at index %s', (index) => {
    const result = removeOperationAt(operations, index);
    expect(result).toHaveLength(2);
    expect(result).not.toContainEqual(operations[index]);
  });

  it.each([0, 1, 2])('duplicates operation at index %s', (index) => {
    const result = duplicateOperationAt(operations, index);
    expect(result).toHaveLength(4);
    expect(result[index + 1]).toEqual(operations[index]);
    expect(result[index + 1]).not.toBe(operations[index]);
  });

  it('persists workflows locally', () => {
    const values = [workflow()];
    let saved = '';
    saveWorkflows(values, { setItem: (_key, value) => (saved = value) });
    expect(loadWorkflows({ getItem: () => saved })).toEqual(values);
  });

  it('keeps only deeply valid workflows from mixed local storage', () => {
    const valid = workflow();
    const invalid = { ...workflow('Bad'), operations: [{ type: 'brightness', amount: 99 }] };
    expect(loadWorkflows({ getItem: () => JSON.stringify([invalid, valid, { operations: [] }]) })).toEqual([valid]);
  });

  it('fails closed before parsing oversized local storage', () => {
    const oversized = { length: MAX_WORKFLOW_JSON_CHARACTERS + 1 } as unknown as string;
    expect(loadWorkflows({ getItem: () => oversized })).toEqual([]);
  });

  it('does not persist malformed workflow operations', () => {
    const invalid = { ...workflow('Bad'), operations: [{ type: 'brightness', amount: 5 }] } as unknown as Workflow;
    let saved = '';
    saveWorkflows([invalid, workflow()], { setItem: (_key, value) => (saved = value) });
    expect(JSON.parse(saved)).toHaveLength(1);
  });

  it.each(['', '{', 'false', '{}'])('recovers safely from invalid storage %s', (stored) => {
    expect(loadWorkflows({ getItem: () => stored })).toEqual([]);
  });
});
