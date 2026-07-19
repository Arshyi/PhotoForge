import { describe, expect, it } from 'vitest';
import type { ComponentConfiguration, PlanValidationReport } from '../types/editor';
import {
  OLLAMA_DEFAULTS,
  formatOllamaModelSize,
  formatOllamaModifiedDate,
  isLocalOllamaEndpoint,
  isOllamaConfigured,
  modelCapabilitySummary,
  normalizeOllamaConfiguration,
  resetOllamaConfiguration,
  validationStatus
} from './ollama';

function configuration(): ComponentConfiguration {
  return {
    activePlanner: 'rule', activeEngine: 'deterministic',
    plannerEndpoint: ' http://localhost:11434 ', initializationTimeoutMs: 5000,
    ollamaTimeoutMs: 15000, ollamaMaxResponseBytes: 262144,
    ollamaSelectedModel: ' gemma3:4b ', ollamaMaxOperations: 8,
    modelDirectories: ['models'], pluginDirectory: 'plugins'
  };
}

describe('Ollama UI utilities', () => {
  it.each([
    [0, 'Size unavailable'],
    [-1, 'Size unavailable'],
    [512, '512 B'],
    [1024, '1.0 KiB'],
    [10 * 1024 * 1024, '10 MiB'],
    [4.5 * 1024 ** 3, '4.5 GiB']
  ])('formats model size %s', (bytes, expected) => {
    expect(formatOllamaModelSize(bytes)).toBe(expected);
  });

  it.each([
    ['2026-07-19T12:30:00Z', '2026-07-19'],
    ['2025-01-02', '2025-01-02'],
    ['', 'Modified date unavailable'],
    ['not-a-date', 'Modified date unavailable']
  ])('formats modified date %s', (value, expected) => {
    expect(formatOllamaModifiedDate(value)).toBe(expected);
  });

  it.each([
    ['http://127.0.0.1:11434', true],
    ['http://localhost:11434', true],
    ['http://[::1]:11434', true],
    ['https://127.0.0.1:11434', false],
    ['http://192.168.1.2:11434', false],
    ['http://ollama.example:11434', false],
    ['http://user@127.0.0.1:11434', false],
    ['http://127.0.0.1:11434/api', false],
    ['http://127.0.0.1:11434?token=x', false],
    ['', false]
  ])('previews local endpoint validation for %s', (endpoint, expected) => {
    expect(isLocalOllamaEndpoint(endpoint)).toBe(expected);
  });

  it.each([
    [[], 'Capabilities not reported'],
    [['completion'], 'completion'],
    [[' completion ', 'vision', 'completion'], 'completion · vision'],
    [['a', 'b', 'c', 'd', 'e', 'f', 'g'], 'a · b · c · d · e · f']
  ])('summarizes model capabilities', (capabilities, expected) => {
    expect(modelCapabilitySummary(capabilities)).toBe(expected);
  });

  it.each([
    [null, 'Not validated'],
    [{ valid: true, errors: [], rejectedFields: [] }, 'Valid plan'],
    [{ valid: false, errors: ['bad'], rejectedFields: ['extra'] }, 'Rejected · 2 issues']
  ])('formats validation status', (partial, expected) => {
    const report = partial
      ? ({ originalResponse: '', validatedResponse: null, validationTimeMs: 0, ...partial } as PlanValidationReport)
      : null;
    expect(validationStatus(report)).toBe(expected);
  });

  it('resets every Ollama field without changing unrelated component settings', () => {
    const reset = resetOllamaConfiguration(configuration());
    expect(reset).toMatchObject({
      plannerEndpoint: OLLAMA_DEFAULTS.endpoint,
      ollamaTimeoutMs: OLLAMA_DEFAULTS.timeoutMs,
      ollamaMaxResponseBytes: OLLAMA_DEFAULTS.maximumResponseBytes,
      ollamaSelectedModel: null,
      ollamaMaxOperations: OLLAMA_DEFAULTS.maximumOperations,
      activeEngine: 'deterministic'
    });
  });

  it.each([
    [{ ollamaTimeoutMs: 1 }, { ollamaTimeoutMs: 250 }],
    [{ ollamaTimeoutMs: 999999 }, { ollamaTimeoutMs: 120000 }],
    [{ ollamaMaxResponseBytes: 1 }, { ollamaMaxResponseBytes: 1024 }],
    [{ ollamaMaxOperations: 99 }, { ollamaMaxOperations: 8 }],
    [{ ollamaMaxOperations: -3 }, { ollamaMaxOperations: 1 }]
  ])('normalizes bounded settings', (input, expected) => {
    expect(normalizeOllamaConfiguration({ ...configuration(), ...input })).toMatchObject(expected);
  });

  it.each([
    ['gemma3:4b', true],
    ['   ', false]
  ])('detects whether a planner model is configured', (model, expected) => {
    expect(isOllamaConfigured(model)).toBe(expected);
  });
});
