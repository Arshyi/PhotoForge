import { MAX_GROUP_DEPTH, MAX_LAYERS, type Layer, type LayerDocument, type SmartSource, type SmartSources } from './types';

/** These limits mirror the Rust smart-source trust boundary. */
export const MAX_SMART_SOURCES = 512;
export const MAX_SMART_DEPTH = 4;
export const MAX_SMART_SOURCE_EDGE = 32_768;
export const MAX_SMART_COMPOSITE_BYTES = 536_870_912;

/** Native source size is also the placement pivot; it is not the parent canvas. */
export function sourceDimensions(layer: Layer, document: LayerDocument): { width: number; height: number } {
  if (layer.content.type === 'pixel') return { width: layer.content.width, height: layer.content.height };
  if (layer.content.type === 'smart_object') {
    const source = document.smartSources?.[layer.content.sourceId];
    if (!source) throw new Error(`Smart source ${layer.content.sourceId} is missing.`);
    return { width: source.width, height: source.height };
  }
  return { width: document.canvasWidth, height: document.canvasHeight };
}

/** Opens source content as an ordinary editable subdocument, without rasterising. */
export function smartSourceDocument(document: LayerDocument, sourceId: string): LayerDocument {
  const source = document.smartSources?.[sourceId];
  if (!source) throw new Error(`Smart source ${sourceId} is missing.`);
  const problems = validateSmartSources(document);
  if (problems.length) throw new Error(problems[0]);
  // A source editor is a real subdocument, not the parent document with the
  // same layers duplicated in its registry. Keep exactly the nested DAG needed
  // by this stack; ancestors would introduce dangling or duplicate identities.
  const nested: SmartSources = {};
  const pending = [...source.layers];
  while (pending.length) {
    const layer = pending.pop()!;
    if (layer.content.type === 'group') pending.push(...layer.content.children);
    else if (layer.content.type === 'smart_object' && !Object.hasOwn(nested, layer.content.sourceId)) {
      const child = document.smartSources![layer.content.sourceId];
      nested[layer.content.sourceId] = child;
      pending.push(...child.layers);
    }
  }
  return {
    ...document,
    precision: 'linear_srgb_f32',
    canvasWidth: source.width,
    canvasHeight: source.height,
    layers: source.layers,
    activeLayerId: source.layers.at(-1)?.id ?? null,
    smartSources: nested
  };
}

/**
 * Saves a source editor's immutable tree into its parent document. The parent
 * instances are deliberately not rebuilt: transforms and masks stay independent
 * and the source registry is captured in the same undo entry as any other edit.
 * Editing imported contents detaches the link; the external file is never written.
 * The backend must still validate this candidate before the caller commits it.
 */
export function replaceSmartSourceDocument(
  document: LayerDocument,
  sourceId: string,
  edited: LayerDocument
): LayerDocument {
  if (!document.smartSources?.[sourceId]) throw new Error(`Smart source ${sourceId} is missing.`);
  const replacement: SmartSource = {
    width: edited.canvasWidth,
    height: edited.canvasHeight,
    layers: edited.layers
  };
  const candidate = {
    ...document,
    smartSources: { ...document.smartSources, ...edited.smartSources, [sourceId]: replacement }
  };
  const problems = validateSmartSources(candidate);
  if (problems.length) throw new Error(problems[0]);
  return candidate;
}

