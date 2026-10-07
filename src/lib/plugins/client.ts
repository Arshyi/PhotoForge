import { invoke } from '@tauri-apps/api/core';
import type { EditOperation } from '../types/editor';
import type { LayerDocument } from '../layers/types';
import type { TransactionResult } from '../operations/types';
import type {
  InstallReport,
  Inspection,
  PluginStatus,
  RequirementStatus,
  RunPluginCommand,
  TestReport
} from './types';

/** Every installed plugin, and whether this build can run them at all. */
export function listPlugins(): Promise<PluginStatus> {
  return invoke<PluginStatus>('list_plugins');
}

/** Describes a package file: everything installing it would allow. Changes nothing. */
export function inspectPluginPackage(path: string): Promise<Inspection> {
  return invoke<Inspection>('inspect_plugin_package', { path });
}

/** Installs the package that was inspected, granting exactly what was chosen. */
export function installPluginPackage(
  path: string,
  expectedHash: string,
  grant: string[]
): Promise<InstallReport> {
  return invoke<InstallReport>('install_plugin_package', { path, expectedHash, grant });
}

export function setPluginEnabled(plugin: string, enabled: boolean): Promise<void> {
  return invoke<void>('set_plugin_enabled', { plugin, enabled });
}

export function setPluginGrants(plugin: string, grant: string[]): Promise<void> {
  return invoke<void>('set_plugin_grants', { plugin, grant });
}

export function removePlugin(plugin: string): Promise<void> {
  return invoke<void>('remove_plugin', { plugin });
}

export function removePluginVersion(plugin: string, contentHash: string): Promise<void> {
  return invoke<void>('remove_plugin_version', { plugin, contentHash });
}

/** Runs the plugin's self-test again: every filter, twice, whole and tiled. */
export function testPlugin(plugin: string): Promise<TestReport> {
  return invoke<TestReport>('test_plugin', { plugin });
}

/** What a document and its operation list need from plugins, and whether each can be had. */
export function documentPluginStatus(
  document: LayerDocument | null,
  operations: EditOperation[]
): Promise<RequirementStatus[]> {
  return invoke<RequirementStatus[]>('document_plugin_status', { document, operations });
}

export function pluginRememberedValues(plugin: string, key: string): Promise<Record<string, number>> {
  return invoke<Record<string, number>>('plugin_remembered_values', { plugin, key });
}

/** Remembers the values last used for a filter or command, so its dialog opens there. */
export function rememberPluginValues(
  plugin: string,
  key: string,
  values: Record<string, number>
): Promise<void> {
  return invoke<void>('remember_plugin_values', { plugin, key, values });
}

/** Runs one plugin command as one transaction: all of it, or none of it. */
export function runPluginCommand(request: RunPluginCommand): Promise<TransactionResult> {
  return invoke<TransactionResult>('run_plugin_command', { request });
}
