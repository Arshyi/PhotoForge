import type {
  EditOperation,
  EditPlan,
  GuidedHistoryEntry,
  GuidedSettings
} from '../types/editor';

export const MAX_RECENT_REQUESTS = 25;
export const GUIDED_SETTINGS_KEY = 'photoforge.guided-settings.v1';
export const RECENT_REQUESTS_KEY = 'photoforge.guided-recent.v1';

export const defaultGuidedSettings: GuidedSettings = {
  showWarnings: true,
  showConfidence: true,
  autoOpenPlanInspector: true,
  rememberPromptHistory: true
};

export const suggestedPrompts = [
  'Fix indoor lighting',
  'Improve this scan',
  'Make handwriting easier to read',
  'Reduce JPEG artifacts',
  'Reduce noise',
  'Sharpen slightly',
  'Improve without changing colors',
  'Make colors more natural',
  'Fix uneven lighting',
  'Clean up this receipt'
] as const;

type LocalStore = Pick<Storage, 'getItem' | 'setItem' | 'removeItem'>;

function browserStorage(): LocalStore | null {
  try {
    return typeof localStorage === 'undefined' ? null : localStorage;
  } catch {
    return null;
  }
}

export function loadGuidedSettings(store: LocalStore | null = browserStorage()): GuidedSettings {
  if (!store) return { ...defaultGuidedSettings };
  try {
    const parsed = JSON.parse(store.getItem(GUIDED_SETTINGS_KEY) ?? '{}') as Partial<GuidedSettings>;
    return {
      showWarnings:
        typeof parsed.showWarnings === 'boolean'
          ? parsed.showWarnings
          : defaultGuidedSettings.showWarnings,
      showConfidence:
        typeof parsed.showConfidence === 'boolean'
          ? parsed.showConfidence
          : defaultGuidedSettings.showConfidence,
      autoOpenPlanInspector:
        typeof parsed.autoOpenPlanInspector === 'boolean'
          ? parsed.autoOpenPlanInspector
          : defaultGuidedSettings.autoOpenPlanInspector,
      rememberPromptHistory:
        typeof parsed.rememberPromptHistory === 'boolean'
          ? parsed.rememberPromptHistory
          : defaultGuidedSettings.rememberPromptHistory
    };
  } catch {
    return { ...defaultGuidedSettings };
  }
}

export function saveGuidedSettings(
  settings: GuidedSettings,
  store: LocalStore | null = browserStorage()
): void {
  if (!store) return;
  try {
    store.setItem(GUIDED_SETTINGS_KEY, JSON.stringify(settings));
    if (!settings.rememberPromptHistory) store.removeItem(RECENT_REQUESTS_KEY);
  } catch {
    // Local preferences are optional and must never block editing.
  }
}

export function loadRecentRequests(
  store: LocalStore | null = browserStorage()
): GuidedHistoryEntry[] {
  if (!store) return [];
  try {
    const parsed = JSON.parse(store.getItem(RECENT_REQUESTS_KEY) ?? '[]') as unknown;
    if (!Array.isArray(parsed)) return [];
    return parsed
      .map((value): GuidedHistoryEntry | null => {
        if (typeof value === 'string' && value.trim()) {
          return { prompt: value.trim().slice(0, 1_000), provider: 'Rule' };
        }
        if (!value || typeof value !== 'object') return null;
        const candidate = value as Partial<GuidedHistoryEntry>;
        if (typeof candidate.prompt !== 'string' || !candidate.prompt.trim()) return null;
        return {
          prompt: candidate.prompt.trim().slice(0, 1_000),
          provider: candidate.provider === 'Ollama' ? 'Ollama' : 'Rule'
        };
      })
      .filter((value): value is GuidedHistoryEntry => value !== null)
      .slice(0, MAX_RECENT_REQUESTS);
  } catch {
    return [];
  }
}

