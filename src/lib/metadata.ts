import { writable } from 'svelte/store';
import type { MetadataPolicy, MetadataReport, SourceMetadataSummary } from './types/editor';

/**
 * What an export does with the photograph's own EXIF, XMP and IPTC. The default removes what
 * identifies a place or a device and keeps what describes the picture; the person can keep
 * everything or remove everything, and the export always says what it did.
 */
export const DEFAULT_METADATA_POLICY: MetadataPolicy = 'remove_private';

export const METADATA_POLICIES: { value: MetadataPolicy; label: string; hint: string }[] = [
  {
    value: 'remove_private',
    label: 'Remove location and serial numbers',
    hint: 'Keeps camera, exposure, date, title, keywords and copyright. Removes the place, owner and serial numbers, maker notes and embedded previews.'
  },
  {
    value: 'keep_all',
    label: 'Keep everything',
    hint: 'Writes what the source file carried, including where it was taken. The orientation and size are corrected and the embedded preview is dropped, because it shows the picture before editing.'
  },
  {
    value: 'remove_all',
    label: 'Remove everything',
    hint: 'Writes no EXIF, XMP or IPTC at all.'
  }
];

const STORAGE_KEY = 'photoforge.metadataPolicy';

function isPolicy(value: unknown): value is MetadataPolicy {
  return value === 'keep_all' || value === 'remove_private' || value === 'remove_all';
}

/** The remembered choice. Storage can be blocked or empty; the default is then the answer. */
export function loadMetadataPolicy(): MetadataPolicy {
  try {
    const stored = localStorage.getItem(STORAGE_KEY);
    return isPolicy(stored) ? stored : DEFAULT_METADATA_POLICY;
  } catch {
    return DEFAULT_METADATA_POLICY;
  }
}

const current = writable<MetadataPolicy>(DEFAULT_METADATA_POLICY, (set) => {
  // Re-read the remembered choice whenever the first component starts watching.
  set(loadMetadataPolicy());
});

export function saveMetadataPolicy(policy: MetadataPolicy): void {
  current.set(policy);
  try {
    localStorage.setItem(STORAGE_KEY, policy);
  } catch {
    // A per-viewer convenience; nothing depends on it.
  }
}

/**
 * The person's current choice, shared by every place that exports. Components read it as
 * `$metadataPolicy` and bind a select to it; setting it saves the choice, so the Metadata card,
 * the batch panel and every export see the same value and a change in one reaches the others.
 */
export const metadataPolicy = { subscribe: current.subscribe, set: saveMetadataPolicy };

export function policyHint(policy: MetadataPolicy): string {
  return METADATA_POLICIES.find((entry) => entry.value === policy)?.hint ?? '';
}

/** What the file carried, as short phrases for the metadata card. */
export function describeSource(summary: SourceMetadataSummary | null | undefined): string[] {
  if (!summary) return [];
  const lines: string[] = [];
  const camera = [summary.make, summary.model].filter(Boolean).join(' ');
  if (summary.hasExif) lines.push(['EXIF', camera, summary.taken].filter(Boolean).join(' · '));
  if (summary.hasXmp) lines.push('XMP (title, keywords, rating, edit history)');
  if (summary.hasIptc) lines.push('IPTC (caption, credit, copyright)');
  if (summary.hasLocation) lines.push('Includes the place it was taken');
  if (summary.hasMakerNote) lines.push("Includes the camera maker's private notes");
  if (summary.hasIdentifiers) lines.push('Includes an owner name or serial numbers');
  if (summary.orientation !== 1 && summary.orientationApplied) {
    lines.push('Turned upright when opened');
  }
  return lines;
}

/** One sentence for the notification after an export: what was removed, or that nothing was. */
export function metadataSentence(report: MetadataReport | null | undefined): string {
  if (!report) return '';
  const removed = report.notes.filter((note) => note.kind === 'removed').map((note) => note.detail);
  const kept = report.notes.filter((note) => note.kind === 'kept');
  if (removed.length === 0 && kept.length === 0) return '';
  if (removed.length === 0) return 'Metadata kept.';
  const shown = removed.slice(0, 2).join('; ');
  const more = removed.length > 2 ? ` and ${removed.length - 2} more` : '';
  return `Metadata removed: ${shown}${more}.`;
}
