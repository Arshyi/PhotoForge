import type { EditOperation, Workflow, WorkflowDocument } from '../types/editor';
import { cloneOperations } from './operations';

export const WORKFLOW_SCHEMA_VERSION = 1 as const;
export const WORKFLOW_STORAGE_KEY = 'photoforge.workflows.v1';
export const MAX_WORKFLOWS = 250;
export const MAX_WORKFLOW_OPERATIONS = 200;

export function createWorkflow(
  name: string,
  operations: EditOperation[],
  folder = '',
  now = new Date()
): Workflow {
  const timestamp = now.toISOString();
  const slug = name
    .trim()
    .toLowerCase()
    .replace(/[^a-z0-9]+/g, '-')
    .replace(/^-|-$/g, '') || 'workflow';
  return {
    id: `${slug}-${now.getTime().toString(36)}`,
    name: name.trim(),
    description: '',
    folder: folder.trim(),
    favorite: false,
    operations: cloneOperations(operations),
    createdAt: timestamp,
    updatedAt: timestamp
  };
}

export function validateWorkflow(workflow: Workflow): string[] {
  const errors: string[] = [];
  if (!workflow.id?.trim() || workflow.id.length > 128) errors.push('Workflow ID is invalid.');
  if (!workflow.name?.trim() || workflow.name.length > 120) errors.push('Workflow name is required.');
  if (!Array.isArray(workflow.operations) || workflow.operations.length === 0) errors.push('Add at least one operation.');
  if (workflow.operations?.length > MAX_WORKFLOW_OPERATIONS) errors.push('Workflow has too many operations.');
  if (workflow.folder?.length > 120) errors.push('Workflow folder is too long.');
  return errors;
}

export function workflowDocument(workflow: Workflow): WorkflowDocument {
  return { schemaVersion: WORKFLOW_SCHEMA_VERSION, workflow: cloneWorkflow(workflow) };
}

export function parseWorkflowDocument(json: string): WorkflowDocument {
  const value = JSON.parse(json) as Partial<WorkflowDocument>;
  if (value.schemaVersion !== WORKFLOW_SCHEMA_VERSION) {
    throw new Error(`Unsupported workflow schema version ${String(value.schemaVersion)}.`);
  }
  if (!value.workflow) throw new Error('Workflow document is missing its workflow.');
  const errors = validateWorkflow(value.workflow);
  if (errors.length) throw new Error(errors.join(' '));
  return workflowDocument(value.workflow);
}

export function loadWorkflows(storage: Pick<Storage, 'getItem'> = localStorage): Workflow[] {
  try {
    const raw = storage.getItem(WORKFLOW_STORAGE_KEY);
    if (!raw) return [];
    const values = JSON.parse(raw) as Workflow[];
    if (!Array.isArray(values)) return [];
    return values.filter((workflow) => validateWorkflow(workflow).length === 0).slice(0, MAX_WORKFLOWS).map(cloneWorkflow);
  } catch {
    return [];
  }
}

export function saveWorkflows(workflows: Workflow[], storage: Pick<Storage, 'setItem'> = localStorage) {
  storage.setItem(WORKFLOW_STORAGE_KEY, JSON.stringify(workflows.slice(0, MAX_WORKFLOWS)));
}

export function upsertWorkflow(workflows: Workflow[], workflow: Workflow): Workflow[] {
  if (validateWorkflow(workflow).length) return workflows.map(cloneWorkflow);
  const copy = workflows.map(cloneWorkflow);
  const index = copy.findIndex((candidate) => candidate.id === workflow.id);
  if (index === -1) copy.unshift(cloneWorkflow(workflow));
  else copy[index] = cloneWorkflow(workflow);
  return copy.slice(0, MAX_WORKFLOWS);
}

export function duplicateWorkflow(workflows: Workflow[], workflowId: string, now = new Date()): Workflow[] {
  const source = workflows.find((workflow) => workflow.id === workflowId);
  if (!source) return workflows.map(cloneWorkflow);
  const duplicate = createWorkflow(`${source.name} Copy`, source.operations, source.folder, now);
  duplicate.description = source.description;
  return [duplicate, ...workflows.map(cloneWorkflow)].slice(0, MAX_WORKFLOWS);
}

export function removeWorkflow(workflows: Workflow[], workflowId: string): Workflow[] {
  return workflows.filter((workflow) => workflow.id !== workflowId).map(cloneWorkflow);
}

export function toggleFavorite(workflows: Workflow[], workflowId: string): Workflow[] {
  return workflows.map((workflow) => ({
    ...cloneWorkflow(workflow),
    favorite: workflow.id === workflowId ? !workflow.favorite : workflow.favorite
  }));
}

export function searchWorkflows(workflows: Workflow[], query: string): Workflow[] {
  const needle = query.trim().toLocaleLowerCase();
  return workflows
    .filter((workflow) => !needle || [workflow.name, workflow.description, workflow.folder].some((value) => value.toLocaleLowerCase().includes(needle)))
    .sort((left, right) => Number(right.favorite) - Number(left.favorite) || left.folder.localeCompare(right.folder) || left.name.localeCompare(right.name))
    .map(cloneWorkflow);
}

export function moveOperation(operations: EditOperation[], index: number, delta: -1 | 1): EditOperation[] {
  const target = index + delta;
  const copy = cloneOperations(operations);
  if (index < 0 || index >= copy.length || target < 0 || target >= copy.length) return copy;
  [copy[index], copy[target]] = [copy[target], copy[index]];
  return copy;
}

export function removeOperationAt(operations: EditOperation[], index: number): EditOperation[] {
  return cloneOperations(operations).filter((_, operationIndex) => operationIndex !== index);
}

export function duplicateOperationAt(operations: EditOperation[], index: number): EditOperation[] {
  const copy = cloneOperations(operations);
  if (!copy[index] || copy.length >= MAX_WORKFLOW_OPERATIONS) return copy;
  copy.splice(index + 1, 0, structuredClone(copy[index]));
  return copy;
}

export function cloneWorkflow(workflow: Workflow): Workflow {
  return { ...workflow, operations: structuredClone(workflow.operations) };
}
