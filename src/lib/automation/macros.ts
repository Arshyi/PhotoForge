/**
 * Macros: recorded, inspectable, editable lists of registered operations.
 *
 * A macro is data. Each step is the id of an operation from the backend's registry,
 * its parameters, an on/off switch, and optionally one condition. Replaying it sends
 * the enabled steps to the backend as a single transaction — the same engine the
 * Layers panel, a plugin and the planner use — so it is atomic, it is validated after
 * every step, and it is one entry in the Undo history. There is no scripting: a step
 * cannot compute anything, loop, or call anything that is not in the registry.
 *
 * This file knows how to hold, check and reorder macros. It does not know what any
 * operation does; the registry says that, and the backend decides whether a macro is
 * really valid when it is run.
 */
import { blendModes } from '../layers/types';
import { validateEditOperation } from '../utils/workflows';
import type { Condition, LayerSelector, OperationCall, OperationSpec, ParamKind } from '../operations/types';
import type { PluginSummary } from '../plugins/types';

export const MACRO_SCHEMA_VERSION = 1;
export const MACRO_STORAGE_KEY = 'photoforge.macros.v1';
export const MAX_MACROS = 200;
export const MAX_MACRO_STEPS = 100;
export const MAX_MACRO_JSON_CHARACTERS = 1_000_000;
const MAX_NAME = 120;
const MAX_DESCRIPTION = 500;
const MAX_NOTE = 200;

export interface MacroStep {
  id: string;
  enabled: boolean;
  op: string;
  params: Record<string, unknown>;
  when: Condition | null;
  note: string;
}

export interface Macro {
  schemaVersion: 1;
  id: string;
  name: string;
  description: string;
  steps: MacroStep[];
  createdAt: string;
  modifiedAt: string;
}

const ALPHABET = 'abcdefghijklmnopqrstuvwxyz0123456789';

export function newId(prefix: string): string {
  const values = new Uint8Array(10);
  if (typeof globalThis.crypto?.getRandomValues === 'function') globalThis.crypto.getRandomValues(values);
  else for (let index = 0; index < values.length; index += 1) values[index] = Math.floor(Math.random() * 256);
  return prefix + [...values].map((value) => ALPHABET[value % ALPHABET.length]).join('');
}

export function newMacro(name: string, now: Date = new Date()): Macro {
  const stamp = now.toISOString();
  return { schemaVersion: 1, id: newId('m'), name: name.trim() || 'Untitled macro', description: '', steps: [], createdAt: stamp, modifiedAt: stamp };
}

export function newStep(op: string, params: Record<string, unknown> = {}, when: Condition | null = null): MacroStep {
  return { id: newId('s'), enabled: true, op, params: structuredClone(params), when: when ? structuredClone(when) : null, note: '' };
}

function touch(macro: Macro, steps: MacroStep[], now: Date = new Date()): Macro {
  return { ...macro, steps, modifiedAt: now.toISOString() };
}

// ---- editing: every function returns a new macro and leaves the old one alone ------

export function insertStep(macro: Macro, step: MacroStep, index = macro.steps.length): Macro {
  if (macro.steps.length >= MAX_MACRO_STEPS) throw new Error(`A macro may have at most ${MAX_MACRO_STEPS} steps.`);
  const steps = [...macro.steps];
  steps.splice(Math.max(0, Math.min(index, steps.length)), 0, step);
  return touch(macro, steps);
}

export function removeStep(macro: Macro, stepId: string): Macro {
  return touch(macro, macro.steps.filter((step) => step.id !== stepId));
}

export function moveStep(macro: Macro, stepId: string, delta: number): Macro {
  const from = macro.steps.findIndex((step) => step.id === stepId);
  if (from < 0) return macro;
  const to = Math.max(0, Math.min(macro.steps.length - 1, from + delta));
  if (to === from) return macro;
  const steps = [...macro.steps];
  const [moved] = steps.splice(from, 1);
  steps.splice(to, 0, moved);
  return touch(macro, steps);
}

export function duplicateStep(macro: Macro, stepId: string): Macro {
  const index = macro.steps.findIndex((step) => step.id === stepId);
  if (index < 0) return macro;
  const copy: MacroStep = { ...structuredClone(macro.steps[index]), id: newId('s') };
  return insertStep(macro, copy, index + 1);
}

