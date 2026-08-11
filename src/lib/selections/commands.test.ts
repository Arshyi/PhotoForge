import { invoke } from '@tauri-apps/api/core';
import { beforeEach, describe, expect, it, vi } from 'vitest';
import {
  exportMaskFile,
  exportMaskPng,
  importMaskFile,
  importMaskPng
} from './commands';
import type { MaskFile, MaskSnapshot } from './types';

vi.mock('@tauri-apps/api/core', () => ({ invoke: vi.fn() }));

const invokeMock = vi.mocked(invoke);

const mask: MaskSnapshot = {
  version: 1,
  width: 1,
  height: 1,
  encoding: 'base64_u8',
  data: '_w',
  checksum: 'fnv1a64:af63bd4c8601b7df'
};

const document: MaskFile = {
  format: 'photoforge-mask',
  version: 1,
  id: 'mask-1',
  name: 'Mask 1',
  mask,
  metadata: { createdAt: '', modifiedAt: '' }
};

beforeEach(() => invokeMock.mockReset().mockResolvedValue(undefined));

describe('mask file commands', () => {
  it('always includes the owning document and request for JSON import/export', async () => {
    const scope = { documentId: 41, requestId: 72 };
    await importMaskFile({ path: 'C:\\masks\\subject.json', ...scope });
    await exportMaskFile({ path: 'C:\\masks\\subject.json', document, ...scope });

    expect(invokeMock.mock.calls).toEqual([
      ['import_mask_file', { path: 'C:\\masks\\subject.json', ...scope }],
      ['export_mask_file', { path: 'C:\\masks\\subject.json', document, ...scope }]
    ]);
  });

  it('always includes the owning document and request for PNG import/export', async () => {
    const scope = { documentId: 9, requestId: 12 };
    await importMaskPng({ path: 'C:\\masks\\subject.png', ...scope });
    await exportMaskPng({ path: 'C:\\masks\\subject.png', mask, ...scope });

    expect(invokeMock.mock.calls).toEqual([
      ['import_mask_png', { path: 'C:\\masks\\subject.png', ...scope }],
      ['export_mask_png', { path: 'C:\\masks\\subject.png', mask, ...scope }]
    ]);
  });
});
