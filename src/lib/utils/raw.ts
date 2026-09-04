import { invoke } from '@tauri-apps/api/core';
import type { RawInspectionResult } from '../types/editor';

/** Inspect a camera RAW path without decoding or mutating the source. */
export function inspectRaw(path: string): Promise<RawInspectionResult> {
  return invoke<RawInspectionResult>('inspect_raw', { path });
}
