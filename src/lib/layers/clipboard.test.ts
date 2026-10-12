import { beforeEach, describe, expect, it, vi } from 'vitest';
import { invoke } from '@tauri-apps/api/core';
import {
  clipboardHasImage,
  copySentence,
  copyToClipboard,
  openClipboardImage,
  pasteClipboardImage,
  pasteSentence,
  type ClipboardCopyResult,
  type ClipboardPasteResult
} from './clipboard';
import { createDocument } from './tree';

vi.mock('@tauri-apps/api/core', () => ({ invoke: vi.fn() }));

const copied = (overrides: Partial<ClipboardCopyResult> = {}): ClipboardCopyResult => ({
  width: 40,
  height: 30,
  scope: 'merged',
  selection: false,
  formats: ['PNG', 'CF_DIBV5'],
  pngBytes: 1200,
  clippedHighlights: false,
  processingTimeMs: 4,
  ...overrides
});

describe('clipboard helpers', () => {
  beforeEach(() => vi.mocked(invoke).mockReset().mockResolvedValue({}));

  it('calls the commands with the arguments the backend takes', async () => {
    const document = createDocument(4, 4, [], 'legacy_srgb8');
    await copyToClipboard(document, [], 'l1', null);
    expect(invoke).toHaveBeenLastCalledWith('copy_to_clipboard', {
      document,
      operations: [],
      layerId: 'l1',
      selection: null
    });
    await pasteClipboardImage();
    expect(invoke).toHaveBeenLastCalledWith('paste_clipboard_image');
    await openClipboardImage(9);
    expect(invoke).toHaveBeenLastCalledWith('open_clipboard_image', { requestId: 9 });
    await clipboardHasImage();
    expect(invoke).toHaveBeenLastCalledWith('clipboard_has_image');
  });

  it('says what was copied, and of what', () => {
    expect(copySentence(copied())).toBe('Copied the picture (40 × 30) to the clipboard.');
    expect(copySentence(copied({ scope: 'layer' }))).toContain('Copied the layer (40 × 30)');
    expect(copySentence(copied({ scope: 'layer', selection: true }))).toContain(
      'the selected part of the layer'
    );
    expect(copySentence(copied({ selection: true }))).toContain('the selected part of the picture');
  });

  it('says when highlights were clipped for the clipboard', () => {
    expect(copySentence(copied({ clippedHighlights: true }))).toMatch(/clipped.*8-bit/);
  });

  it('says what was pasted', () => {
    const pasted = { width: 12, height: 8, pixelId: 'px1', filename: 'Pasted image', format: 'png' } as ClipboardPasteResult;
    expect(pasteSentence(pasted)).toBe('Pasted a 12 × 8 picture from the clipboard as a new layer.');
  });
});