/** Validate the DAG once, instead of expanding shared instances exponentially. */
export function validateSmartSources(document: LayerDocument): string[] {
  const sources = document.smartSources ?? {};
  const problems: string[] = [];
  const entries = Object.entries(sources);
  if (entries.length > MAX_SMART_SOURCES) return [`A document may contain at most ${MAX_SMART_SOURCES} smart sources.`];
  const validId = (id: string) => /^[A-Za-z0-9_-]{1,64}$/.test(id);
  const edges = new Map<string, Set<string>>();
  // Layer identifiers are document-global in Rust, including layers held by
  // source trees. Keeping one set here prevents the UI from accepting a tree
  // that will only fail later at the Tauri boundary.
  const globalLayerIds = new Set<string>();
  const collect = (layers: Layer[], label: string): Set<string> => {
    const references = new Set<string>();
    const pending = layers.map((layer) => ({ layer, depth: 1 }));
    let count = 0;
    while (pending.length) {
      const { layer, depth } = pending.pop()!;
      if (++count > MAX_LAYERS || depth > MAX_GROUP_DEPTH) {
        problems.push(`${label} exceeds the layer count or group nesting limit.`);
        break;
      }
      if (!validId(layer.id) || globalLayerIds.has(layer.id)) problems.push(`${label} has an invalid or duplicate layer identifier.`);
      globalLayerIds.add(layer.id);
      switch (layer.content.type) {
        case 'smart_object':
          if (!validId(layer.content.sourceId) || !Object.hasOwn(sources, layer.content.sourceId)) {
            problems.push(`Smart source ${layer.content.sourceId} is missing or invalid.`);
          } else references.add(layer.content.sourceId);
          break;
        case 'group':
          pending.push(...layer.content.children.map((child) => ({ layer: child, depth: depth + 1 })));
          break;
        case 'pixel': case 'adjustment': case 'text': case 'shape': break;
        default: problems.push(`${label} contains an unsupported layer kind.`);
      }
    }
    return references;
  };
  const roots = collect(document.layers, 'The document');
  let compositeBytes = 0;
  for (const [id, source] of entries) {
    if (!validId(id)) problems.push(`Smart source ${id} has an invalid identifier.`);
    if (![source.width, source.height].every((value) => Number.isInteger(value) && value > 0 && value <= MAX_SMART_SOURCE_EDGE)) {
      problems.push(`Smart source ${id} has invalid dimensions.`);
    }
    compositeBytes += source.width * source.height * 16;
    if (source.link && (!validLocalLinkPath(source.link.path) || Array.from(source.link.path).length > 4096 ||
      !/^[a-fA-F0-9]{64}$/.test(source.link.digest) || !Number.isSafeInteger(source.link.bytes) || source.link.bytes < 0)) {
      problems.push(`Smart source ${id} has invalid linked-file metadata.`);
    }
    edges.set(id, collect(source.layers, `Smart source ${id}`));
  }
  if (compositeBytes > MAX_SMART_COMPOSITE_BYTES) problems.push('Smart source composites exceed the 512 MiB memory budget.');
  const visiting = new Set<string>();
  const depths = new Map<string, number>();
  const depth = (id: string): number => {
    if (visiting.has(id)) {
      problems.push('Smart objects cannot contain a reference cycle.');
      return MAX_SMART_DEPTH + 1;
    }
    // Stop before the JavaScript call stack becomes the resource limit.
    if (visiting.size >= MAX_SMART_DEPTH) return MAX_SMART_DEPTH + 1;
    const known = depths.get(id);
    if (known !== undefined) return known;
    visiting.add(id);
    let result = 1;
    for (const child of edges.get(id) ?? []) result = Math.max(result, 1 + depth(child));
    visiting.delete(id);
    depths.set(id, result);
    return result;
  };
  // Unplaced sources are validated too: a later placement must not awaken an
  // unchecked cycle that was hidden in the registry at project-load time.
  for (const id of new Set([...roots, ...sourcesKeys(sources)])) {
    if (depth(id) > MAX_SMART_DEPTH) problems.push(`Smart objects may nest at most ${MAX_SMART_DEPTH} levels.`);
  }
  return [...new Set(problems)];
}

function sourcesKeys(sources: SmartSources): string[] { return Object.keys(sources); }

/** Links are inert local metadata; UNC, URLs, ADS and traversal are refused. */
function validLocalLinkPath(path: string): boolean {
  const normalised = path.replaceAll('\\', '/');
  return !path.includes('\0') && !normalised.startsWith('//') &&
    (/^[A-Za-z]:\//.test(normalised) || normalised.startsWith('/')) &&
    !normalised.split('/').some((part) => part === '.' || part === '..') &&
    !normalised.slice(2).includes(':');
}
