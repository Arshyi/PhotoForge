/**
 * Helpers for the parameters a plugin declares.
 *
 * The backend checks every value again before it reaches a module; these exist so
 * the dialog never offers one it would refuse, and mirror `ParamDecl::accepts`.
 */
import type { Locality, ParamDecl } from './types';

export function defaultValue(param: ParamDecl): number {
  switch (param.type) {
    case 'bool':
      return param.default ? 1 : 0;
    default:
      return param.default;
  }
}

export function defaultValues(params: ParamDecl[] | undefined): Record<string, number> {
  return Object.fromEntries((params ?? []).map((param) => [param.id, defaultValue(param)]));
}

export function accepts(param: ParamDecl, value: number): boolean {
  if (!Number.isFinite(value)) return false;
  switch (param.type) {
    case 'number':
      return value >= param.min && value <= param.max;
    case 'integer':
      return Number.isInteger(value) && value >= param.min && value <= param.max;
    case 'bool':
      return value === 0 || value === 1;
    case 'choice':
      return Number.isInteger(value) && value >= 0 && value < param.options.length;
  }
}

/**
 * The values a dialog starts from: the remembered ones that are still valid, and the
 * declared default for anything else. A value that no longer fits (the plugin was
 * updated and the range moved) falls back instead of being offered.
 */
export function initialValues(
  params: ParamDecl[] | undefined,
  remembered: Record<string, number> = {}
): Record<string, number> {
  const values: Record<string, number> = {};
  for (const param of params ?? []) {
    const earlier = remembered[param.id];
    values[param.id] = earlier !== undefined && accepts(param, earlier) ? earlier : defaultValue(param);
  }
  return values;
}

/** The first parameter whose value would be refused, as a sentence, or null. */
export function firstProblem(params: ParamDecl[] | undefined, values: Record<string, number>): string | null {
  for (const param of params ?? []) {
    const value = values[param.id];
    if (value === undefined || !accepts(param, value)) {
      return `${param.title} is not a value this plugin accepts.`;
    }
  }
  return null;
}

export function describeLocality(locality: Locality): string {
  switch (locality.kind) {
    case 'pointwise':
      return 'Each pixel on its own';
    case 'local':
      return `Looks up to ${locality.radius} pixel${locality.radius === 1 ? '' : 's'} around each pixel`;
    case 'global':
      return 'Looks at the whole image at once';
  }
}

export function stepOf(param: ParamDecl): number {
  switch (param.type) {
    case 'integer':
      return 1;
    case 'number':
      return param.step ?? (param.max - param.min) / 100;
    default:
      return 1;
  }
}
