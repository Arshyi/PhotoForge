import { cloneSelectionState, createSelectionState } from './state';
import { hasValidDecodedCoverageChecksum } from './checksum';
import {
  canonicalizeGeometryOperations,
  computeStageDimensions,
  geometryFingerprint
} from './geometry';
import type {
  MaskDiagnostics,
  MaskSnapshot,
  NamedMask,
  OverlaySettings,
  SelectionSettings,
  SelectionState,
  SelectionTool
} from './types';

const STORAGE_PREFIX_V2 = 'photoforge.selection-session.v2:';
const LEGACY_STORAGE_PREFIX = 'photoforge.selection-session.v1:';
const MAX_SESSION_CHARACTERS = 3_500_000;
const MAX_CANVAS_PIXELS = 100_000_000;
const MAX_NAMED_MASKS = 100;

export function documentSelectionKey(sourcePath: string, width: number, height: number): string {
  const value = `${normalizeSourcePath(sourcePath)}\u0000${width}x${height}`;
  let hash = 0xcbf29ce484222325n;
  for (let index = 0; index < value.length; index += 1) {
    const code = value.charCodeAt(index);
    hash ^= BigInt(code & 0xff);
    hash = BigInt.asUintN(64, hash * 0x100000001b3n);
    hash ^= BigInt(code >>> 8);
    hash = BigInt.asUintN(64, hash * 0x100000001b3n);
  }
  return `${width}x${height}-${hash.toString(16).padStart(16, '0')}`;
}

export function legacyDocumentSelectionKey(filename: string, width: number, height: number): string {
  let hash = 0x811c9dc5;
  const value = `${filename}\u0000${width}x${height}`;
  for (let index = 0; index < value.length; index += 1) {
    hash ^= value.charCodeAt(index);
    hash = Math.imul(hash, 0x01000193);
  }
  return `${width}x${height}-${(hash >>> 0).toString(16).padStart(8, '0')}`;
}

export function saveSelectionSession(
  state: SelectionState,
  storage: Pick<Storage, 'setItem'> = localStorage
): boolean {
  try {
    const candidate = parseV2SelectionState(state, state.documentKey);
    if (!candidate) return false;
    const json = JSON.stringify(candidate);
    if (json.length > MAX_SESSION_CHARACTERS) return false;
    storage.setItem(`${STORAGE_PREFIX_V2}${state.documentKey}`, json);
    return true;
  } catch {
    return false;
  }
}

export function loadSelectionSession(
  documentKey: string,
  width: number,
  height: number,
  storage: Pick<Storage, 'getItem'> = localStorage,
  legacyDocumentKey?: string
): SelectionState {
  const fallback = createSelectionState(documentKey, width, height);
  try {
    const keys = [documentKey];
    if (legacyDocumentKey && legacyDocumentKey !== documentKey) keys.push(legacyDocumentKey);
    for (const candidateKey of keys) {
      const current = storage.getItem(`${STORAGE_PREFIX_V2}${candidateKey}`);
      if (current !== null) {
        if (current.length === 0 || current.length > MAX_SESSION_CHARACTERS) return fallback;
        const loaded = parseV2SelectionState(JSON.parse(current), candidateKey, width, height);
        return loaded ? rekeySelectionState(loaded, documentKey) : fallback;
      }

      const legacy = storage.getItem(`${LEGACY_STORAGE_PREFIX}${candidateKey}`);
      if (legacy !== null) {
        if (legacy.length === 0 || legacy.length > MAX_SESSION_CHARACTERS) return fallback;
        const loaded = migrateV1SelectionState(JSON.parse(legacy), candidateKey, width, height);
        return loaded ? rekeySelectionState(loaded, documentKey) : fallback;
      }
    }
    return fallback;
  } catch {
    return fallback;
  }
}

export function createMaskFile(
  id: string,
  name: string,
  mask: MaskSnapshot,
  createdAt: string,
  modifiedAt: string,
  sourceTool?: string
) {
  return {
    format: 'photoforge-mask' as const,
    version: 1 as const,
    id,
    name,
    mask: structuredClone(mask),
    metadata: { createdAt, modifiedAt, ...(sourceTool ? { sourceTool } : {}) }
  };
}