export function updateStep(macro: Macro, stepId: string, change: Partial<Omit<MacroStep, 'id'>>): Macro {
  return touch(macro, macro.steps.map((step) => (step.id === stepId ? { ...step, ...structuredClone(change) } : step)));
}

export function setStepEnabled(macro: Macro, stepId: string, enabled: boolean): Macro {
  return updateStep(macro, stepId, { enabled });
}

/** Changes the name or the description, which is a modification like any other. */
export function setMacroDetails(macro: Macro, details: { name?: string; description?: string }, now: Date = new Date()): Macro {
  return {
    ...macro,
    ...(details.name !== undefined ? { name: details.name } : {}),
    ...(details.description !== undefined ? { description: details.description } : {}),
    modifiedAt: now.toISOString()
  };
}

export function duplicateMacro(macro: Macro, now: Date = new Date()): Macro {
  const stamp = now.toISOString();
  return {
    ...structuredClone(macro),
    id: newId('m'),
    name: `${macro.name} copy`.slice(0, MAX_NAME),
    steps: macro.steps.map((step) => ({ ...structuredClone(step), id: newId('s') })),
    createdAt: stamp,
    modifiedAt: stamp
  };
}

/** The calls a macro stands for: its enabled steps, in order, with their conditions. */
export function toCalls(macro: Macro): OperationCall[] {
  return macro.steps
    .filter((step) => step.enabled)
    .map((step) => ({
      op: step.op,
      params: structuredClone(step.params),
      ...(step.when ? { when: structuredClone(step.when) } : {})
    }));
}

// ---- checking ----------------------------------------------------------------------

const SELECTOR_TYPES = ['id', 'active', 'last_created', 'name', 'bottom', 'top'];
const LAYER_KINDS = ['pixel', 'group', 'adjustment', 'shape', 'text', 'smart_object'];

function isRecord(value: unknown): value is Record<string, unknown> {
  return typeof value === 'object' && value !== null && !Array.isArray(value);
}

export function validateSelector(value: unknown): string | null {
  if (!isRecord(value) || typeof value.type !== 'string' || !SELECTOR_TYPES.includes(value.type)) {
    return 'is not a layer selector';
  }
  const field = value.type === 'id' ? 'id' : value.type === 'name' ? 'name' : null;
  if (Object.keys(value).some((key) => key !== 'type' && key !== field)) return 'has fields a selector does not';
  if (field) {
    const text = value[field];
    if (typeof text !== 'string' || !text.trim() || text.length > (field === 'id' ? 64 : 120)) {
      return `needs a ${field} of 1 to ${field === 'id' ? 64 : 120} characters`;
    }
  }
  return null;
}

/** Mirrors `Condition::validate` in the backend. */
export function validateCondition(value: unknown, depth = 0): string | null {
  if (!isRecord(value) || typeof value.kind !== 'string') return 'the condition is not one';
  const only = (...keys: string[]) => Object.keys(value).every((key) => key === 'kind' || keys.includes(key));
  switch (value.kind) {
    case 'layer_exists': {
      if (!only('selector')) return 'the condition has fields it does not use';
      const problem = validateSelector(value.selector);
      return problem ? `the condition's layer ${problem}` : null;
    }
    case 'layer_count_at_least':
      return only('count') && Number.isInteger(value.count) && (value.count as number) >= 1 && (value.count as number) <= 512
        ? null
        : 'the layer count must be a whole number from 1 to 512';
    case 'active_layer_kind':
      return only('layerKind') && LAYER_KINDS.includes(String(value.layerKind)) ? null : 'that is not a kind of layer';
    case 'precision':
      return only('precision') && ['linear_srgb_f32', 'legacy_srgb8'].includes(String(value.precision))
        ? null
        : 'that is not a document precision';
    case 'not':
      if (!only('condition')) return 'the condition has fields it does not use';
      if (depth >= 2) return 'not may be nested at most 2 deep';
      return validateCondition(value.condition, depth + 1);
    default:
      return `${value.kind} is not a condition`;
  }
}

function selectorProblem(name: string, value: unknown): string | null {
  const problem = validateSelector(value);
  return problem ? `${name} ${problem}` : null;
}

