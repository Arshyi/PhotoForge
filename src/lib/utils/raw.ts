import { invoke } from '@tauri-apps/api/core';
import type {
  ExportResult,
  RawCaptureMetadata,
  RawDevelopmentParameters,
  RawDevelopResult,
  RawInspectionResult,
  RawLayerSource,
  RawSourceReference,
  RawSourceStatus,
  RawSourceStatusResult
} from '../types/editor';

/** Inspect a camera RAW path without decoding or mutating the source. */
export function inspectRaw(path: string): Promise<RawInspectionResult> {
  return invoke<RawInspectionResult>('inspect_raw', { path });
}

/**
 * Decodes a RAW file and develops it into a layer buffer.
 *
 * `fullResolution` is false for interactive work: a large sensor is decimated
 * and demosaiced with the fast algorithm so a slider stays usable. Export and
 * final render pass true.
 */
export function openRawLayer(
  path: string,
  parameters: RawDevelopmentParameters | null,
  fullResolution: boolean
): Promise<RawDevelopResult> {
  return invoke<RawDevelopResult>('open_raw_layer', { path, parameters, fullResolution });
}

/** Re-develops a RAW layer from its original file with new parameters. */
export function developRawLayer(
  source: RawLayerSource,
  parameters: RawDevelopmentParameters | null,
  fullResolution: boolean,
  documentId: number,
  requestId: number
): Promise<RawDevelopResult> {
  return invoke<RawDevelopResult>('develop_raw_layer', {
    request: { source, parameters, fullResolution, documentId, requestId }
  });
}

/** Reports whether a linked RAW source is present and unchanged. */
export function verifyRawSource(
  reference: RawSourceReference,
  path: string
): Promise<RawSourceStatusResult> {
  return invoke<RawSourceStatusResult>('verify_raw_source', { reference, path });
}

/**
 * Points a RAW layer at a file the user chose after the original went missing.
 * The backend verifies the hash and refuses a different photograph.
 */
export function relinkRawSource(
  source: RawLayerSource,
  path: string
): Promise<RawLayerSource> {
  return invoke<RawLayerSource>('relink_raw_source', { source, path });
}

/** Develops at full sensor resolution and writes a true 16-bit PNG. */
export function exportRawLayerPng16(
  outputPath: string,
  source: RawLayerSource,
  parameters: RawDevelopmentParameters | null
): Promise<ExportResult> {
  return invoke<ExportResult>('export_raw_layer_png16', { outputPath, source, parameters });
}

/**
 * Whether a path names a camera RAW file this build can decode.
 *
 * Only DNG: other camera extensions are recognised by the backend so it can
 * explain itself, but routing one here would promise a decode that does not
 * exist.
 */
export function isRawPath(path: string): boolean {
  return /\.dng$/i.test(path.trim());
}

/** The default development a freshly opened RAW starts from. */
export function defaultDevelopment(): RawDevelopmentParameters {
  return {
    whiteBalance: { mode: 'asShot', multipliers: [1, 1, 1] },
    exposureEv: 0,
    contrast: 0,
    highlights: 0,
    shadows: 0,
    whites: 0,
    blacks: 0
  };
}

/** Whether a source is usable right now. */
export function isSourceAvailable(status: RawSourceStatus): boolean {
  return status === 'available';
}

/**
 * A short sentence explaining a source state, for the interface to show
 * instead of a raw status value.
 */
export function describeSourceStatus(status: RawSourceStatus, filename: string): string {
  if (status === 'available') return `${filename} is linked and unchanged.`;
  if (status === 'missing') {
    return `${filename} is not where this project last saw it. Locate it to keep editing.`;
  }
  return `The file found in place of ${filename} is a different photograph. PhotoForge will not use it without you confirming.`;
}

/** One metadata row for the RAW information panel. */
export interface RawMetadataRow {
  label: string;
  value: string;
}

function shutterText(seconds: number): string {
  if (seconds <= 0) return '';
  // Photographers read fractions, not decimals, below a second.
  if (seconds >= 1) return `${Number(seconds.toFixed(1))} s`;
  return `1/${Math.round(1 / seconds)} s`;
}

/**
 * Builds the rows the metadata panel shows.
 *
 * A field the file did not carry is left out entirely rather than displayed as
 * unknown or filled in with a plausible default: an invented ISO is worse than
 * a missing one.
 */
export function metadataRows(
  metadata: RawCaptureMetadata,
  extra: {
    sensorWidth?: number;
    sensorHeight?: number;
    bitsPerSample?: number;
    cfaPattern?: string;
    sha256?: string;
    multipliers?: [number, number, number];
    colorManaged?: boolean;
  } = {}
): RawMetadataRow[] {
  const rows: RawMetadataRow[] = [];
  const push = (label: string, value: string | null | undefined) => {
    if (value !== null && value !== undefined && value !== '') rows.push({ label, value });
  };

  const camera = [metadata.manufacturer, metadata.model]
    .filter((part): part is string => Boolean(part))
    .join(' ')
    .trim();
  push('Camera', camera);
  push('Lens', metadata.lens);
  push('ISO', metadata.iso ? String(metadata.iso) : null);
  push(
    'Shutter',
    metadata.shutterSpeedSeconds ? shutterText(metadata.shutterSpeedSeconds) : null
  );
  push('Aperture', metadata.aperture ? `f/${Number(metadata.aperture.toFixed(1))}` : null);
  push(
    'Focal length',
    metadata.focalLengthMm ? `${Math.round(metadata.focalLengthMm)} mm` : null
  );
  push('Captured', metadata.captureTime);
  if (extra.sensorWidth && extra.sensorHeight) {
    push('Sensor', `${extra.sensorWidth} x ${extra.sensorHeight}`);
  }
  push('RAW depth', extra.bitsPerSample ? `${extra.bitsPerSample}-bit` : null);
  push('Filter array', extra.cfaPattern);
  push('Orientation', metadata.orientation ? String(metadata.orientation) : null);
  if (extra.multipliers) {
    push(
      'White balance',
      extra.multipliers.map((value) => Number(value.toFixed(3))).join(', ')
    );
  }
  if (extra.colorManaged !== undefined) {
    // Said plainly either way: a file with no matrix is not colour managed and
    // the panel must not imply otherwise.
    push(
      'Camera profile',
      extra.colorManaged ? 'Camera matrix applied' : 'None in file; camera RGB shown as-is'
    );
  }
  // Enough of the hash to recognise, not so much that it fills the panel.
  push('Source hash', extra.sha256 ? `${extra.sha256.slice(0, 16)}...` : null);
  return rows;
}
