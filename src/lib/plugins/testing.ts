import type {
  Availability,
  Inspection,
  PluginManifest,
  PluginStatus,
  PluginSummary,
  TestReport
} from './types';

/** A manifest shaped like the Shape Generator example. */
export function manifestFixture(overrides: Partial<PluginManifest> = {}): PluginManifest {
  return {
    format: 'photoforge-plugin',
    formatVersion: 1,
    apiVersion: 1,
    id: 'photoforge.example.shapes',
    name: 'Shape Generator',
    version: '1.0.0',
    publisher: 'PhotoForge examples',
    description: 'Draws a circle, square or diamond onto a new layer.',
    license: 'MIT',
    entry: { path: 'plugin.wasm', sha256: 'e'.repeat(64) },
    capabilities: ['filter.pixels', 'document.operations', 'ui.tool'],
    filters: [
      {
        id: 'shape',
        title: 'Draw shape',
        description: 'A generator: it ignores the layer and draws the shape.',
        locality: { kind: 'pointwise' },
        parameters: [
          { id: 'shape', title: 'Shape', type: 'choice', options: ['Circle', 'Square', 'Diamond'], default: 0 },
          { id: 'size', title: 'Size', type: 'number', min: 0.05, max: 1, default: 0.5, description: 'Fraction of the short side.' }
        ]
      }
    ],
    commands: [
      {
        id: 'add_shape',
        title: 'Add shape layer',
        description: 'Adds a new layer and draws the shape on it.',
        parameters: [
          { id: 'shape', title: 'Shape', type: 'choice', options: ['Circle', 'Square', 'Diamond'], default: 0 },
          { id: 'size', title: 'Size', type: 'number', min: 0.05, max: 1, default: 0.5 }
        ],
        steps: [
          { op: 'core.layer.add_pixel', params: { name: 'Shape' } },
          { op: 'core.plugin.apply_filter', params: { plugin: '$self', filter: 'shape' } }
        ]
      }
    ],
    tools: [{ id: 'add_shape', title: 'Add shape', description: 'Draw a shape on a new layer.', command: 'add_shape' }],
    ...overrides
  };
}

/** The Document Inspector: a panel and a command, and no module. */
export function inspectorManifest(): PluginManifest {
  return {
    format: 'photoforge-plugin',
    formatVersion: 1,
    apiVersion: 1,
    id: 'photoforge.example.inspector',
    name: 'Document Inspector',
    version: '1.0.0',
    publisher: 'PhotoForge examples',
    capabilities: ['ui.panel', 'document.read', 'document.operations'],
    commands: [
      {
        id: 'add_inspected_group',
        title: 'Add a group called Inspected',
        steps: [{ op: 'core.layer.add_group', params: { name: 'Inspected' } }]
      }
    ],
    panels: [
      {
        id: 'facts',
        title: 'Document facts',
        rows: [
          { kind: 'text', text: 'Read from the open document.' },
          { kind: 'fact', label: 'Layers', fact: 'layer_count' },
          { kind: 'fact', label: 'Canvas width', fact: 'canvas_width' },
          { kind: 'fact', label: 'Selected layer', fact: 'active_layer_name' },
          { kind: 'command', label: 'Add a group called Inspected', command: 'add_inspected_group' }
        ]
      }
    ]
  };
}

export function pluginFixture(
  manifest: PluginManifest = manifestFixture(),
  overrides: Partial<PluginSummary> = {}
): PluginSummary {
  return {
    id: manifest.id,
    enabled: true,
    granted: [...(manifest.capabilities ?? [])],
    activeHash: 'a'.repeat(64),
    versions: [{ contentHash: 'a'.repeat(64), version: manifest.version, installedAt: '2026-10-06T00:00:00.000Z', packageSha256: 'b'.repeat(64) }],
    manifest,
    availability: { kind: 'available' } as Availability,
    readme: 'An example plugin.',
    license: 'MIT',
    ...overrides
  };
}

export function statusFixture(plugins: PluginSummary[] = [], overrides: Partial<PluginStatus> = {}): PluginStatus {
  return {
    runtimeAvailable: true,
    pluginFolder: 'C:\\Users\\Test\\AppData\\Local\\PhotoForge\\plugins',
    warning: null,
    plugins,
    ...overrides
  };
}

export function inspectionFixture(overrides: Partial<Inspection> = {}): Inspection {
  const manifest = overrides.manifest ?? manifestFixture();
  return {
    manifest,
    contentHash: 'c'.repeat(64),
    packageSha256: 'd'.repeat(64),
    packageBytes: 1715,
    capabilities: [
      { id: 'filter.pixels', description: 'Run its filters on the pixels of layers you choose' },
      { id: 'document.operations', description: 'Change the document with its commands (undoable, and respecting locked layers)' },
      { id: 'ui.tool', description: 'Add entries to the tools menu' }
    ],
    installedVersion: null,
    alreadyInstalled: false,
    olderThanInstalled: false,
    newCapabilities: [],
    hasModule: true,
    runtimeAvailable: true,
    readme: 'Shape Generator adds a layer and draws a shape on it.',
    license: 'MIT',
    signature: 'Not signed. PhotoForge does not verify who made a plugin; what it limits is what a plugin can do.',
    ...overrides
  };
}

export function passingTest(plugin = 'photoforge.example.shapes'): TestReport {
  return { plugin, passed: true, filters: [{ filter: 'shape', passed: true, message: 'ok', tiles: 12, fuelUsed: 1000 }] };
}