function validateParam(spec: OperationSpec['params'][number], value: unknown): string | null {
  const kind = spec.kind;
  switch (kind.kind) {
    case 'selector':
      return selectorProblem(spec.name, value);
    case 'optionalSelector':
      return value === null || value === undefined ? null : selectorProblem(spec.name, value);
    case 'selectors':
      if (!Array.isArray(value) || value.length < 1 || value.length > 100) return `${spec.name} needs 1 to 100 layers`;
      for (const entry of value) {
        const problem = validateSelector(entry);
        if (problem) return `${spec.name} ${problem}`;
      }
      return null;
    case 'text':
      return typeof value === 'string' && value.trim().length >= 1 && value.length <= kind.maxChars
        ? null
        : `${spec.name} needs 1 to ${kind.maxChars} characters`;
    case 'bool':
      return typeof value === 'boolean' ? null : `${spec.name} must be true or false`;
    case 'number':
      return typeof value === 'number' && Number.isFinite(value) && value >= kind.min && value <= kind.max
        ? null
        : `${spec.name} must be between ${kind.min} and ${kind.max}`;
    case 'integer':
      return typeof value === 'number' && Number.isInteger(value) && value >= kind.min && value <= kind.max
        ? null
        : `${spec.name} must be a whole number from ${kind.min} to ${kind.max}`;
    case 'blendMode':
      return typeof value === 'string' && blendModes.some((mode) => mode.id === value) ? null : `${spec.name} is not a blend mode`;
    case 'editOperation': {
      const problem = validateEditOperation(value);
      return problem ? `${spec.name}: ${problem}` : null;
    }
    case 'editOperations': {
      if (!Array.isArray(value) || value.length < 1 || value.length > 100) return `${spec.name} needs 1 to 100 edits`;
      for (const entry of value) {
        const problem = validateEditOperation(entry);
        if (problem) return `${spec.name}: ${problem}`;
      }
      return null;
    }
    case 'json':
      return null;
  }
}

/**
 * What is wrong with a macro, as sentences that name the step. The backend checks
 * again, and is the authority; this is so the editor can say so while it is being
 * edited, not only when it is run.
 */
export function validateMacro(macro: Macro, specs: OperationSpec[]): string[] {
  const problems: string[] = [];
  if (!macro.name.trim() || macro.name.length > MAX_NAME) problems.push(`The name must contain 1 to ${MAX_NAME} characters.`);
  if (macro.description.length > MAX_DESCRIPTION) problems.push(`The description may be at most ${MAX_DESCRIPTION} characters.`);
  if (macro.steps.length > MAX_MACRO_STEPS) problems.push(`A macro may have at most ${MAX_MACRO_STEPS} steps.`);
  const seen = new Set<string>();
  macro.steps.forEach((step, index) => {
    const label = `Step ${index + 1}`;
    if (seen.has(step.id)) problems.push(`${label} shares its identifier with another step.`);
    seen.add(step.id);
    if (step.note.length > MAX_NOTE) problems.push(`${label}'s note is longer than ${MAX_NOTE} characters.`);
    const spec = specs.find((candidate) => candidate.id === step.op);
    if (!spec) {
      problems.push(`${label}: ${step.op} is not an operation this version of PhotoForge has.`);
      return;
    }
    for (const key of Object.keys(step.params)) {
      if (!spec.params.some((param) => param.name === key)) problems.push(`${label} (${spec.title}): ${key} is not a parameter of it.`);
    }
    for (const param of spec.params) {
      const value = step.params[param.name];
      if (value === undefined || (value === null && param.kind.kind !== 'optionalSelector')) {
        if (param.required) problems.push(`${label} (${spec.title}): ${param.name} is required.`);
        continue;
      }
      const problem = validateParam(param, value);
      if (problem) problems.push(`${label} (${spec.title}): ${problem}.`);
    }
    if (step.when) {
      const problem = validateCondition(step.when);
      if (problem) problems.push(`${label} (${spec.title}): ${problem}.`);
    }
  });
  return problems;
}

// ---- describing --------------------------------------------------------------------

export function describeSelector(selector: LayerSelector): string {
  switch (selector.type) {
    case 'id':
      return `layer ${selector.id}`;
    case 'name':
      return `the layer named “${selector.name}”`;
    case 'active':
      return 'the selected layer';
    case 'last_created':
      return 'the layer the last step made';
    case 'bottom':
      return 'the bottom layer';
    case 'top':
      return 'the top layer';
  }
}

