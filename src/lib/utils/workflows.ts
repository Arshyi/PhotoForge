import type { BaseEditOperation, EditOperation, Workflow, WorkflowDocument } from '../types/editor';
import type { MaskSnapshot } from '../selections/types';
import { decodedCoverageChecksum } from '../selections/checksum';
import { cloneOperations } from './operations';
import { blendModes } from '../layers/types';
import { MAX_LAYER_WORKFLOW_STEPS, type LayerWorkflowStep } from '../layers/workflow';

export const WORKFLOW_SCHEMA_VERSION = 2 as const;
export const WORKFLOW_STORAGE_KEY = 'photoforge.workflows.v1';
export const MAX_WORKFLOWS = 250;
export const MAX_WORKFLOW_OPERATIONS = 200;
export const MAX_WORKFLOW_JSON_CHARACTERS = 64 * 1024 * 1024;
const MAX_MASK_PIXELS = 100_000_000;

export function createWorkflow(
  name: string,
  operations: EditOperation[],
  folder = '',
  now = new Date(),
  layerSteps: LayerWorkflowStep[] = []
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
    layerSteps: structuredClone(layerSteps),
    createdAt: timestamp,
    updatedAt: timestamp
  };
}

export function validateWorkflow(workflow: unknown): string[] {
  const errors: string[] = [];
  if (!isRecord(workflow)) return ['Workflow structure is invalid.'];
  if (!boundedText(workflow.id, 1, 128, true)) errors.push('Workflow ID is invalid.');
  if (!boundedText(workflow.name, 1, 120, true)) errors.push('Workflow name is required.');
  if (!boundedText(workflow.description, 0, 1_000)) errors.push('Workflow description is too long.');
  if (!boundedText(workflow.folder, 0, 120)) errors.push('Workflow folder is too long.');
  if (typeof workflow.favorite !== 'boolean') errors.push('Workflow favorite flag is invalid.');
  if (!boundedText(workflow.createdAt, 0, 64) || !boundedText(workflow.updatedAt, 0, 64)) {
    errors.push('Workflow timestamps are invalid.');
  }
  if (!Array.isArray(workflow.operations)) {
    errors.push('Workflow operations must be an array.');
  } else if (workflow.operations.length > MAX_WORKFLOW_OPERATIONS) {
    errors.push('Workflow has too many operations.');
  } else {
    workflow.operations.forEach((operation, index) => {
      const error = validateEditOperation(operation);
      if (error) errors.push(`Operation ${index + 1}: ${error}`);
    });
  }
  const layerSteps = workflow.layerSteps === undefined ? [] : workflow.layerSteps;
  errors.push(...validateLayerWorkflowSteps(layerSteps));
  if (Array.isArray(workflow.operations) && workflow.operations.length === 0 &&
    Array.isArray(layerSteps) && layerSteps.length === 0) {
    errors.push('Add at least one operation or layer step.');
  }
  return errors;
}

export function workflowDocument(workflow: Workflow): WorkflowDocument {
  const errors = validateWorkflow(workflow);
  if (errors.length) throw new Error(errors.join(' '));
  return { schemaVersion: WORKFLOW_SCHEMA_VERSION, workflow: cloneWorkflow(workflow) };
}

export function parseWorkflowDocument(json: string): WorkflowDocument {
  if (json.length > MAX_WORKFLOW_JSON_CHARACTERS) throw new Error('Workflow JSON exceeds the bounded limit.');
  const value: unknown = JSON.parse(json);
  if (!isRecord(value) || (value.schemaVersion !== 1 && value.schemaVersion !== WORKFLOW_SCHEMA_VERSION)) {
    throw new Error(`Unsupported workflow schema version ${String(isRecord(value) ? value.schemaVersion : undefined)}.`);
  }
  if (!isRecord(value.workflow)) throw new Error('Workflow document is missing its workflow.');
  if (value.schemaVersion === 1 && value.workflow.layerSteps !== undefined &&
    (!Array.isArray(value.workflow.layerSteps) || value.workflow.layerSteps.length !== 0)) {
    throw new Error('Schema version 1 cannot contain layer steps; use version 2.');
  }
  const workflow = normalizeWorkflow(value.workflow);
  const errors = workflow ? [] : validateWorkflow(withWorkflowDefaults(value.workflow));
  if (errors.length) throw new Error(errors.join(' '));
  if (!workflow) throw new Error('Workflow structure is invalid.');
  return workflowDocument(workflow);
}