export function validSnapshot(
  value: MaskSnapshot | null | undefined,
  width?: number,
  height?: number
): value is MaskSnapshot {
  if (!value) return false;
  const pixels = value.width * value.height;
  const encoding = value.encoding;
  const maximumBytes = encoding === 'base64_rle_u8' ? pixels * 2 : pixels;
  const maximumCharacters = Math.ceil(maximumBytes / 3) * 4;
  const structurallyValid = (
    value.version === 1 &&
    Number.isSafeInteger(value.width) &&
    Number.isSafeInteger(value.height) &&
    value.width > 0 &&
    value.height > 0 &&
    Number.isSafeInteger(pixels) &&
    pixels <= MAX_CANVAS_PIXELS &&
    (width === undefined || value.width === width) &&
    (height === undefined || value.height === height) &&
    (encoding === 'base64_u8' || encoding === 'base64_rle_u8') &&
    typeof value.data === 'string' &&
    value.data.length <= maximumCharacters &&
    /^[A-Za-z0-9+/]*={0,2}$/.test(value.data) &&
    /^fnv1a64:[0-9a-f]{16}$/.test(value.checksum)
  );
  return structurallyValid && hasValidDecodedCoverageChecksum(value);
}

function parseV2SelectionState(
  value: unknown,
  documentKey: string,
  sourceWidth?: number,
  sourceHeight?: number
): SelectionState | null {
  if (!isRecord(value) || value.schemaVersion !== 2 || value.documentKey !== documentKey) return null;
  if (!validCanvasDimensions(value.canvasWidth, value.canvasHeight)) return null;

  const geometryOperations = canonicalizeGeometryOperations(value.geometryOperations);
  if (!geometryOperations || value.geometryFingerprint !== geometryFingerprint(geometryOperations)) {
    return null;
  }
  if (sourceWidth !== undefined || sourceHeight !== undefined) {
    if (sourceWidth === undefined || sourceHeight === undefined ||
      !validCanvasDimensions(sourceWidth, sourceHeight)) return null;
    const expected = computeStageDimensions(sourceWidth, sourceHeight, geometryOperations);
    if (expected.width !== value.canvasWidth || expected.height !== value.canvasHeight) return null;
  }

  const activeMask = value.activeMask === null
    ? null
    : validSnapshot(value.activeMask as MaskSnapshot, value.canvasWidth, value.canvasHeight)
      ? structuredClone(value.activeMask as MaskSnapshot)
      : undefined;
  if (activeMask === undefined) return null;
  const namedMasks = parseNamedMasks(value.namedMasks, value.canvasWidth, value.canvasHeight, true);
  if (!namedMasks) return null;
  const activeDiagnostics = parseDiagnostics(
    value.activeDiagnostics,
    value.canvasWidth,
    value.canvasHeight,
    activeMask !== null
  );
  if (activeDiagnostics === undefined) return null;
  const overlay = parseOverlay(value.overlay);
  const settings = parseSettings(value.settings);
  if (!overlay || !settings || !isSelectionTool(value.tool) || !isCompositionMode(value.mode) ||
    !isApplyScope(value.applyScope) || typeof value.panelCollapsed !== 'boolean' ||
    !validTimestamp(value.updatedAt)) return null;

  return cloneSelectionState({
    schemaVersion: 2,
    documentKey,
    canvasWidth: value.canvasWidth,
    canvasHeight: value.canvasHeight,
    geometryOperations,
    geometryFingerprint: value.geometryFingerprint,
    activeMask,
    activeDiagnostics,
    namedMasks,
    tool: value.tool,
    mode: value.mode,
    applyScope: value.applyScope,
    overlay,
    settings,
    panelCollapsed: value.panelCollapsed,
    updatedAt: value.updatedAt
  });
}

function migrateV1SelectionState(
  value: unknown,
  documentKey: string,
  width: number,
  height: number
): SelectionState | null {
  if (!isRecord(value) || value.schemaVersion !== 1 || value.documentKey !== documentKey) return null;
  if (!validCanvasDimensions(width, height)) return null;
  const migrated = createSelectionState(documentKey, width, height);

  if (isSelectionTool(value.tool)) migrated.tool = value.tool;
  if (isCompositionMode(value.mode)) migrated.mode = value.mode;
  if (isApplyScope(value.applyScope)) migrated.applyScope = value.applyScope;
  migrated.overlay = mergeLegacyOverlay(migrated.overlay, value.overlay);
  migrated.settings = mergeLegacySettings(migrated.settings, value.settings);
  if (typeof value.panelCollapsed === 'boolean') migrated.panelCollapsed = value.panelCollapsed;
  if (validTimestamp(value.updatedAt)) migrated.updatedAt = value.updatedAt;

  if (validSnapshot(value.activeMask as MaskSnapshot, width, height)) {
    migrated.activeMask = structuredClone(value.activeMask as MaskSnapshot);
  }
  migrated.namedMasks = parseNamedMasks(value.namedMasks, width, height, false) ?? [];
  migrated.activeDiagnostics = null;
  return migrated;
}

