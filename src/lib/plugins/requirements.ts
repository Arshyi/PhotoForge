import type { LayerDocument } from '../layers/types';
import { eachLayer } from '../layers/tree';
import type { EditOperation } from '../types/editor';

interface Reference {
  plugin: string;
  version: string;
  sha256: string;
}

function referenceOf(operation: EditOperation | undefined): Reference | null {
  if (!operation) return null;
  if (operation.type === 'plugin_filter') {
    return { plugin: operation.plugin, version: operation.version, sha256: operation.sha256 };
  }
  if (operation.type === 'masked') return referenceOf(operation.operation);
  return null;
}

/**
 * A short string naming every plugin version the document and its operation list use,
 * or the empty string for a document that uses none.
 *
 * It exists so the interface asks the backend whether those plugins are available only
 * when there is something to ask about, and only again when the answer could have
 * changed: this is a cheap scan of values already in memory, and says nothing about
 * whether a plugin is installed.
 */
export function pluginReferenceKey(document: LayerDocument | null, operations: EditOperation[]): string {
  const found = new Set<string>();
  const add = (reference: Reference | null) => {
    if (reference) found.add(`${reference.plugin}@${reference.version}#${reference.sha256}`);
  };
  for (const operation of operations) add(referenceOf(operation));
  if (document) {
    for (const layer of eachLayer(document)) {
      if (layer.content.type === 'adjustment') add(referenceOf(layer.content.operation as EditOperation));
    }
    for (const source of Object.values(document.smartSources ?? {})) {
      for (const layer of source.layers ?? []) {
        if (layer.content.type === 'adjustment') add(referenceOf(layer.content.operation as EditOperation));
      }
    }
  }
  return [...found].sort().join('|');
}