export function describeCondition(condition: Condition): string {
  switch (condition.kind) {
    case 'layer_exists':
      return `${describeSelector(condition.selector)} exists`;
    case 'layer_count_at_least':
      return `the document has at least ${condition.count} layer${condition.count === 1 ? '' : 's'}`;
    case 'active_layer_kind':
      return `the selected layer is a ${condition.layerKind.replace('_', ' ')} layer`;
    case 'precision':
      return condition.precision === 'linear_srgb_f32' ? 'the document is linear float' : 'the document is legacy 8-bit';
    case 'not':
      return `not (${describeCondition(condition.condition)})`;
  }
}

function summariseValue(value: unknown): string {
  if (isRecord(value) && typeof value.type === 'string' && SELECTOR_TYPES.includes(value.type)) {
    return describeSelector(value as unknown as LayerSelector);
  }
  if (Array.isArray(value)) return `${value.length} item${value.length === 1 ? '' : 's'}`;
  if (isRecord(value)) return value.type ? String(value.type) : 'settings';
  return String(value);
}

/** A line saying what a step does, from its parameters. */
export function describeStep(step: MacroStep, specs: OperationSpec[]): { title: string; summary: string } {
  const spec = specs.find((candidate) => candidate.id === step.op);
  const parts = Object.entries(step.params).map(([key, value]) => `${key}: ${summariseValue(value)}`);
  return { title: spec?.title ?? step.op, summary: parts.join(' · ') };
}

/** The plugin ids a macro's steps use, so a person is told before it runs without them. */
export function macroPluginDependencies(macro: Macro): string[] {
  const found = new Set<string>();
  const scan = (value: unknown) => {
    if (!isRecord(value)) return;
    if (value.type === 'plugin_filter' && typeof value.plugin === 'string') found.add(value.plugin);
    if (isRecord(value.operation)) scan(value.operation);
  };
  for (const step of macro.steps) {
    if (step.op.startsWith('core.plugin.') && typeof step.params.plugin === 'string') found.add(step.params.plugin);
    scan(step.params.operation);
    if (Array.isArray(step.params.operations)) step.params.operations.forEach(scan);
  }
  return [...found].sort();
}

// ---- storage and files ---------------------------------------------------------------

function normaliseStep(value: unknown): MacroStep | null {
  if (!isRecord(value) || typeof value.op !== 'string' || !isRecord(value.params)) return null;
  const when = value.when === undefined || value.when === null ? null : (value.when as Condition);
  if (when && validateCondition(when)) return null;
  return {
    id: typeof value.id === 'string' && value.id.length > 0 && value.id.length <= 64 ? value.id : newId('s'),
    enabled: value.enabled !== false,
    op: value.op,
    params: structuredClone(value.params),
    when,
    note: typeof value.note === 'string' ? value.note.slice(0, MAX_NOTE) : ''
  };
}

/** Reads one macro from untrusted JSON, or `null` if it is not a macro. */
export function normaliseMacro(value: unknown): Macro | null {
  if (!isRecord(value) || value.schemaVersion !== MACRO_SCHEMA_VERSION) return null;
  if (typeof value.name !== 'string' || !Array.isArray(value.steps) || value.steps.length > MAX_MACRO_STEPS) return null;
  const steps = value.steps.map(normaliseStep);
  if (steps.some((step) => step === null)) return null;
  const stamp = new Date().toISOString();
  return {
    schemaVersion: 1,
    id: typeof value.id === 'string' && value.id.length > 0 && value.id.length <= 64 ? value.id : newId('m'),
    name: value.name.slice(0, MAX_NAME) || 'Untitled macro',
    description: typeof value.description === 'string' ? value.description.slice(0, MAX_DESCRIPTION) : '',
    steps: steps as MacroStep[],
    createdAt: typeof value.createdAt === 'string' ? value.createdAt : stamp,
    modifiedAt: typeof value.modifiedAt === 'string' ? value.modifiedAt : stamp
  };
}

export function loadMacros(storage: Pick<Storage, 'getItem'> = localStorage): Macro[] {
  try {
    const raw = storage.getItem(MACRO_STORAGE_KEY);
    if (!raw || raw.length > MAX_MACRO_JSON_CHARACTERS * 4) return [];
    const parsed: unknown = JSON.parse(raw);
    if (!Array.isArray(parsed)) return [];
    return parsed.slice(0, MAX_MACROS).map(normaliseMacro).filter((macro): macro is Macro => macro !== null);
  } catch {
    return [];
  }
}

