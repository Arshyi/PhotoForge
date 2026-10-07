/**
 * Types for plugins. They mirror `plugins::{manifest, store, document}` in the
 * backend, which is the only place that decides what a plugin may do and whether
 * a document can have the one it needs. Nothing here decides either: the interface
 * shows what the backend reports and asks the backend to act.
 */
import type { LayerDocument } from '../layers/types';
import type { MaskSnapshot } from '../selections/types';

export type CapabilityId =
  | 'filter.pixels'
  | 'document.read'
  | 'document.operations'
  | 'ui.panel'
  | 'ui.tool';

export type Locality =
  | { kind: 'pointwise' }
  | { kind: 'local'; radius: number }
  | { kind: 'global' };

export type ParamKind =
  | { type: 'number'; min: number; max: number; default: number; step?: number | null }
  | { type: 'integer'; min: number; max: number; default: number }
  | { type: 'bool'; default: boolean }
  | { type: 'choice'; options: string[]; default: number };

export type ParamDecl = ParamKind & {
  id: string;
  title: string;
  description?: string;
};

export interface FilterDecl {
  id: string;
  title: string;
  description?: string;
  locality: Locality;
  deterministic?: boolean;
  parameters?: ParamDecl[];
}

export interface StepDecl {
  op: string;
  params?: unknown;
}

export interface CommandDecl {
  id: string;
  title: string;
  description?: string;
  parameters?: ParamDecl[];
  steps: StepDecl[];
}

export type HostFact =
  | 'layer_count'
  | 'pixel_layer_count'
  | 'canvas_width'
  | 'canvas_height'
  | 'precision'
  | 'active_layer_name'
  | 'active_layer_kind'
  | 'active_layer_opacity';

export type PanelRow =
  | { kind: 'text'; text: string }
  | { kind: 'fact'; label: string; fact: HostFact }
  | { kind: 'command'; label: string; command: string };

export interface PanelDecl {
  id: string;
  title: string;
  rows: PanelRow[];
}

export interface ToolDecl {
  id: string;
  title: string;
  description?: string;
  command: string;
}

export interface PluginManifest {
  format: 'photoforge-plugin';
  formatVersion: number;
  id: string;
  name: string;
  version: string;
  apiVersion: number;
  publisher: string;
  description?: string;
  license?: string;
  entry?: { path: string; sha256: string } | null;
  capabilities?: CapabilityId[];
  limits?: { memoryMib: number } | null;
  filters?: FilterDecl[];
  commands?: CommandDecl[];
  panels?: PanelDecl[];
  tools?: ToolDecl[];
}

export type Availability =
  | { kind: 'available' }
  | { kind: 'missing' }
  | { kind: 'disabled' }
  | { kind: 'notGranted'; capability: string }
  | { kind: 'runtimeUnavailable' }
  | { kind: 'damaged'; reason: string }
  | { kind: 'otherVersion'; installedVersion: string; installedHash: string };

export interface VersionRecord {
  contentHash: string;
  version: string;
  installedAt: string;
  packageSha256: string;
}

export interface PluginSummary {
  id: string;
  enabled: boolean;
  granted: CapabilityId[];
  activeHash: string;
  versions: VersionRecord[];
  manifest: PluginManifest | null;
  availability: Availability;
  readme: string | null;
  license: string | null;
}

export interface PluginStatus {
  runtimeAvailable: boolean;
  pluginFolder: string;
  warning: string | null;
  plugins: PluginSummary[];
}

export interface CapabilityInfo {
  id: CapabilityId;
  description: string;
}

export interface Inspection {
  manifest: PluginManifest;
  contentHash: string;
  packageSha256: string;
  packageBytes: number;
  capabilities: CapabilityInfo[];
  installedVersion: string | null;
  alreadyInstalled: boolean;
  olderThanInstalled: boolean;
  newCapabilities: CapabilityInfo[];
  hasModule: boolean;
  runtimeAvailable: boolean;
  readme: string | null;
  license: string | null;
  signature: string;
}

export interface FilterTest {
  filter: string;
  passed: boolean;
  message: string;
  tiles: number;
  fuelUsed: number;
}

export interface TestReport {
  plugin: string;
  passed: boolean;
  filters: FilterTest[];
}

export interface InstallReport {
  plugin: string;
  version: string;
  contentHash: string;
  updatedFrom: string | null;
  test: TestReport;
}

export interface RequirementStatus {
  plugin: string;
  version: string;
  sha256: string;
  filters: string[];
  layerIds: string[];
  availability: Availability;
  message: string;
}

/** A layer left out of a render because its plugin is not available. */
export interface MissingPlugin {
  layerId: string;
  layerName: string;
  plugin: string;
  version: string;
  message: string;
}

export interface RunPluginCommand {
  plugin: string;
  command: string;
  values?: Record<string, number>;
  document: LayerDocument;
  selection?: MaskSnapshot | null;
  expectedRevision?: string | null;
}

/** What a person chose in the plugin filter dialog. */
export interface FilterRun {
  /** `adjustment` adds a layer that keeps the filter live; `pixels` bakes it into a layer. */
  mode: 'adjustment' | 'pixels';
  plugin: string;
  filter: string;
  values: Record<string, number>;
}
