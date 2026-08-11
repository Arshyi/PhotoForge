import { describe, expect, it } from 'vitest';
import { decodedCoverageChecksum } from './checksum';
import { geometryFingerprint } from './geometry';
import { createNamedMask, createSelectionState, setActiveMask } from './state';
import {
  createMaskFile,
  documentSelectionKey,
  legacyDocumentSelectionKey,
  loadSelectionSession,
  saveSelectionSession,
  validSnapshot
} from './serialization';
import type { GeometryOperation, MaskSnapshot } from './types';

function snapshot(width = 2, height = 2): MaskSnapshot {
  const value: MaskSnapshot = {
    version: 1,
    width,
    height,
    encoding: 'base64_u8',
    data: btoa(String.fromCharCode(...new Uint8Array(width * height))),
    checksum: ''
  };
  value.checksum = decodedCoverageChecksum(value) as string;
  return value;
}

class MemoryStorage {
  values = new Map<string, string>();
  setCalls = 0;
  getItem(key: string) { return this.values.get(key) ?? null; }
  setItem(key: string, value: string) {
    this.setCalls += 1;
    this.values.set(key, value);
  }
}

const legacySettings = {
  brushDiameter: 72,
  brushHardness: 0.6,
  brushOpacity: 0.8,
  wandTolerance: 0.2,
  wandConnectivity: 'four',
  wandAntiAlias: false,
  wandContiguous: false,
  sampleMerged: false,
  colorTolerance: 0.3,
  luminanceSensitivity: 0.4,
  hueSensitivity: 0.5,
  saturationSensitivity: 0.6,
  fixedAspect: true,
  fromCenter: true
};

function literalV1(documentKey = 'legacy') {
  return {
    schemaVersion: 1,
    documentKey,
    activeMask: snapshot(),
    activeDiagnostics: null,
    namedMasks: [{
      id: 'mask-1',
      name: 'Subject',
      mask: snapshot(),
      visible: true,
      locked: false,
      createdAt: '2026-01-01T00:00:00.000Z',
      modifiedAt: '2026-01-02T00:00:00.000Z',
      sourceTool: 'brush'
    }],
    tool: 'brush',
    mode: 'add',
    applyScope: 'inside',
    overlay: { visible: false, mode: 'grayscale', opacity: 0.5, color: '#123456' },
    settings: legacySettings,
    panelCollapsed: true,
    updatedAt: '2026-01-03T00:00:00.000Z'
  };
}