export function loadWorkflows(storage: Pick<Storage, 'getItem'> = localStorage): Workflow[] {
  try {
    const raw = storage.getItem(WORKFLOW_STORAGE_KEY);
    if (!raw || raw.length > MAX_WORKFLOW_JSON_CHARACTERS) return [];
    const values: unknown = JSON.parse(raw);
    if (!Array.isArray(values)) return [];
    return values
      .map(normalizeWorkflow)
      .filter((workflow): workflow is Workflow => workflow !== null)
      .slice(0, MAX_WORKFLOWS)
      .map(cloneWorkflow);
  } catch {
    return [];
  }
}

export function saveWorkflows(workflows: Workflow[], storage: Pick<Storage, 'setItem'> = localStorage) {
  const valid = workflows
    .filter((workflow) => validateWorkflow(workflow).length === 0)
    .slice(0, MAX_WORKFLOWS)
    .map(cloneWorkflow);
  const json = JSON.stringify(valid);
  if (json.length > MAX_WORKFLOW_JSON_CHARACTERS) throw new Error('Workflow storage exceeds the bounded limit.');
  storage.setItem(WORKFLOW_STORAGE_KEY, json);
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
  const duplicate = createWorkflow(`${source.name} Copy`, source.operations, source.folder, now, source.layerSteps);
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
  return { ...workflow, operations: structuredClone(workflow.operations), layerSteps: structuredClone(workflow.layerSteps ?? []) };
}

/** Validate all persisted layer instructions before any replay or migration. */
export function validateLayerWorkflowSteps(value: unknown): string[] {
  if (!Array.isArray(value)) return ['Workflow layer steps must be an array.'];
  if (value.length > MAX_LAYER_WORKFLOW_STEPS) return ['Workflow has too many layer steps.'];
  const errors: string[] = [];
  let created = false;
  value.forEach((step, index) => {
    const error = validateLayerWorkflowStep(step);
    if (error) errors.push(`Layer step ${index + 1}: ${error}`);
    else if (isRecord(step)) {
      if (isRecord(step.selector) && step.selector.type === 'last_created' && !created) {
        errors.push(`Layer step ${index + 1}: last_created requires an earlier adjustment-layer creation.`);
      }
      if (step.type === 'create_adjustment_layer') created = true;
    }
  });
  return errors;
}

function validateLayerWorkflowStep(value: unknown): string | null {
  if (!isRecord(value) || typeof value.type !== 'string') return 'layer step structure is invalid.';
  const fields: Record<string, string[]> = {
    select_layer: ['selector'], set_visibility: ['selector', 'visible'],
    set_opacity: ['selector', 'opacity'], set_blend_mode: ['selector', 'blendMode'],
    create_adjustment_layer: ['operation', 'name'], apply_to_layer: ['selector', 'operations'],
    create_mask_from_selection: ['selector'], merge_down: ['selector'],
    flatten: [], export_composite: []
  };
  if (!Object.prototype.hasOwnProperty.call(fields, value.type)) return `unsupported layer step type ${value.type}.`;
  if (Object.keys(value).some((key) => key !== 'type' && !fields[value.type as string].includes(key))) {
    return 'unknown layer step fields are not allowed.';
  }
  if (fields[value.type].includes('selector')) {
    const error = validateLayerSelector(value.selector);
    if (error) return error;
  }
  switch (value.type) {
    case 'set_visibility': return typeof value.visible === 'boolean' ? null : 'visibility must be a boolean.';
    case 'set_opacity': return finiteRange(value.opacity, 0, 1) ? null : 'opacity must be between 0 and 1.';
    case 'set_blend_mode': return blendModes.some((mode) => mode.id === value.blendMode) ? null : 'blend mode is invalid.';
    case 'create_adjustment_layer': {
      if (value.name !== undefined && value.name !== null && !boundedLayerText(value.name, 120)) return 'layer name is invalid.';
      return validateLayerOperation(value.operation);
    }
    case 'apply_to_layer': {
      if (!Array.isArray(value.operations) || value.operations.length === 0 || value.operations.length > MAX_WORKFLOW_OPERATIONS) {
        return `apply_to_layer needs 1 to ${MAX_WORKFLOW_OPERATIONS} operations.`;
      }
      for (const operation of value.operations) {
        const error = validateLayerOperation(operation);
        if (error) return error;
      }
    }
  }
  return null;
}

function validateLayerSelector(value: unknown): string | null {
  if (!isRecord(value) || typeof value.type !== 'string') return 'layer selector is invalid.';
  const field = value.type === 'id' ? 'id' : value.type === 'name' ? 'name' : null;
  if (!['id', 'name', 'active', 'last_created', 'bottom', 'top'].includes(value.type) ||
    Object.keys(value).some((key) => key !== 'type' && key !== field)) return 'layer selector is invalid.';
  return field && !boundedLayerText(value[field], field === 'id' ? 64 : 120) ? 'layer selector text is invalid.' : null;
}

