import type {
  EngineRegistration,
  PlannerRegistration,
  RestorationCapabilities,
  PlannerCapabilities
} from '../types/editor';

type Registration = PlannerRegistration | EngineRegistration;
type Capabilities = PlannerCapabilities | RestorationCapabilities;

export function componentStatus(component: Registration): 'Active' | 'Ready' | 'Unavailable' {
  if (component.active) return 'Active';
  return component.installed ? 'Ready' : 'Unavailable';
}

export function formatMemoryEstimate(megabytes: number): string {
  if (megabytes <= 0) return 'Not specified';
  if (megabytes >= 1024) {
    const gigabytes = megabytes / 1024;
    return `${Number.isInteger(gigabytes) ? gigabytes : gigabytes.toFixed(1)} GB estimated`;
  }
  return `${megabytes} MB estimated`;
}

export function splitModelDirectories(value: string): string[] {
  return value
    .split(/\r?\n/)
    .map((directory) => directory.trim())
    .filter(Boolean)
    .slice(0, 8);
}

export function capabilityLabels(capabilities: Capabilities): string[] {
  const labels: string[] = [];
  if (capabilities.offline) labels.push('Offline');
  if (capabilities.requiresModel) labels.push('Model required');
  if ('supportsGuidedEditing' in capabilities && capabilities.supportsGuidedEditing) {
    labels.push('Guided editing');
  }
  if ('supportsReasoning' in capabilities && capabilities.supportsReasoning) {
    labels.push('Reasoning');
  }
  if ('supportsRestoration' in capabilities && capabilities.supportsRestoration) {
    labels.push('Restoration');
  }
  if ('supportsNeuralModels' in capabilities && capabilities.supportsNeuralModels) {
    labels.push('Neural models');
  }
  if ('preservesAlpha' in capabilities && capabilities.preservesAlpha) {
    labels.push('Preserves alpha');
  }
  return labels;
}

export function formatNanoseconds(nanoseconds: number): string {
  if (nanoseconds <= 0) return '<1 ns';
  if (nanoseconds < 1_000) return `${Math.round(nanoseconds)} ns`;
  if (nanoseconds < 1_000_000) return `${(nanoseconds / 1_000).toFixed(2)} µs`;
  return `${(nanoseconds / 1_000_000).toFixed(2)} ms`;
}
