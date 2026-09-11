import { describe, expect, it } from 'vitest';
import {
  createDocument,
  createPixelLayer,
  createSmartObjectLayer,
  duplicateSmartObjectIndependent,
  findLayer
} from './tree';
import { validateSmartSources } from './smart';

function smartDocument() {
  const sourceLayer = createPixelLayer('Source pixels', 'source-pixels', 32, 24);
  const instance = createSmartObjectLayer('Logo', 'source');
  const document = createDocument(96, 64, [instance], 'linear_srgb_f32');
  document.smartSources = {
    source: { width: 32, height: 24, layers: [sourceLayer], link: null }
  };
  document.activeLayerId = instance.id;
  return document;
}

describe('smart-object source helpers', () => {
  it('accepts a valid source DAG and rejects a root/source ID collision', () => {
    const document = smartDocument();
    expect(validateSmartSources(document)).toEqual([]);
    document.smartSources!.source.layers[0] = {
      ...document.smartSources!.source.layers[0],
      id: document.layers[0].id
    };
    expect(validateSmartSources(document).some((problem) => problem.includes('duplicate'))).toBe(true);
  });

  it('clones the referenced source for an independent instance', () => {
    const document = smartDocument();
    const originalId = document.layers[0].id;
    const result = duplicateSmartObjectIndependent(document, originalId);
    expect(result.layer).not.toBeNull();
    expect(result.document.layers).toHaveLength(2);
    const copy = result.layer!;
    expect(copy.content.type).toBe('smart_object');
    if (copy.content.type !== 'smart_object') return;
    expect(copy.content.sourceId).not.toBe('source');
    expect(Object.keys(result.document.smartSources ?? {})).toHaveLength(2);
    const copiedSource = result.document.smartSources![copy.content.sourceId];
    expect(copiedSource.layers[0].id).not.toBe(document.smartSources!.source.layers[0].id);
    expect(findLayer(result.document, originalId)?.content).toMatchObject({ type: 'smart_object', sourceId: 'source' });
  });
});