function boundedLayerText(value: unknown, maximum: number): boolean {
  return boundedText(value, 1, maximum, true) && new TextEncoder().encode(value).length <= maximum;
}

function validateLayerOperation(value: unknown): string | null {
  const error = validateEditOperation(value);
  if (error) return error;
  return isRecord(value) && supportsMasking(value.type) && value.type !== 'decontaminate_colors'
    ? null : 'this operation cannot be used as a layer adjustment or single-layer edit.';
}

export function validateEditOperation(value: unknown): string | null {
  if (!isRecord(value) || typeof value.type !== 'string') return 'operation structure is invalid.';
  if (value.type === 'masked') {
    if (!isRecord(value.operation) || value.operation.type === 'masked') return 'nested masked operations are invalid.';
    const operationError = validateBaseOperation(value.operation);
    if (operationError) return operationError;
    if (!supportsMasking(value.operation.type)) return `${value.operation.type} cannot be masked.`;
    if (typeof value.invert !== 'boolean') return 'masked invert flag is invalid.';
    if (value.mask_id !== null && !boundedText(value.mask_id, 1, 128, true)) return 'mask identifier is invalid.';
    return validateMaskSnapshot(value.mask);
  }
  if (value.type === 'decontaminate_colors') {
    return validateBaseOperation(value) ?? 'decontaminate_colors requires an embedded selection mask.';
  }
  return validateBaseOperation(value);
}

function validateBaseOperation(value: Record<string, unknown>): string | null {
  const invalid = () => `parameters are invalid for ${String(value.type)}.`;
  switch (value.type) {
    case 'brightness':
    case 'contrast':
    case 'saturation': return finiteRange(value.amount, -1, 1) ? null : invalid();
    case 'gamma': return finiteRange(value.value, 0.2, 3) ? null : invalid();
    case 'grayscale':
    case 'sepia':
    case 'reflect_horizontal': return null;
    case 'rotate':
      return integerRange(value.degrees, -2_147_483_648, 2_147_483_647) &&
        [0, 90, 180, 270].includes(((Number(value.degrees) % 360) + 360) % 360) ? null : invalid();
    case 'gaussian_blur': return finiteRange(value.radius, 0, 20) ? null : invalid();
    case 'sharpen': return finiteRange(value.strength, 0, 2) ? null : invalid();
    case 'auto_white_balance':
    case 'deblock': return finiteRange(value.strength, 0, 1) ? null : invalid();
    case 'local_contrast':
      return finiteRange(value.strength, 0, 1) && integerRange(value.tile_size, 8, 128) && finiteRange(value.clip_limit, 0.5, 4) ? null : invalid();
    case 'denoise':
      return finiteRange(value.strength, 0, 1) && finiteRange(value.preserve_edges, 0, 1) ? null : invalid();
    case 'edge_aware_sharpen':
      return finiteRange(value.strength, 0, 2) && finiteRange(value.radius, 0.5, 4) && finiteRange(value.threshold, 0, 0.25) ? null : invalid();
    case 'mild_deblur':
      return finiteRange(value.strength, 0, 1) && finiteRange(value.radius, 0.5, 3) ? null : invalid();
    case 'document_enhance':
      return finiteRange(value.strength, 0, 1) && typeof value.grayscale === 'boolean' ? null : invalid();
    case 'uneven_lighting_correction':
      return finiteRange(value.strength, 0, 1) && finiteRange(value.radius, 4, 96) ? null : invalid();
    case 'curves': return validCurveSet(value.curves) ? null : invalid();
    case 'levels':
      return byte(value.input_black) && byte(value.input_white) && Number(value.input_black) < Number(value.input_white) &&
        finiteRange(value.gamma, 0.1, 10) && byte(value.output_black) && byte(value.output_white) &&
        Number(value.output_black) <= Number(value.output_white) ? null : invalid();
    case 'white_point':
      return positiveByte(value.red) && positiveByte(value.green) && positiveByte(value.blue) ? null : invalid();
    case 'black_point': return byte(value.red) && byte(value.green) && byte(value.blue) ? null : invalid();
    case 'crop':
      return finiteRange(value.x, 0, 1.000_001) && finiteRange(value.y, 0, 1.000_001) &&
        finiteRange(value.width, Number.MIN_VALUE, 1.000_001) && finiteRange(value.height, Number.MIN_VALUE, 1.000_001) &&
        Number(value.x) + Number(value.width) <= 1.000_001 && Number(value.y) + Number(value.height) <= 1.000_001 &&
        (value.aspect_ratio === null || boundedText(value.aspect_ratio, 0, 32)) &&
        ['none', 'rule_of_thirds', 'golden_ratio'].includes(String(value.overlay)) ? null : invalid();
    case 'straighten': return finiteRange(value.degrees, -45, 45) ? null : invalid();
    case 'perspective': return validPerspective(value.corners) ? null : invalid();
    case 'lens_correction':
      return finiteRange(value.distortion, -0.16, 1) && finiteRange(value.vignetting, -1, 1) &&
        finiteRange(value.chromatic_aberration, -1, 1) ? null : invalid();
    case 'decontaminate_colors':
      return typeof value.enabled === 'boolean' && finiteRange(value.strength, 0, 1) &&
        integerRange(value.radius, 1, 32) ? null : invalid();
    case 'hsl': return validHsl(value.settings) ? null : invalid();
    case 'temperature_tint':
      return finiteRange(value.temperature, -1, 1) && finiteRange(value.tint, -1, 1) ? null : invalid();
    case 'raw_development':
      return validDevelopmentParameters(value.parameters) ? null : invalid();
    case 'selective_color':
      return finiteRange(value.target_hue, 0, 360) && finiteRange(value.width, 1, 180) &&
        validSelectiveAdjustment(value.adjustment) ? null : invalid();
    case 'plugin_filter': return validPluginFilter(value) ? null : invalid();
    default: return `unsupported operation type ${String(value.type)}.`;
  }
}

