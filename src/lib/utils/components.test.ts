import { describe, expect, it } from 'vitest';
import type { EngineRegistration, PlannerRegistration } from '../types/editor';
import {
  capabilityLabels,
  componentStatus,
  formatMemoryEstimate,
  formatNanoseconds,
  splitModelDirectories
} from './components';

const planner: PlannerRegistration = {
  id: 'rule', name: 'Rule Planner', version: '0.4.0', provider: 'PhotoForge',
  memoryEstimateMb: 1, installed: true, loaded: true, active: true,
  unavailableReason: null,
  capabilities: { supportsGuidedEditing: true, supportsReasoning: false, requiresModel: false, offline: true }
};

const engine: EngineRegistration = {
  id: 'deterministic', name: 'Deterministic Engine', version: '0.4.0', provider: 'PhotoForge',
  memoryEstimateMb: 4, installed: true, loaded: true, active: true,
  unavailableReason: null,
  capabilities: { supportsRestoration: true, supportsNeuralModels: false, requiresModel: false, offline: true, preservesAlpha: true, maxInputMegapixels: 80 }
};

describe('component formatting utilities', () => {
  it('reports active components', () => expect(componentStatus(planner)).toBe('Active'));
  it('reports installed inactive components as ready', () => expect(componentStatus({ ...planner, active: false })).toBe('Ready'));
  it('reports missing components as unavailable', () => expect(componentStatus({ ...planner, active: false, installed: false })).toBe('Unavailable'));
  it('handles unspecified memory', () => expect(formatMemoryEstimate(0)).toBe('Not specified'));
  it('formats megabyte memory estimates', () => expect(formatMemoryEstimate(512)).toBe('512 MB estimated'));
  it('formats integral gigabyte estimates', () => expect(formatMemoryEstimate(2048)).toBe('2 GB estimated'));
  it('formats fractional gigabyte estimates', () => expect(formatMemoryEstimate(1536)).toBe('1.5 GB estimated'));
  it('splits and trims local directories', () => expect(splitModelDirectories(' C:\\one \nD:\\two')).toEqual(['C:\\one', 'D:\\two']));
  it('ignores blank model directory rows', () => expect(splitModelDirectories('\n \r\nmodels\n')).toEqual(['models']));
  it('bounds model directories to eight', () => expect(splitModelDirectories(Array.from({ length: 10 }, (_, index) => `m${index}`).join('\n'))).toHaveLength(8));
  it('labels built-in planner capabilities', () => expect(capabilityLabels(planner.capabilities)).toEqual(['Offline', 'Guided editing']));
  it('labels optional planner requirements', () => expect(capabilityLabels({ ...planner.capabilities, requiresModel: true, supportsReasoning: true })).toContain('Reasoning'));
  it('labels restoration and alpha support', () => expect(capabilityLabels(engine.capabilities)).toEqual(['Offline', 'Restoration', 'Preserves alpha']));
  it('labels values below timer resolution', () => expect(formatNanoseconds(0)).toBe('<1 ns'));
  it('formats nanoseconds directly', () => expect(formatNanoseconds(765)).toBe('765 ns'));
  it('formats microseconds', () => expect(formatNanoseconds(12_340)).toBe('12.34 µs'));
  it('formats milliseconds', () => expect(formatNanoseconds(2_500_000)).toBe('2.50 ms'));
});
