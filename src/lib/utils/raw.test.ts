import { describe, expect, it } from 'vitest';
import {
  defaultDevelopment,
  isRawPath,
  describeSourceStatus,
  isSourceAvailable,
  metadataRows
} from './raw';
import type { RawCaptureMetadata } from '../types/editor';

function metadata(overrides: Partial<RawCaptureMetadata> = {}): RawCaptureMetadata {
  return {
    manufacturer: null,
    model: null,
    lens: null,
    focalLengthMm: null,
    aperture: null,
    shutterSpeedSeconds: null,
    iso: null,
    captureTime: null,
    orientation: null,
    exposureCompensation: null,
    whiteBalanceMultipliers: null,
    ...overrides
  };
}

function rowValue(rows: { label: string; value: string }[], label: string): string | undefined {
  return rows.find((row) => row.label === label)?.value;
}

describe('RAW metadata rows', () => {
  /// Values taken from the Canon EOS 5D Mark III sample the decoder is
  /// validated against, so the formatting is exercised on real camera output.
  const canon = metadata({
    manufacturer: 'Canon',
    model: 'Canon EOS 5D Mark III',
    lens: 'EF70-200mm f/2.8L IS II USM',
    iso: 200,
    shutterSpeedSeconds: 0.008,
    aperture: 2.8,
    focalLengthMm: 70,
    captureTime: '2017:01:05 13:52:55',
    orientation: 1
  });

  it('formats what the camera recorded the way a photographer reads it', () => {
    const rows = metadataRows(canon, {
      sensorWidth: 5920,
      sensorHeight: 3950,
      bitsPerSample: 16,
      cfaPattern: 'RGGB'
    });
    expect(rowValue(rows, 'Camera')).toBe('Canon Canon EOS 5D Mark III');
    expect(rowValue(rows, 'Lens')).toBe('EF70-200mm f/2.8L IS II USM');
    expect(rowValue(rows, 'ISO')).toBe('200');
    // A fraction, not 0.008 seconds.
    expect(rowValue(rows, 'Shutter')).toBe('1/125 s');
    expect(rowValue(rows, 'Aperture')).toBe('f/2.8');
    expect(rowValue(rows, 'Focal length')).toBe('70 mm');
    expect(rowValue(rows, 'Sensor')).toBe('5920 x 3950');
    expect(rowValue(rows, 'RAW depth')).toBe('16-bit');
    expect(rowValue(rows, 'Filter array')).toBe('RGGB');
  });

  it('shows a long exposure in seconds rather than as a fraction', () => {
    const rows = metadataRows(metadata({ shutterSpeedSeconds: 30 }));
    expect(rowValue(rows, 'Shutter')).toBe('30 s');
    const half = metadataRows(metadata({ shutterSpeedSeconds: 0.5 }));
    expect(rowValue(half, 'Shutter')).toBe('1/2 s');
  });

  /// The rule the panel exists to obey: a field the file did not carry is
  /// absent, never guessed at and never shown as a placeholder.
  it('invents nothing for fields the file did not carry', () => {
    const rows = metadataRows(metadata({ manufacturer: 'Leica' }));
    expect(rowValue(rows, 'Camera')).toBe('Leica');
    for (const absent of ['Lens', 'ISO', 'Shutter', 'Aperture', 'Focal length', 'Captured']) {
      expect(rowValue(rows, absent)).toBeUndefined();
    }
    expect(rows.every((row) => row.value.trim().length > 0)).toBe(true);
  });

  it('produces no rows at all for a file with no metadata', () => {
    expect(metadataRows(metadata())).toEqual([]);
  });

  it('says plainly when a file carries no camera profile', () => {
    const managed = metadataRows(metadata(), { colorManaged: true });
    expect(rowValue(managed, 'Camera profile')).toBe('Camera matrix applied');
    const unmanaged = metadataRows(metadata(), { colorManaged: false });
    expect(rowValue(unmanaged, 'Camera profile')).toContain('None in file');
  });

  it('shows the white balance actually applied and a short source hash', () => {
    const rows = metadataRows(metadata(), {
      multipliers: [1.649_413_8, 1, 2.165_041],
      sha256: 'a'.repeat(64)
    });
    expect(rowValue(rows, 'White balance')).toBe('1.649, 1, 2.165');
    expect(rowValue(rows, 'Source hash')).toBe(`${'a'.repeat(16)}...`);
  });
});

describe('RAW source status', () => {
  it('treats only an available source as usable', () => {
    expect(isSourceAvailable('available')).toBe(true);
    expect(isSourceAvailable('missing')).toBe(false);
    expect(isSourceAvailable({ changed: { foundSha256: 'b'.repeat(64) } })).toBe(false);
  });

  it('explains each state in a sentence a photographer can act on', () => {
    expect(describeSourceStatus('available', 'IMG_1.dng')).toContain('linked and unchanged');
    expect(describeSourceStatus('missing', 'IMG_1.dng')).toContain('Locate it');
    // The dangerous case has to be unmistakable.
    const changed = describeSourceStatus({ changed: { foundSha256: 'c'.repeat(64) } }, 'IMG_1.dng');
    expect(changed).toContain('different photograph');
    expect(changed).toContain('will not use it');
  });
});

describe('RAW path routing', () => {
  it('routes DNG files, whatever the case of the extension', () => {
    expect(isRawPath('C:/photos/IMG_0001.dng')).toBe(true);
    expect(isRawPath('C:/photos/IMG_0001.DNG')).toBe(true);
    expect(isRawPath('/home/a/shot.Dng')).toBe(true);
    expect(isRawPath('  shot.dng  ')).toBe(true);
  });

  /**
   * The other camera extensions are recognised by the backend so it can explain
   * itself, but routing one to the decoder would promise a decode that does not
   * exist. They must stay on the ordinary path and fail with a real message.
   */
  it('does not route a format this build cannot decode', () => {
    for (const path of ['a.cr2', 'a.cr3', 'a.nef', 'a.arw', 'a.raf', 'a.orf', 'a.rw2']) {
      expect(isRawPath(path)).toBe(false);
    }
  });

  it('does not route ordinary raster images', () => {
    for (const path of ['a.png', 'a.jpg', 'a.jpeg', 'a.webp', 'a.dng.png', 'dng', '']) {
      expect(isRawPath(path)).toBe(false);
    }
  });
});

describe('default development', () => {
  it('starts as shot with every control neutral', () => {
    const parameters = defaultDevelopment();
    expect(parameters.whiteBalance.mode).toBe('asShot');
    expect(parameters.exposureEv).toBe(0);
    for (const value of [
      parameters.contrast,
      parameters.highlights,
      parameters.shadows,
      parameters.whites,
      parameters.blacks
    ]) {
      expect(value).toBe(0);
    }
  });
});
