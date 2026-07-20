import { describe, expect, it } from 'vitest';
import type { EditOperation, Workflow } from '../types/editor';
import {
  createWorkflow,
  duplicateOperationAt,
  duplicateWorkflow,
  loadWorkflows,
  moveOperation,
  parseWorkflowDocument,
  removeOperationAt,
  removeWorkflow,
  saveWorkflows,
  searchWorkflows,
  toggleFavorite,
  upsertWorkflow,
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

  it.each(['', '{', 'false', '{}'])('recovers safely from invalid storage %s', (stored) => {
    expect(loadWorkflows({ getItem: () => stored })).toEqual([]);
  });
});