/** Mirrors `plugins::apply::validate_reference`: the form of the reference, nothing about whether it is installed. */
function validPluginFilter(value: Record<string, unknown>): boolean {
  const local = /^[a-z][a-z0-9_]{0,47}$/;
  const pluginId = typeof value.plugin === 'string' && value.plugin.length >= 3 && value.plugin.length <= 64 &&
    /^[a-z0-9][a-z0-9-]*(\.[a-z0-9][a-z0-9-]*){1,7}$/.test(value.plugin) && !value.plugin.startsWith('core.');
  const locality = value.locality;
  const localityOk = isRecord(locality) && (
    locality.kind === 'pointwise' || locality.kind === 'global' ||
    (locality.kind === 'local' && integerRange(locality.radius, 1, 256)));
  return pluginId &&
    typeof value.version === 'string' && /^\d{1,6}\.\d{1,6}\.\d{1,6}$/.test(value.version) &&
    typeof value.sha256 === 'string' && /^[0-9a-f]{64}$/.test(value.sha256) &&
    typeof value.filter === 'string' && local.test(value.filter) &&
    localityOk &&
    Array.isArray(value.parameters) && value.parameters.length <= 32 &&
    value.parameters.every((parameter) => typeof parameter === 'number' && Number.isFinite(parameter));
}

function validateMaskSnapshot(value: unknown): string | null {
  if (!isRecord(value) || value.version !== 1 || !integerRange(value.width, 1, 100_000) ||
    !integerRange(value.height, 1, 100_000)) return 'embedded mask structure is invalid.';
  const pixels = Number(value.width) * Number(value.height);
  if (!Number.isSafeInteger(pixels) || pixels > MAX_MASK_PIXELS) return 'embedded mask dimensions exceed the bounded limit.';
  if ((value.encoding !== 'base64_u8' && value.encoding !== 'base64_rle_u8') ||
    typeof value.data !== 'string' || typeof value.checksum !== 'string' ||
    !/^fnv1a64:[0-9a-f]{16}$/.test(value.checksum)) return 'embedded mask encoding or checksum is invalid.';
  const maximumEncoded = Math.floor((pixels * 4) / 3) + 4;
  if (value.data.length > maximumEncoded || !/^[A-Za-z0-9+/]*$/.test(value.data) || value.data.length % 4 === 1) {
    return 'embedded mask data exceeds its dimensions or is not base64.';
  }
  const decodedChecksum = decodedCoverageChecksum(value as unknown as MaskSnapshot);
  if (decodedChecksum === null) return 'embedded mask data is malformed or does not match its dimensions.';
  return decodedChecksum === value.checksum ? null : 'embedded mask integrity checksum does not match.';
}

function validCurveSet(value: unknown): boolean {
  if (!isRecord(value)) return false;
  return ['rgb', 'red', 'green', 'blue'].every((channel) => validCurve(value[channel]));
}