function parseNamedMasks(
  value: unknown,
  width: number,
  height: number,
  strict: boolean
): NamedMask[] | null {
  if (!Array.isArray(value)) return strict ? null : [];
  if (value.length > MAX_NAMED_MASKS && strict) return null;
  const namedMasks: NamedMask[] = [];
  const ids = new Set<string>();
  for (const candidate of value.slice(0, MAX_NAMED_MASKS)) {
    const namedMask = parseNamedMask(candidate, width, height);
    if (!namedMask || ids.has(namedMask.id)) {
      if (strict) return null;
      continue;
    }
    ids.add(namedMask.id);
    namedMasks.push(namedMask);
  }
  return namedMasks;
}

function parseNamedMask(value: unknown, width: number, height: number): NamedMask | null {
  if (!isRecord(value) || !boundedString(value.id, 1, 200) || !boundedString(value.name, 0, 120) ||
    typeof value.visible !== 'boolean' || typeof value.locked !== 'boolean' ||
    !validTimestamp(value.createdAt) || !validTimestamp(value.modifiedAt) ||
    !validSnapshot(value.mask as MaskSnapshot, width, height) ||
    (value.sourceTool !== undefined && !isSelectionTool(value.sourceTool))) return null;
  return {
    id: value.id,
    name: value.name,
    mask: structuredClone(value.mask as MaskSnapshot),
    visible: value.visible,
    locked: value.locked,
    createdAt: value.createdAt,
    modifiedAt: value.modifiedAt,
    ...(value.sourceTool === undefined ? {} : { sourceTool: value.sourceTool })
  };
}

function parseDiagnostics(
  value: unknown,
  width: number,
  height: number,
  hasActiveMask: boolean
): MaskDiagnostics | null | undefined {
  if (value === null) return null;
  if (!hasActiveMask || !isRecord(value) || value.width !== width || value.height !== height) {
    return undefined;
  }
  const pixels = width * height;
  if (!safeIntegerRange(value.selectedPixels, 0, pixels) ||
    !safeIntegerRange(value.fullySelectedPixels, 0, pixels) ||
    !finiteRange(value.averageCoverage, 0, 1) ||
    !safeIntegerRange(value.memoryBytes, 0, Number.MAX_SAFE_INTEGER)) return undefined;
  let bounds: [number, number, number, number] | null = null;
  if (value.bounds !== null) {
    if (!Array.isArray(value.bounds) || value.bounds.length !== 4 ||
      !value.bounds.every((coordinate) => safeIntegerRange(coordinate, 0, Math.max(width, height)))) {
      return undefined;
    }
    bounds = [...value.bounds] as [number, number, number, number];
  }
  return {
    width,
    height,
    selectedPixels: value.selectedPixels,
    fullySelectedPixels: value.fullySelectedPixels,
    averageCoverage: value.averageCoverage,
    bounds,
    memoryBytes: value.memoryBytes
  };
}

