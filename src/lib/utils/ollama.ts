import type { ComponentConfiguration, PlanValidationReport } from '../types/editor';

export const OLLAMA_DEFAULTS = Object.freeze({
  endpoint: 'http://127.0.0.1:11434',
  timeoutMs: 15_000,
  maximumResponseBytes: 256 * 1_024,
  maximumOperations: 8
});

export function resetOllamaConfiguration(
  configuration: ComponentConfiguration
): ComponentConfiguration {
  return {
    ...configuration,
    plannerEndpoint: OLLAMA_DEFAULTS.endpoint,
    ollamaTimeoutMs: OLLAMA_DEFAULTS.timeoutMs,
    ollamaMaxResponseBytes: OLLAMA_DEFAULTS.maximumResponseBytes,
    ollamaSelectedModel: null,
    ollamaMaxOperations: OLLAMA_DEFAULTS.maximumOperations
  };
}
export function normalizeOllamaConfiguration(
  configuration: ComponentConfiguration
): ComponentConfiguration {
  const selected = configuration.ollamaSelectedModel?.trim() || null;
  return {
    ...configuration,
    plannerEndpoint: configuration.plannerEndpoint.trim(),
    ollamaTimeoutMs: Math.min(120_000, Math.max(250, Math.round(configuration.ollamaTimeoutMs))),
    ollamaMaxResponseBytes: Math.min(
      2_097_152,
      Math.max(1_024, Math.round(configuration.ollamaMaxResponseBytes))
    ),
    ollamaSelectedModel: selected,
    ollamaMaxOperations: Math.min(8, Math.max(1, Math.round(configuration.ollamaMaxOperations)))
  };
}

export function isOllamaConfigured(selectedModel: string | null | undefined): boolean {
  return Boolean(selectedModel?.trim());
}

export function isLocalOllamaEndpoint(endpoint: string): boolean {
  try {
    const parsed = new URL(endpoint.trim());
    return (
      parsed.protocol === 'http:' &&
      ['', '/'].includes(parsed.pathname) &&
      !parsed.username &&
      !parsed.password &&
      !parsed.search &&
      !parsed.hash &&
      ['127.0.0.1', 'localhost', '[::1]'].includes(parsed.hostname)
    );
  } catch {
    return false;
  }
}

export function formatOllamaModelSize(bytes: number): string {
  if (!Number.isFinite(bytes) || bytes <= 0) return 'Size unavailable';
  const units = ['B', 'KiB', 'MiB', 'GiB', 'TiB'];
  const exponent = Math.min(Math.floor(Math.log(bytes) / Math.log(1024)), units.length - 1);
  const value = bytes / 1024 ** exponent;
  const digits = exponent === 0 || value >= 10 ? 0 : 1;
  return `${value.toFixed(digits)} ${units[exponent]}`;
}

export function formatOllamaModifiedDate(value: string): string {
  const parsed = new Date(value);
  return Number.isNaN(parsed.valueOf()) ? 'Modified date unavailable' : parsed.toISOString().slice(0, 10);
}

export function modelCapabilitySummary(capabilities: string[]): string {
  const unique = Array.from(
    new Set(capabilities.map((capability) => capability.trim()).filter(Boolean))
  );
  return unique.length ? unique.slice(0, 6).join(' · ') : 'Capabilities not reported';
}

export function validationStatus(report: PlanValidationReport | null): string {
  if (!report) return 'Not validated';
  if (report.valid) return 'Valid plan';
  const count = report.errors.length + report.rejectedFields.length;
  return count === 1 ? 'Rejected · 1 issue' : `Rejected · ${count} issues`;
}
