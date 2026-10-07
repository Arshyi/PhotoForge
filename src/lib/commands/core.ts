import type { PluginSummary } from '../plugins/types';
import { pluginCommandId, type PaletteCommand } from './registry';

/**
 * What the palette needs to know about the application, and what it may ask it to do.
 *
 * Each action is a function the interface already has. The palette adds no behaviour:
 * choosing "Merge down" is the same call as pressing the Layers panel's button, so the
 * two cannot disagree about what happens or when it is allowed.
 */
export interface CoreCommandContext {
  hasImage: boolean;
  hasLayers: boolean;
  hasActiveLayer: boolean;
  canUndo: boolean;
  canRedo: boolean;
  /** Something is running that owns the document. */
  busy: boolean;
  hasEdits: boolean;
  /** The document is a region of a larger file. */
  hasSourceRegion: boolean;
  /** At least one plugin filter could run now. */
  hasPluginFilters: boolean;
  shortcutFor: (action: string) => string | undefined;
  actions: {
    openImage: () => void;
    openProject: () => void;
    saveProject: () => void;
    exportImage: () => void;
    undo: () => void;
    redo: () => void;
    resetEdits: () => void;
    toggleCompare: () => void;
    newPixelLayer: () => void;
    newGroup: () => void;
    duplicateLayer: () => void;
    deleteLayer: () => void;
    groupLayers: () => void;
    ungroupLayer: () => void;
    mergeDown: () => void;
    flatten: () => void;
    resetTransform: () => void;
    openSettings: () => void;
    openPlugins: () => void;
    runPluginFilter: () => void;
    changeSourceRegion: () => void;
    openAutomation: () => void;
  };
}

const BUSY = 'Another operation is running. Wait for it to finish.';

/** The built-in commands, in the order the palette shows them with no query. */
export function buildCoreCommands(context: CoreCommandContext): PaletteCommand[] {
  const { actions: a } = context;
  const needs = (condition: boolean, reason: string) => () => (context.busy ? BUSY : condition ? null : reason);
  const always = () => null;
  const noImage = 'Open an image first.';
  const noLayers = 'This needs a layered document: open or create one first.';
  const noLayer = 'Select a layer first.';

  const entry = (
    id: string,
    title: string,
    group: string,
    run: () => void,
    unavailable: () => string | null,
    options: { description?: string; shortcut?: string; keywords?: string[] } = {}
  ): PaletteCommand => ({
    id: `core.${id}`,
    title,
    group,
    run,
    unavailable,
    source: 'core',
    ...options
  });

  return [
    entry('open_image', 'Open image…', 'File', a.openImage, always, {
      shortcut: context.shortcutFor('Open image'),
      keywords: ['import', 'photo', 'raw', 'dng']
    }),
    entry('open_project', 'Open project…', 'File', a.openProject, always, { keywords: ['photoforge'] }),
    entry('save_project', 'Save project', 'File', a.saveProject, needs(context.hasLayers, noLayers)),
    entry('export_image', 'Export image…', 'File', a.exportImage, needs(context.hasImage, noImage), {
      shortcut: context.shortcutFor('Export image'),
      keywords: ['save', 'png', 'jpeg', 'webp']
    }),
    entry('undo', 'Undo', 'Edit', a.undo, needs(context.canUndo, 'There is nothing to undo.'), {
      shortcut: context.shortcutFor('Undo')
    }),
    entry('redo', 'Redo', 'Edit', a.redo, needs(context.canRedo, 'There is nothing to redo.'), {
      shortcut: context.shortcutFor('Redo')
    }),
    entry('reset_edits', 'Reset all edits', 'Edit', a.resetEdits, needs(context.hasEdits, 'There are no edits to reset.'), {
      keywords: ['revert', 'clear']
    }),
    entry('toggle_compare', 'Toggle before and after', 'View', a.toggleCompare, needs(context.hasImage, noImage), {
      shortcut: context.shortcutFor('Compare'),
      keywords: ['comparison', 'original']
    }),
    entry('new_pixel_layer', 'New pixel layer', 'Layer', a.newPixelLayer, needs(context.hasLayers, noLayers)),
    entry('new_group', 'New group', 'Layer', a.newGroup, needs(context.hasLayers, noLayers)),
    entry('duplicate_layer', 'Duplicate layer', 'Layer', a.duplicateLayer, needs(context.hasActiveLayer, noLayer)),
    entry('delete_layer', 'Delete layer', 'Layer', a.deleteLayer, needs(context.hasActiveLayer, noLayer)),
    entry('group_layers', 'Group layers', 'Layer', a.groupLayers, needs(context.hasActiveLayer, noLayer)),
    entry('ungroup_layer', 'Ungroup', 'Layer', a.ungroupLayer, needs(context.hasActiveLayer, noLayer)),
    entry('merge_down', 'Merge down', 'Layer', a.mergeDown, needs(context.hasActiveLayer, noLayer)),
    entry('flatten', 'Flatten image', 'Layer', a.flatten, needs(context.hasLayers, noLayers)),
    entry('reset_transform', 'Reset layer transform', 'Layer', a.resetTransform, needs(context.hasActiveLayer, noLayer)),
    entry('change_source_region', 'Change source region…', 'File', a.changeSourceRegion,
      needs(context.hasSourceRegion, 'This document is not a region of a larger file.'), {
        keywords: ['crop', 'oversized', 'large']
      }),
    entry('plugin_filter', 'Run a plugin filter…', 'Plugins', a.runPluginFilter,
      needs(context.hasPluginFilters, 'No installed plugin has a filter that can run.'), {
        keywords: ['extension', 'wasm']
      }),
    entry('plugins', 'Manage plugins…', 'Plugins', a.openPlugins, always, { keywords: ['extensions', 'install'] }),
    entry('automation', 'Automation…', 'Plugins', a.openAutomation, always, {
      keywords: ['macro', 'record', 'replay', 'workflow']
    }),
    entry('settings', 'Settings…', 'Application', a.openSettings, always, { keywords: ['preferences', 'memory', 'budget'] })
  ];
}

/**
 * Commands and tools from the plugins that may offer them.
 *
 * A command is offered only if the plugin is available *and* was granted the right to
 * change the document; a tool only if it was also granted the tools menu. Anything
 * short of that is not listed at all rather than listed and refused, because a
 * plugin has no standing to appear in the palette beyond what was allowed.
 */
export function buildPluginCommands(
  plugins: PluginSummary[],
  run: (plugin: PluginSummary, command: string) => void,
  context: { hasLayers: boolean; busy: boolean }
): PaletteCommand[] {
  const out: PaletteCommand[] = [];
  for (const plugin of plugins) {
    const manifest = plugin.manifest;
    if (!manifest || plugin.availability.kind !== 'available') continue;
    if (!plugin.granted.includes('document.operations')) continue;
    const unavailable = () =>
      context.busy ? BUSY : context.hasLayers ? null : 'This needs a layered document: open or create one first.';
    const toolled = new Set(plugin.granted.includes('ui.tool') ? (manifest.tools ?? []).map((tool) => tool.command) : []);
    for (const command of manifest.commands ?? []) {
      out.push({
        id: pluginCommandId(plugin.id, command.id),
        title: `${manifest.name}: ${command.title}`,
        description: command.description,
        group: manifest.name,
        keywords: ['plugin', toolled.has(command.id) ? 'tool' : ''].filter(Boolean),
        source: 'plugin',
        unavailable,
        run: () => run(plugin, command.id)
      });
    }
  }
  return out;
}