function parseOverlay(value: unknown): OverlaySettings | null {
  if (!isRecord(value) || typeof value.visible !== 'boolean' ||
    !isOverlayMode(value.mode) ||
    !finiteRange(value.opacity, 0, 1) || typeof value.color !== 'string' ||
    !/^#[0-9a-f]{6}$/i.test(value.color)) return null;
  return {
    visible: value.visible,
    mode: value.mode,
    opacity: value.opacity,
    color: value.color
  };
}

function parseSettings(value: unknown): SelectionSettings | null {
  if (!isRecord(value) || !safeIntegerRange(value.brushDiameter, 1, 512) ||
    !finiteRange(value.brushHardness, 0, 1) || !finiteRange(value.brushOpacity, 0, 1) ||
    typeof value.pressureEnabled !== 'boolean' || typeof value.pressureAffectsSize !== 'boolean' ||
    typeof value.pressureAffectsOpacity !== 'boolean' ||
    !finiteRange(value.pressureMinSizeFactor, 0, 1) ||
    !finiteRange(value.pressureMinOpacityFactor, 0, 1) ||
    !finiteRange(value.wandTolerance, 0, 1) ||
    (value.wandConnectivity !== 'four' && value.wandConnectivity !== 'eight') ||
    typeof value.wandAntiAlias !== 'boolean' || typeof value.wandContiguous !== 'boolean' ||
    typeof value.sampleMerged !== 'boolean' || !finiteRange(value.colorTolerance, 0, 1) ||
    !finiteRange(value.luminanceSensitivity, 0, 1) || !finiteRange(value.hueSensitivity, 0, 1) ||
    !finiteRange(value.saturationSensitivity, 0, 1) || typeof value.fixedAspect !== 'boolean' ||
    typeof value.fromCenter !== 'boolean') return null;
  return {
    brushDiameter: value.brushDiameter,
    brushHardness: value.brushHardness,
    brushOpacity: value.brushOpacity,
    pressureEnabled: value.pressureEnabled,
    pressureAffectsSize: value.pressureAffectsSize,
    pressureAffectsOpacity: value.pressureAffectsOpacity,
    pressureMinSizeFactor: value.pressureMinSizeFactor,
    pressureMinOpacityFactor: value.pressureMinOpacityFactor,
    wandTolerance: value.wandTolerance,
    wandConnectivity: value.wandConnectivity,
    wandAntiAlias: value.wandAntiAlias,
    wandContiguous: value.wandContiguous,
    sampleMerged: value.sampleMerged,
    colorTolerance: value.colorTolerance,
    luminanceSensitivity: value.luminanceSensitivity,
    hueSensitivity: value.hueSensitivity,
    saturationSensitivity: value.saturationSensitivity,
    fixedAspect: value.fixedAspect,
    fromCenter: value.fromCenter
  };
}

function mergeLegacyOverlay(defaults: OverlaySettings, value: unknown): OverlaySettings {
  if (!isRecord(value)) return defaults;
  const candidate = {
    visible: typeof value.visible === 'boolean' ? value.visible : defaults.visible,
    mode: isOverlayMode(value.mode)
      ? value.mode
      : defaults.mode,
    opacity: finiteRange(value.opacity, 0, 1) ? value.opacity : defaults.opacity,
    color: typeof value.color === 'string' && /^#[0-9a-f]{6}$/i.test(value.color)
      ? value.color
      : defaults.color
  };
  return candidate as OverlaySettings;
}

function mergeLegacySettings(defaults: SelectionSettings, value: unknown): SelectionSettings {
  if (!isRecord(value)) return defaults;
  const candidate = { ...defaults } as Record<keyof SelectionSettings, unknown>;
  for (const key of Object.keys(defaults) as Array<keyof SelectionSettings>) {
    if (key in value) candidate[key] = value[key];
  }
  return parseSettings(candidate) ?? defaults;
}

function validCanvasDimensions(width: unknown, height: unknown): boolean {
  if (!Number.isSafeInteger(width) || !Number.isSafeInteger(height) ||
    Number(width) < 1 || Number(height) < 1) return false;
  const pixels = Number(width) * Number(height);
  return Number.isSafeInteger(pixels) && pixels <= MAX_CANVAS_PIXELS;
}

function isSelectionTool(value: unknown): value is SelectionTool {
  return value === 'none' || value === 'rectangle' || value === 'ellipse' ||
    value === 'freehand' || value === 'polygon' || value === 'brush' || value === 'eraser' ||
    value === 'magic_wand' || value === 'color_range';
}

function isOverlayMode(value: unknown): value is OverlaySettings['mode'] {
  return value === 'marching_ants' || value === 'color' || value === 'grayscale' ||
    value === 'black' || value === 'white' || value === 'mask_only';
}

function isCompositionMode(value: unknown): value is SelectionState['mode'] {
  return value === 'replace' || value === 'add' || value === 'subtract' || value === 'intersect';
}

function isApplyScope(value: unknown): value is SelectionState['applyScope'] {
  return value === 'global' || value === 'inside' || value === 'outside';
}

function validTimestamp(value: unknown): value is string {
  return boundedString(value, 1, 64) && Number.isFinite(Date.parse(value));
}

function boundedString(value: unknown, minimum: number, maximum: number): value is string {
  return typeof value === 'string' && value.length >= minimum && value.length <= maximum;
}

function finiteRange(value: unknown, minimum: number, maximum: number): value is number {
  return typeof value === 'number' && Number.isFinite(value) && value >= minimum && value <= maximum;
}

function safeIntegerRange(value: unknown, minimum: number, maximum: number): value is number {
  return Number.isSafeInteger(value) && Number(value) >= minimum && Number(value) <= maximum;
}

function isRecord(value: unknown): value is Record<string, any> {
  return value !== null && typeof value === 'object' && !Array.isArray(value);
}

function rekeySelectionState(state: SelectionState, documentKey: string): SelectionState {
  return state.documentKey === documentKey ? state : cloneSelectionState({ ...state, documentKey });
}

function normalizeSourcePath(value: string): string {
  const slashNormalized = value.trim().replace(/\\/g, '/').replace(/\/{2,}/g, '/').toLowerCase();
  const absolute = slashNormalized.startsWith('/') || /^[a-z]:\//.test(slashNormalized);
  const parts: string[] = [];
  for (const part of slashNormalized.split('/')) {
    if (!part || part === '.') continue;
    if (part === '..' && parts.length && parts.at(-1) !== '..') parts.pop();
    else if (part !== '..' || !absolute) parts.push(part);
  }
  const prefix = slashNormalized.startsWith('/') ? '/' : '';
  return `${prefix}${parts.join('/')}`;
}
