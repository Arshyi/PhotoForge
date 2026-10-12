import { invoke } from '@tauri-apps/api/core';
import type { OriginState, Rect, SourceAdmission, SourceOrigin, SourcePreview } from './types';

/** Reads a file's header and reports what can be done with it. */
export function probeImageSource(path: string): Promise<SourceAdmission> {
  return invoke<SourceAdmission>('probe_image_source', { path });
}

/** A bounded picture of the whole source, for choosing a region on. */
export function sourcePreviewImage(path: string, maxEdge: number): Promise<SourcePreview> {
  return invoke<SourcePreview>('source_preview_image', { path, maxEdge });
}

/** Stops the preview in flight. It holds the CPU job gate while it decodes. */
export function cancelSourcePreview(): Promise<void> {
  return invoke<void>('cancel_source_preview');
}

/**
 * Where a region of a camera RAW's sensor (as a layer records it) lies on the picture a region is
 * chosen on, or null when any part of it is outside that picture (it took in the masked border).
 */
export function rawRegionOnPicture(path: string, view: Rect): Promise<Rect | null> {
  return invoke<Rect | null>('raw_region_on_picture', { path, view });
}

/** Whether the file a layer's origin names is still the one it describes. */
export function inspectSourceOrigin(origin: SourceOrigin): Promise<OriginState> {
  return invoke<OriginState>('inspect_source_origin', { origin });
}
