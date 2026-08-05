import { describe, expect, it } from 'vitest';
import { createSelectionState, setActiveMask } from './state';
import {
  createMaskFile,
  documentSelectionKey,
  loadSelectionSession,
  saveSelectionSession,
  validSnapshot
} from './serialization';
import type { MaskSnapshot } from './types';

const snapshot: MaskSnapshot = {
  version: 1,
  width: 2,
  height: 2,
  encoding: 'base64_u8',
  data: 'AP+A/w',
  checksum: 'fnv1a64:0123456789abcdef'
};

class MemoryStorage {
  values = new Map<string, string>();
  getItem(key: string) { return this.values.get(key) ?? null; }
  setItem(key: string, value: string) { this.values.set(key, value); }
}

describe('selection session serialization', () => {
  it('uses a stable document fingerprint', () => {
    expect(documentSelectionKey('photo.png', 100, 50)).toBe(documentSelectionKey('photo.png', 100, 50));
    expect(documentSelectionKey('other.png', 100, 50)).not.toBe(documentSelectionKey('photo.png', 100, 50));
  });

  it('round trips bounded state and rejects changed dimensions', () => {
    const storage = new MemoryStorage();
    const state = setActiveMask(createSelectionState('doc'), snapshot, null);
    expect(saveSelectionSession(state, storage)).toBe(true);
    expect(loadSelectionSession('doc', 2, 2, storage).activeMask).toEqual(snapshot);
    expect(loadSelectionSession('doc', 3, 2, storage).activeMask).toBeNull();
  });

  it('fails closed for malformed or future snapshots', () => {
    expect(validSnapshot({ ...snapshot, version: 99 })).toBe(false);
    expect(validSnapshot({ ...snapshot, data: 'x'.repeat(100) })).toBe(false);
    const storage = new MemoryStorage();
    storage.setItem('photoforge.selection-session.v1:doc', '{bad');
    expect(loadSelectionSession('doc', 2, 2, storage).activeMask).toBeNull();
  });

  it('creates the documented local mask envelope', () => {
    expect(createMaskFile('id', 'Subject', snapshot, 'created', 'modified', 'rectangle')).toMatchObject({
      format: 'photoforge-mask',
      version: 1,
      id: 'id',
      name: 'Subject',
      metadata: { sourceTool: 'rectangle' }
    });
  });
});