export function rememberRecentRequest(
  recent: GuidedHistoryEntry[],
  request: string,
  provider: GuidedHistoryEntry['provider'] = 'Rule',
  store: LocalStore | null = browserStorage()
): GuidedHistoryEntry[] {
  const normalized = request.trim().slice(0, 1_000);
  if (!normalized) return recent.slice(0, MAX_RECENT_REQUESTS);
  const next = [{ prompt: normalized, provider }, ...recent.filter(
    (candidate) =>
      candidate.prompt.toLocaleLowerCase() !== normalized.toLocaleLowerCase() ||
      candidate.provider !== provider
  )].slice(0, MAX_RECENT_REQUESTS);
  try {
    store?.setItem(RECENT_REQUESTS_KEY, JSON.stringify(next));
  } catch {
    // The in-memory history still works when persistent storage is unavailable.
  }
  return next;
}

export function confidenceLabel(confidence: number): 'Low' | 'Medium' | 'High' {
  if (confidence >= 0.78) return 'High';
  if (confidence >= 0.55) return 'Medium';
  return 'Low';
}

export interface PlanValueControl {
  value: number;
  min: number;
  max: number;
  step: number;
  noun: 'amount' | 'strength';
}

export function planValueControl(operation: EditOperation): PlanValueControl | null {
  switch (operation.type) {
    case 'brightness':
    case 'contrast':
      return { value: operation.amount, min: -0.5, max: 0.5, step: 0.01, noun: 'amount' };
    case 'saturation':
      return { value: operation.amount, min: -1, max: 1, step: 0.01, noun: 'amount' };
    case 'auto_white_balance':
    case 'denoise':
    case 'deblock':
    case 'mild_deblur':
    case 'document_enhance':
    case 'uneven_lighting_correction':
      return { value: operation.strength, min: 0, max: 1, step: 0.01, noun: 'strength' };
    case 'local_contrast':
      return { value: operation.strength, min: 0, max: 1, step: 0.01, noun: 'strength' };
    case 'edge_aware_sharpen':
      return { value: operation.strength, min: 0, max: 2, step: 0.01, noun: 'strength' };
    default:
      return null;
  }
}

export function withPlanValue(operation: EditOperation, value: number): EditOperation {
  if ('amount' in operation) return { ...operation, amount: value };
  if ('strength' in operation) return { ...operation, strength: value } as EditOperation;
  return { ...operation };
}

export function removePlanOperation(plan: EditPlan, index: number): EditPlan {
  return {
    ...plan,
    operations: plan.operations.filter((_, candidate) => candidate !== index),
    operationExplanations: plan.operationExplanations.filter(
      (_, candidate) => candidate !== index
    )
  };
}

export function movePlanOperation(plan: EditPlan, index: number, delta: -1 | 1): EditPlan {
  const destination = index + delta;
  if (index < 0 || index >= plan.operations.length || destination < 0 || destination >= plan.operations.length) {
    return clonePlan(plan);
  }
  const operations = plan.operations.map((operation) => ({ ...operation })) as EditOperation[];
  const explanations = [...plan.operationExplanations];
  [operations[index], operations[destination]] = [operations[destination], operations[index]];
  [explanations[index], explanations[destination]] = [
    explanations[destination],
    explanations[index]
  ];
  return { ...plan, operations, operationExplanations: explanations };
}

export function updatePlanOperation(
  plan: EditPlan,
  index: number,
  operation: EditOperation
): EditPlan {
  if (index < 0 || index >= plan.operations.length) return clonePlan(plan);
  const operations = plan.operations.map((candidate) => ({ ...candidate })) as EditOperation[];
  operations[index] = operation;
  return { ...plan, operations };
}

export function clonePlan(plan: EditPlan): EditPlan {
  return {
    ...plan,
    warnings: [...plan.warnings],
    operations: plan.operations.map((operation) => ({ ...operation })) as EditOperation[],
    operationExplanations: [...plan.operationExplanations]
  };
}
