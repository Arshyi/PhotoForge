import { invoke } from '@tauri-apps/api/core';
import type { OriginState, SourceAdmission, SourceOrigin, SourcePreview } from './types';

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

/** Whether the file a layer's origin names is still the one it describes. */
export function inspectSourceOrigin(origin: SourceOrigin): Promise<OriginState> {
  return invoke<OriginState>('inspect_source_origin', { origin });
}