function validCurve(value: unknown): boolean {
  return Array.isArray(value) && value.length >= 2 && value.length <= 32 && value.every((point) =>
    isRecord(point) && finiteRange(point.input, 0, 1) && finiteRange(point.output, 0, 1)) &&
    value.every((point, index) => index === 0 || Number(value[index - 1].input) < Number(point.input)) &&
    Number(value[0].input) === 0 && Number(value[value.length - 1].input) === 1;
}

function validHsl(value: unknown): boolean {
  if (!isRecord(value)) return false;
  return ['master', 'red', 'yellow', 'green', 'cyan', 'blue', 'magenta'].every((key) => {
    const adjustment = value[key];
    return isRecord(adjustment) && finiteRange(adjustment.hue, -180, 180) &&
      finiteRange(adjustment.saturation, -1, 1) && finiteRange(adjustment.lightness, -1, 1);
  });
}

function validPerspective(value: unknown): boolean {
  if (!isRecord(value)) return false;
  const topLeft = point(value.topLeft); const topRight = point(value.topRight);
  const bottomRight = point(value.bottomRight); const bottomLeft = point(value.bottomLeft);
  return Boolean(topLeft && topRight && bottomRight && bottomLeft && topLeft[0] < topRight[0] &&
    bottomLeft[0] < bottomRight[0] && topLeft[1] < bottomLeft[1] && topRight[1] < bottomRight[1]);
}

function point(value: unknown): [number, number] | null {
  return Array.isArray(value) && value.length === 2 && finiteRange(value[0], 0, 1) && finiteRange(value[1], 0, 1)
    ? [Number(value[0]), Number(value[1])] : null;
}

function validSelectiveAdjustment(value: unknown): boolean {
  return isRecord(value) && ['cyan', 'magenta', 'yellow', 'black'].every((key) => finiteRange(value[key], -1, 1));
}

function validDevelopmentParameters(value: unknown): boolean {
  if (!isRecord(value) || !finiteRange(value.exposureEv, -8, 8) ||
    !finiteRange(value.contrast, -1, 1) || !finiteRange(value.highlights, -1, 1) ||
    !finiteRange(value.shadows, -1, 1) || !finiteRange(value.whites, -1, 1) ||
    !finiteRange(value.blacks, -1, 1) || !isRecord(value.whiteBalance) ||
    typeof value.whiteBalance.mode !== 'string') return false;
  switch (value.whiteBalance.mode) {
    case 'auto': return true;
    case 'temperatureTint':
      return finiteRange(value.whiteBalance.temperature, -1, 1) && finiteRange(value.whiteBalance.tint, -1, 1);
    case 'asShot':
    case 'custom':
      return Array.isArray(value.whiteBalance.multipliers) && value.whiteBalance.multipliers.length === 3 &&
        value.whiteBalance.multipliers.every((multiplier) => finiteRange(multiplier, 0.01, 16));
    default: return false;
  }
}

function supportsMasking(type: unknown): type is BaseEditOperation['type'] {
  return typeof type === 'string' && !['reflect_horizontal', 'rotate', 'crop', 'straighten', 'perspective', 'lens_correction', 'masked'].includes(type);
}

function normalizeWorkflow(value: unknown): Workflow | null {
  if (!isRecord(value)) return null;
  const candidate = withWorkflowDefaults(value);
  return validateWorkflow(candidate).length === 0 ? cloneWorkflow(candidate as unknown as Workflow) : null;
}

function withWorkflowDefaults(value: Record<string, unknown>): Record<string, unknown> {
  return {
    ...value,
    description: value.description === undefined ? '' : value.description,
    folder: value.folder === undefined ? '' : value.folder,
    favorite: value.favorite === undefined ? false : value.favorite,
    createdAt: value.createdAt === undefined ? '' : value.createdAt,
    updatedAt: value.updatedAt === undefined ? '' : value.updatedAt,
    layerSteps: value.layerSteps === undefined ? [] : value.layerSteps
  };
}

function isRecord(value: unknown): value is Record<string, unknown> {
  return value !== null && typeof value === 'object' && !Array.isArray(value);
}

function boundedText(value: unknown, minimum: number, maximum: number, trimmed = false): value is string {
  return typeof value === 'string' && value.length >= minimum && value.length <= maximum &&
    (!trimmed || value.trim().length >= minimum);
}

function finiteRange(value: unknown, minimum: number, maximum: number): boolean {
  return typeof value === 'number' && Number.isFinite(value) && value >= minimum && value <= maximum;
}

function integerRange(value: unknown, minimum: number, maximum: number): boolean {
  return Number.isSafeInteger(value) && Number(value) >= minimum && Number(value) <= maximum;
}

function byte(value: unknown): boolean { return integerRange(value, 0, 255); }
function positiveByte(value: unknown): boolean { return integerRange(value, 1, 255); }