describe('selection session serialization', () => {
  it('uses a normalized path fingerprint without storing plaintext path data', () => {
    expect(documentSelectionKey('C:\\Photos\\photo.png', 100, 50)).toBe(
      documentSelectionKey('c:/photos/./photo.png', 100, 50)
    );
    expect(documentSelectionKey('C:/one/photo.png', 100, 50)).not.toBe(
      documentSelectionKey('C:/two/photo.png', 100, 50)
    );
    expect(documentSelectionKey('C:/one/photo.png', 100, 50)).not.toContain('photo.png');
  });

  it('reads the old filename key only as a fallback and rekeys state without writing', () => {
    const storage = new MemoryStorage();
    const oldKey = legacyDocumentSelectionKey('photo.png', 2, 2);
    const newKey = documentSelectionKey('C:/different/folder/photo.png', 2, 2);
    storage.values.set(`photoforge.selection-session.v1:${oldKey}`, JSON.stringify(literalV1(oldKey)));
    const loaded = loadSelectionSession(newKey, 2, 2, storage, oldKey);
    expect(loaded.documentKey).toBe(newKey);
    expect(loaded.activeMask).toEqual(snapshot());
    expect(storage.setCalls).toBe(0);
    expect(storage.values.has(`photoforge.selection-session.v2:${newKey}`)).toBe(false);
  });

  it('round trips v2 transformed rectangular canvases and named masks', () => {
    const storage = new MemoryStorage();
    const geometryOperations: GeometryOperation[] = [
      {
        type: 'crop', x: 0, y: 0, width: 0.75, height: 1,
        aspect_ratio: null, overlay: 'none'
      },
      { type: 'reflect_horizontal' },
      { type: 'rotate', degrees: 90 },
      {
        type: 'lens_correction', distortion: 0.1, vignetting: -0.2,
        chromatic_aberration: 0.05
      }
    ];
    let state = createSelectionState('doc', 8, 4);
    state = {
      ...state,
      canvasWidth: 4,
      canvasHeight: 6,
      geometryOperations,
      geometryFingerprint: geometryFingerprint(geometryOperations),
      settings: { ...state.settings, pressureEnabled: true, pressureAffectsOpacity: true }
    };
    state = setActiveMask(state, snapshot(4, 6), null);
    state = createNamedMask(state, 'Subject', new Date('2026-01-01T00:00:00.000Z'), 'mask-1');

    expect(saveSelectionSession(state, storage)).toBe(true);
    expect(storage.values.has('photoforge.selection-session.v2:doc')).toBe(true);
    expect(loadSelectionSession('doc', 8, 4, storage)).toEqual(state);
    expect(loadSelectionSession('doc', 9, 4, storage)).toMatchObject({
      canvasWidth: 9,
      canvasHeight: 4,
      geometryOperations: [],
      activeMask: null,
      namedMasks: []
    });
  });

  it('migrates a literal v1 session to identity geometry and current pressure defaults without writes', () => {
    const storage = new MemoryStorage();
    storage.values.set('photoforge.selection-session.v1:legacy', JSON.stringify(literalV1()));

    const migrated = loadSelectionSession('legacy', 2, 2, storage);
    expect(migrated).toMatchObject({
      schemaVersion: 2,
      documentKey: 'legacy',
      canvasWidth: 2,
      canvasHeight: 2,
      geometryOperations: [],
      geometryFingerprint: geometryFingerprint([]),
      activeMask: snapshot(),
      tool: 'brush',
      mode: 'add',
      applyScope: 'inside',
      panelCollapsed: true
    });
    expect(migrated.namedMasks).toHaveLength(1);
    expect(migrated.settings).toMatchObject({
      brushDiameter: 72,
      pressureEnabled: false,
      pressureAffectsSize: true,
      pressureAffectsOpacity: false,
      pressureMinSizeFactor: 0.35,
      pressureMinOpacityFactor: 0.25
    });
    expect(storage.setCalls).toBe(0);
  });

  it('rejects malformed, future, mismatched-fingerprint, and mismatched-mask v2 sessions', () => {
    const valid = createSelectionState('doc', 2, 2);
    const cases: unknown[] = [
      { ...valid, schemaVersion: 3 },
      { ...valid, geometryFingerprint: 'geometry-v1:0000000000000000' },
      { ...valid, canvasWidth: Number.NaN },
      { ...valid, activeMask: snapshot(3, 2) }
    ];
    for (const candidate of cases) {
      const storage = new MemoryStorage();
      storage.values.set('photoforge.selection-session.v2:doc', JSON.stringify(candidate));
      storage.values.set('photoforge.selection-session.v1:doc', JSON.stringify(literalV1('doc')));
      expect(loadSelectionSession('doc', 2, 2, storage)).toEqual(createSelectionState('doc', 2, 2));
      expect(storage.setCalls).toBe(0);
    }

    const malformed = new MemoryStorage();
    malformed.values.set('photoforge.selection-session.v2:doc', '{bad');
    expect(loadSelectionSession('doc', 2, 2, malformed)).toEqual(createSelectionState('doc', 2, 2));
  });

  it('refuses to save invalid v2 state and validates bounded snapshots', () => {
    const storage = new MemoryStorage();
    const state = createSelectionState('doc', 2, 2);
    expect(saveSelectionSession({ ...state, geometryFingerprint: 'bad' }, storage)).toBe(false);
    expect(storage.setCalls).toBe(0);
    expect(validSnapshot({ ...snapshot(), version: 99 })).toBe(false);
    expect(validSnapshot({ ...snapshot(), data: 'x'.repeat(100) })).toBe(false);
    expect(validSnapshot({ ...snapshot(), width: 0 })).toBe(false);
    expect(validSnapshot({ ...snapshot(), checksum: 'fnv1a64:0123456789abcdef' })).toBe(false);
  });

  it('creates the documented local mask envelope', () => {
    expect(createMaskFile('id', 'Subject', snapshot(), 'created', 'modified', 'rectangle')).toMatchObject({
      format: 'photoforge-mask',
      version: 1,
      id: 'id',
      name: 'Subject',
      metadata: { sourceTool: 'rectangle' }
    });
  });
});