/** Saves the macros. Returns false if the browser would not hold them, so the caller can say so. */
export function saveMacros(macros: Macro[], storage: Pick<Storage, 'setItem'> = localStorage): boolean {
  try {
    storage.setItem(MACRO_STORAGE_KEY, JSON.stringify(macros.slice(0, MAX_MACROS)));
    return true;
  } catch {
    return false;
  }
}

export interface MacroDocument {
  schemaVersion: 1;
  kind: 'photoforge-macro';
  macro: Macro;
}

export function macroDocument(macro: Macro): MacroDocument {
  return { schemaVersion: 1, kind: 'photoforge-macro', macro };
}

/** Reads an exported macro file, refusing anything that is not exactly one. */
export function parseMacroDocument(text: string): Macro {
  if (text.length > MAX_MACRO_JSON_CHARACTERS) throw new Error('The macro file is larger than PhotoForge will read.');
  let value: unknown;
  try {
    value = JSON.parse(text);
  } catch {
    throw new Error('The file is not a macro: it is not valid JSON.');
  }
  if (!isRecord(value) || value.kind !== 'photoforge-macro' || value.schemaVersion !== MACRO_SCHEMA_VERSION) {
    throw new Error('The file is not a PhotoForge macro of a version this build reads.');
  }
  const macro = normaliseMacro(value.macro);
  if (!macro) throw new Error('The macro in the file is malformed.');
  // An imported macro is a new one: it never overwrites the one it was exported from.
  return { ...macro, id: newId('m') };
}

// ---- starting values and plugin status -----------------------------------------------

/** A value of this kind that passes the checks, for a parameter just switched on. */
export function defaultValue(kind: ParamKind): unknown {
  switch (kind.kind) {
    case 'selector':
    case 'optionalSelector':
      return { type: 'active' };
    case 'selectors':
      return [{ type: 'active' }];
    case 'text':
      return '';
    case 'bool':
      return false;
    case 'number':
    case 'integer':
      return Math.min(Math.max(0, kind.min), kind.max);
    case 'blendMode':
      return 'normal';
    case 'editOperation':
      return { type: 'contrast', amount: 0 };
    case 'editOperations':
      return [{ type: 'contrast', amount: 0 }];
    case 'json':
      return {};
  }
}

/** The parameters a new step of this operation starts with: every required one. */
export function defaultParams(spec: OperationSpec): Record<string, unknown> {
  const params: Record<string, unknown> = {};
  for (const param of spec.params) {
    if (!param.required) continue;
    const value = defaultValue(param.kind);
    // A required text has no sensible default; the empty string is reported as missing.
    params[param.name] = value;
  }
  return params;
}

export type DependencyState = 'ready' | 'turned_off' | 'missing' | 'unavailable';

export interface DependencyStatus {
  id: string;
  name: string;
  state: DependencyState;
  /** Why it cannot run, in words, when it cannot. */
  reason: string;
}

/** For each plugin a macro uses, whether it can run now, and if not, why. */
export function dependencyStatus(ids: string[], plugins: PluginSummary[]): DependencyStatus[] {
  return ids.map((id) => {
    const plugin = plugins.find((candidate) => candidate.id === id);
    const name = plugin?.manifest?.name ?? id;
    if (!plugin) return { id, name, state: 'missing', reason: 'It is not installed.' };
    const availability = plugin.availability;
    switch (availability.kind) {
      case 'available':
        return { id, name, state: 'ready', reason: '' };
      case 'disabled':
        return { id, name, state: 'turned_off', reason: 'It is turned off.' };
      case 'missing':
        return { id, name, state: 'missing', reason: 'It is not installed.' };
      case 'notGranted':
        return { id, name, state: 'unavailable', reason: `It has not been given the ${availability.capability} permission it needs.` };
      case 'runtimeUnavailable':
        return { id, name, state: 'unavailable', reason: 'This build of PhotoForge cannot run plugins.' };
      case 'damaged':
        return { id, name, state: 'unavailable', reason: `Its package is damaged: ${availability.reason}` };
      case 'otherVersion':
        return {
          id,
          name,
          state: 'unavailable',
          reason: `Version ${availability.installedVersion} is installed, not the version this macro was made with.`
        };
    }
  });
}
