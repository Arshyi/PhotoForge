import { describe, expect, it } from 'vitest';
import type { EditPlan, GuidedSettings } from '../types/editor';
import {
  GUIDED_SETTINGS_KEY,
  MAX_RECENT_REQUESTS,
  RECENT_REQUESTS_KEY,
  clonePlan,
  confidenceLabel,
  defaultGuidedSettings,
  loadGuidedSettings,
  loadRecentRequests,
  movePlanOperation,
  planValueControl,
  rememberRecentRequest,
  removePlanOperation,
  saveGuidedSettings,
  suggestedPrompts,
  updatePlanOperation,
  withPlanValue
} from './guided';

class MemoryStore {
  values = new Map<string, string>();
  getItem(key: string) { return this.values.get(key) ?? null; }
  setItem(key: string, value: string) { this.values.set(key, value); }
  removeItem(key: string) { this.values.delete(key); }
}

function plan(): EditPlan {
  return {
    summary: 'Test plan',
    confidence: 0.8,
    warnings: ['Review the result.'],
    operations: [
      { type: 'denoise', strength: 0.3, preserve_edges: 0.8 },
      { type: 'edge_aware_sharpen', strength: 0.2, radius: 1, threshold: 0.04 }
    ],
    operationExplanations: ['Reduce noise.', 'Improve edges.']
  };
}

describe('guided editing utilities', () => {
  it.each(suggestedPrompts)('ships the suggested prompt: %s', (prompt) => {
    expect(prompt.length).toBeGreaterThan(4);
  });

  it('loads default settings when storage is empty', () => {
    expect(loadGuidedSettings(new MemoryStore())).toEqual(defaultGuidedSettings);
  });

  it('round-trips local settings', () => {
    const store = new MemoryStore();
    const settings: GuidedSettings = { ...defaultGuidedSettings, showWarnings: false };
    saveGuidedSettings(settings, store);
    expect(loadGuidedSettings(store)).toEqual(settings);
  });

  it('ignores malformed local settings', () => {
    const store = new MemoryStore();
    store.setItem(GUIDED_SETTINGS_KEY, '{');
    expect(loadGuidedSettings(store)).toEqual(defaultGuidedSettings);
  });

  it('clears prompt history when remembering is disabled', () => {
    const store = new MemoryStore();
    store.setItem(RECENT_REQUESTS_KEY, '["Reduce noise"]');
    saveGuidedSettings({ ...defaultGuidedSettings, rememberPromptHistory: false }, store);
    expect(store.getItem(RECENT_REQUESTS_KEY)).toBeNull();
  });

  it('deduplicates recent requests case-insensitively', () => {
    const next = rememberRecentRequest(
      [{ prompt: 'Reduce Noise', provider: 'Rule' }, { prompt: 'Fix lighting', provider: 'Rule' }],
      'reduce noise',
      'Rule',
      null
    );
    expect(next).toEqual([
      { prompt: 'reduce noise', provider: 'Rule' },
      { prompt: 'Fix lighting', provider: 'Rule' }
    ]);
  });

  it('bounds recent requests to 25', () => {
    let recent: Array<{ prompt: string; provider: 'Rule' | 'Ollama' }> = [];
    for (let index = 0; index < 30; index += 1) {
      recent = rememberRecentRequest(recent, `Request ${index}`, 'Rule', null);
    }
    expect(recent).toHaveLength(MAX_RECENT_REQUESTS);
    expect(recent[0].prompt).toBe('Request 29');
  });

  it('loads only valid recent request strings', () => {
    const store = new MemoryStore();
    store.setItem(RECENT_REQUESTS_KEY, JSON.stringify([' Good ', 4, '', 'Also good']));
    expect(loadRecentRequests(store)).toEqual([
      { prompt: 'Good', provider: 'Rule' },
      { prompt: 'Also good', provider: 'Rule' }
    ]);
  });

  it('labels confidence as low, medium, or high', () => {
    expect(confidenceLabel(0.2)).toBe('Low');
    expect(confidenceLabel(0.6)).toBe('Medium');
    expect(confidenceLabel(0.9)).toBe('High');
  });

  it('describes bounded plan controls', () => {
    expect(planValueControl({ type: 'brightness', amount: -0.1 })).toEqual({
      value: -0.1,
      min: -0.5,
      max: 0.5,
      step: 0.01,
      noun: 'amount'
    });
    expect(planValueControl({ type: 'grayscale' })).toBeNull();
  });

  it('updates amount and strength values without changing operation type', () => {
    expect(withPlanValue({ type: 'brightness', amount: 0.1 }, -0.2)).toEqual({
      type: 'brightness',
      amount: -0.2
    });
    expect(withPlanValue({ type: 'deblock', strength: 0.2 }, 0.6)).toEqual({
      type: 'deblock',
      strength: 0.6
    });
  });

  it('removes an operation and its matching explanation', () => {
    const next = removePlanOperation(plan(), 0);
    expect(next.operations).toHaveLength(1);
    expect(next.operationExplanations).toEqual(['Improve edges.']);
  });

  it('moves operations and explanations together', () => {
    const next = movePlanOperation(plan(), 1, -1);
    expect(next.operations[0].type).toBe('edge_aware_sharpen');
    expect(next.operationExplanations[0]).toBe('Improve edges.');
  });

  it('leaves a plan stable for an impossible move', () => {
    expect(movePlanOperation(plan(), 0, -1)).toEqual(plan());
  });

  it('updates one planned operation', () => {
    const next = updatePlanOperation(plan(), 0, {
      type: 'denoise',
      strength: 0.7,
      preserve_edges: 0.8
    });
    expect(next.operations[0]).toMatchObject({ strength: 0.7 });
  });

  it('deep-clones mutable plan arrays', () => {
    const source = plan();
    const copy = clonePlan(source);
    copy.warnings.push('New');
    copy.operations[0] = { type: 'grayscale' };
    expect(source.warnings).toHaveLength(1);
    expect(source.operations[0].type).toBe('denoise');
  });
});
