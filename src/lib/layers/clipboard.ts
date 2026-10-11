import { invoke } from '@tauri-apps/api/core';
import type { EditOperation } from '../types/editor';
import type { MaskSnapshot } from '../selections/types';
import type { LayerDocument, LayerPixelsResult, ProjectLoadResult } from './types';

/** What a copy put on the clipboard. */
export interface ClipboardCopyResult {
  width: number;
  height: number;
  /** "layer" or "merged" (the visible picture with the document's edits). */
  scope: 'layer' | 'merged';
  /** A selection limited what was copied. */
  selection: boolean;
  /** The forms written, by name: PNG (lossless, with alpha) and CF_DIBV5. */
  formats: string[];
  pngBytes: number;
  /** Highlights beyond white in a high-precision document are clipped for the clipboard. */
  clippedHighlights: boolean;
  processingTimeMs: number;
}

/** A picture read from the clipboard into a session buffer, for a new layer. */
export interface ClipboardPasteResult extends LayerPixelsResult {
  /** The form it was read in: PNG, CF_DIBV5 or CF_DIB. */
  format: 'png' | 'dib_v5' | 'dib';
}

/**
 * Copies a layer (`layerId`) or, with `layerId` null, the visible picture with the document's
 * edits to the system clipboard. `selection` limits it, in the space of what is copied. The
 * document is not changed.
 */
export function copyToClipboard(
  document: LayerDocument,
  operations: EditOperation[],
  layerId: string | null,
  selection: MaskSnapshot | null
): Promise<ClipboardCopyResult> {
  return invoke<ClipboardCopyResult>('copy_to_clipboard', {
    document,
    operations,
    layerId,
    selection
  });
}

/** Reads the picture on the clipboard into a new pixel buffer in the session. */
export function pasteClipboardImage(): Promise<ClipboardPasteResult> {
  return invoke<ClipboardPasteResult>('paste_clipboard_image');
}

/** Opens the picture on the clipboard as a new, unsaved document. */
export function openClipboardImage(requestId: number): Promise<ProjectLoadResult> {
  return invoke<ProjectLoadResult>('open_clipboard_image', { requestId });
}

/** Whether the clipboard holds a picture. */
export function clipboardHasImage(): Promise<boolean> {
  return invoke<boolean>('clipboard_has_image');
}

/** One sentence for the notification after a copy. */
export function copySentence(result: ClipboardCopyResult): string {
  const what =
    result.scope === 'layer'
      ? result.selection
        ? 'the selected part of the layer'
        : 'the layer'
      : result.selection
        ? 'the selected part of the picture'
        : 'the picture';
  const size = `${result.width} × ${result.height}`;
  const clipped = result.clippedHighlights
    ? ' Highlights brighter than white were clipped: the clipboard holds 8-bit colour.'
    : '';
  return `Copied ${what} (${size}) to the clipboard.${clipped}`;
}

/** One sentence for the notification after a paste. */
export function pasteSentence(result: ClipboardPasteResult): string {
  return `Pasted a ${result.width} × ${result.height} picture from the clipboard as a new layer.`;
}
