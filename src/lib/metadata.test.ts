import { afterEach, describe, expect, it, vi } from 'vitest';
import {
  DEFAULT_METADATA_POLICY,
  describeSource,
  loadMetadataPolicy,
  METADATA_POLICIES,
  metadataSentence,
  policyHint,
  saveMetadataPolicy
} from './metadata';
import type { SourceMetadataSummary } from './types/editor';

const summary: SourceMetadataSummary = {
  hasExif: true,
  hasXmp: true,
  hasIptc: false,
  make: 'Acme',
  model: 'Cam-One',
  lens: null,
  taken: '2026:10:10 14:03:09',
  hasLocation: true,
  hasMakerNote: true,
  hasIdentifiers: true,
  hasThumbnail: true,
  orientation: 6,
  orientationApplied: true
};

afterEach(() => {
  vi.restoreAllMocks();
  localStorage.clear();
});

describe('the metadata policy', () => {
  it('defaults to removing what identifies a place or a device', () => {
    expect(DEFAULT_METADATA_POLICY).toBe('remove_private');
    expect(loadMetadataPolicy()).toBe('remove_private');
  });

  it('offers the three policies, each with a reason', () => {
    expect(METADATA_POLICIES.map((entry) => entry.value)).toEqual(['remove_private', 'keep_all', 'remove_all']);
    for (const entry of METADATA_POLICIES) expect(policyHint(entry.value).length).toBeGreaterThan(20);
  });

  it('remembers a choice and ignores a stored value it does not know', () => {
    saveMetadataPolicy('keep_all');
    expect(loadMetadataPolicy()).toBe('keep_all');
    localStorage.setItem('photoforge.metadataPolicy', 'something_else');
    expect(loadMetadataPolicy()).toBe('remove_private');
  });

  it('works when storage is blocked', () => {
    vi.spyOn(Storage.prototype, 'getItem').mockImplementation(() => {
      throw new Error('blocked');
    });
    vi.spyOn(Storage.prototype, 'setItem').mockImplementation(() => {
      throw new Error('blocked');
    });
    expect(loadMetadataPolicy()).toBe('remove_private');
    expect(() => saveMetadataPolicy('remove_all')).not.toThrow();
  });
});

describe('describing a source', () => {
  it('says what the file carried without showing where it was taken', () => {
    const lines = describeSource(summary);
    expect(lines[0]).toBe('EXIF · Acme Cam-One · 2026:10:10 14:03:09');
    expect(lines).toContain('Includes the place it was taken');
    expect(lines).toContain('Turned upright when opened');
    expect(lines.join(' ')).not.toMatch(/\d+\.\d+/);
  });

  it('has nothing to say about a file with no metadata', () => {
    expect(describeSource(null)).toEqual([]);
    expect(describeSource(undefined)).toEqual([]);
  });
});

describe('the sentence after an export', () => {
  it('is empty when there was nothing to consider', () => {
    expect(metadataSentence(null)).toBe('');
    expect(metadataSentence({ policy: 'keep_all', notes: [] })).toBe('');
  });

  it('says that metadata was kept when nothing was removed', () => {
    expect(metadataSentence({ policy: 'keep_all', notes: [{ kind: 'kept', detail: 'EXIF' }] })).toBe('Metadata kept.');
  });

  it('names what was removed and counts the rest', () => {
    const report = {
      policy: 'remove_private' as const,
      notes: [
        { kind: 'removed' as const, detail: 'the place' },
        { kind: 'removed' as const, detail: 'serial numbers' },
        { kind: 'removed' as const, detail: 'the preview' }
      ]
    };
    expect(metadataSentence(report)).toBe('Metadata removed: the place; serial numbers and 1 more.');
  });
});
