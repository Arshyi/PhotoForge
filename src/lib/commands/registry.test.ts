import { describe, expect, it, vi } from 'vitest';
import { buildCoreCommands, buildPluginCommands, type CoreCommandContext } from './core';
import { CommandRegistry, pluginCommandId, scoreCommand, searchCommands, type PaletteCommand } from './registry';
import type { PluginSummary } from '../plugins/types';

function command(id: string, title: string, extra: Partial<PaletteCommand> = {}): PaletteCommand {
  return {
    id,
    title,
    group: 'Test',
    source: id.startsWith('plugin:') ? 'plugin' : 'core',
    unavailable: () => null,
    run: () => undefined,
    ...extra
  };
}

describe('the command registry', () => {
  it('refuses a repeated id', () => {
    const registry = new CommandRegistry();
    registry.register(command('core.a', 'A'));
    expect(() => registry.register(command('core.a', 'Again'))).toThrow(/already a command/);
  });

  it('keeps each source to its own namespace, so a plugin cannot take a built-in name', () => {
    const registry = new CommandRegistry();
    expect(() => registry.register(command('core.undo', 'Undo', { source: 'plugin' }))).toThrow(/plugin:<plugin>/);
    expect(() => registry.register(command('plugin:x:y', 'Y', { source: 'core' }))).toThrow(/core\.<name>/);
    registry.register(command('plugin:com.example.a:go', 'Go'));
    expect(registry.get('plugin:com.example.a:go')?.source).toBe('plugin');
    expect(pluginCommandId('com.example.a', 'go')).toBe('plugin:com.example.a:go');
  });

  it('refuses a missing or oversized title', () => {
    const registry = new CommandRegistry();
    expect(() => registry.register(command('core.a', '   '))).toThrow();
    expect(() => registry.register(command('core.b', 'x'.repeat(121)))).toThrow();
  });

  it('can drop one source and keep the other, for rebuilding when plugins change', () => {
    const registry = new CommandRegistry();
    registry.register(command('core.a', 'A'));
    registry.register(command('plugin:p.q:go', 'Go'));
    registry.clear('plugin');
    expect(registry.list().map((entry) => entry.id)).toEqual(['core.a']);
    registry.clear();
    expect(registry.list()).toEqual([]);
  });
});

describe('searching commands', () => {
  const commands = [
    command('core.open_image', 'Open image…', { group: 'File', keywords: ['import'] }),
    command('core.open_project', 'Open project…', { group: 'File' }),
    command('core.export_image', 'Export image…', { group: 'File', keywords: ['save'] }),
    command('core.merge_down', 'Merge down', { group: 'Layer' }),
    command('core.undo', 'Undo', { group: 'Edit' })
  ];

  it('returns everything, in order, for an empty query', () => {
    expect(searchCommands(commands, '').map((hit) => hit.command.id)).toEqual(commands.map((entry) => entry.id));
    expect(searchCommands(commands, '   ').length).toBe(commands.length);
  });

  it('requires every word of the query to match', () => {
    expect(searchCommands(commands, 'open proj').map((hit) => hit.command.id)).toEqual(['core.open_project']);
    expect(searchCommands(commands, 'open zzz')).toEqual([]);
  });

  it('ranks the start of a title above the start of a word above the middle of one', () => {
    const ranked = searchCommands(
      [command('core.a', 'Rearrange'), command('core.b', 'Open range'), command('core.c', 'Range check')],
      'range'
    );
    expect(ranked.map((hit) => hit.command.id)).toEqual(['core.c', 'core.b', 'core.a']);
  });

  it('finds a command by its group, keywords or description, but ranks a title match above them', () => {
    expect(searchCommands(commands, 'import').map((hit) => hit.command.id)).toEqual(['core.open_image']);
    expect(searchCommands(commands, 'layer').map((hit) => hit.command.id)).toEqual(['core.merge_down']);
    const withDescription = command('core.x', 'Something', { description: 'rotates the picture' });
    expect(searchCommands([withDescription], 'rotates').length).toBe(1);
    const both = [command('core.g', 'Zzz', { group: 'merge' }), command('core.t', 'Merge')];
    expect(searchCommands(both, 'merge')[0].command.id).toBe('core.t');
  });

  it('keeps registration order between equal matches and honours a limit', () => {
    const same = [command('core.a', 'Tool a'), command('core.b', 'Tool b'), command('core.c', 'Tool c')];
    expect(searchCommands(same, 'tool').map((hit) => hit.command.id)).toEqual(['core.a', 'core.b', 'core.c']);
    expect(searchCommands(same, 'tool', 2).length).toBe(2);
  });

  it('ignores case and punctuation', () => {
    expect(scoreCommand(commands[0], 'OPEN-IMAGE')).toBeGreaterThan(0);
    expect(scoreCommand(commands[0], '')).toBe(1);
  });
});

function context(overrides: Partial<CoreCommandContext> = {}): CoreCommandContext {
  const noop = vi.fn();
  return {
    hasImage: true,
    hasLayers: true,
    hasActiveLayer: true,
    canUndo: true,
    canRedo: false,
    busy: false,
    hasEdits: true,
    hasSourceRegion: false,
    hasPluginFilters: true,
    shortcutFor: (action) => ({ 'Open image': 'Ctrl+O', Undo: 'Ctrl+Z' })[action],
    actions: {
      openImage: noop, openProject: noop, saveProject: noop, exportImage: noop, undo: noop, redo: noop,
      resetEdits: noop, toggleCompare: noop, newPixelLayer: noop, newGroup: noop, duplicateLayer: noop,
      deleteLayer: noop, groupLayers: noop, ungroupLayer: noop, mergeDown: noop, flatten: noop,
      resetTransform: noop, openSettings: noop, openPlugins: noop, runPluginFilter: noop,
      changeSourceRegion: noop, openAutomation: noop
    },
    ...overrides
  };
}

describe('the built-in commands', () => {
  const find = (commands: PaletteCommand[], id: string) => commands.find((entry) => entry.id === id)!;

  it('are all in the core namespace, unique, and carry their shortcut hints', () => {
    const commands = buildCoreCommands(context());
    expect(new Set(commands.map((entry) => entry.id)).size).toBe(commands.length);
    expect(commands.every((entry) => entry.id.startsWith('core.') && entry.source === 'core')).toBe(true);
    expect(find(commands, 'core.open_image').shortcut).toBe('Ctrl+O');
    expect(find(commands, 'core.undo').shortcut).toBe('Ctrl+Z');
    // They can be registered together without complaint.
    const registry = new CommandRegistry();
    for (const entry of commands) registry.register(entry);
    expect(registry.list().length).toBe(commands.length);
  });

  it('call the action the interface already has, and nothing else', () => {
    const calls: string[] = [];
    const actions = Object.fromEntries(
      Object.keys(context().actions).map((name) => [name, () => calls.push(name)])
    ) as unknown as CoreCommandContext['actions'];
    const commands = buildCoreCommands(context({ actions }));
    find(commands, 'core.merge_down').run();
    find(commands, 'core.plugins').run();
    expect(calls).toEqual(['mergeDown', 'openPlugins']);
  });

  it('say why they cannot run instead of disappearing', () => {
    const none = buildCoreCommands(
      context({ hasImage: false, hasLayers: false, hasActiveLayer: false, canUndo: false, hasEdits: false, hasPluginFilters: false })
    );
    expect(find(none, 'core.merge_down').unavailable()).toMatch(/Select a layer/);
    expect(find(none, 'core.save_project').unavailable()).toMatch(/layered document/);
    expect(find(none, 'core.export_image').unavailable()).toMatch(/Open an image/);
    expect(find(none, 'core.undo').unavailable()).toMatch(/nothing to undo/);
    expect(find(none, 'core.reset_edits').unavailable()).toMatch(/no edits/);
    expect(find(none, 'core.plugin_filter').unavailable()).toMatch(/No installed plugin/);
    expect(find(none, 'core.change_source_region').unavailable()).toMatch(/not a region/);
    // Opening and settings need nothing.
    expect(find(none, 'core.open_image').unavailable()).toBeNull();
    expect(find(none, 'core.settings').unavailable()).toBeNull();

    const busy = buildCoreCommands(context({ busy: true }));
    expect(find(busy, 'core.merge_down').unavailable()).toMatch(/running/);
    expect(find(buildCoreCommands(context()), 'core.merge_down').unavailable()).toBeNull();
  });
});

function plugin(overrides: Partial<PluginSummary> & { id?: string } = {}): PluginSummary {
  return {
    id: 'com.example.shapes',
    enabled: true,
    granted: ['filter.pixels', 'document.operations', 'ui.tool'],
    activeHash: 'a'.repeat(64),
    versions: [],
    availability: { kind: 'available' },
    readme: null,
    license: null,
    manifest: {
      format: 'photoforge-plugin', formatVersion: 1, apiVersion: 1,
      id: 'com.example.shapes', name: 'Shapes', version: '1.0.0', publisher: 'Example',
      capabilities: ['filter.pixels', 'document.operations', 'ui.tool'],
      commands: [
        { id: 'add_shape', title: 'Add shape', description: 'Draws one', steps: [] },
        { id: 'other', title: 'Other', steps: [] }
      ],
      tools: [{ id: 't', title: 'T', command: 'add_shape' }]
    },
    ...overrides
  };
}

describe('plugin commands', () => {
  const ctx = { hasLayers: true, busy: false };

  it('are listed under the plugin, with a namespaced id that cannot clash with a built-in', () => {
    const run = vi.fn();
    const commands = buildPluginCommands([plugin()], run, ctx);
    expect(commands.map((entry) => entry.id)).toEqual([
      'plugin:com.example.shapes:add_shape',
      'plugin:com.example.shapes:other'
    ]);
    expect(commands[0].title).toBe('Shapes: Add shape');
    expect(commands[0].source).toBe('plugin');
    commands[0].run();
    expect(run).toHaveBeenCalledWith(expect.objectContaining({ id: 'com.example.shapes' }), 'add_shape');
    const registry = new CommandRegistry();
    for (const entry of commands) registry.register(entry);
  });

  it('are offered only by a plugin that is available and was allowed to change the document', () => {
    const run = vi.fn();
    expect(buildPluginCommands([plugin({ granted: ['filter.pixels'] })], run, ctx)).toEqual([]);
    expect(buildPluginCommands([plugin({ availability: { kind: 'disabled' } })], run, ctx)).toEqual([]);
    expect(buildPluginCommands([plugin({ availability: { kind: 'missing' } })], run, ctx)).toEqual([]);
    expect(buildPluginCommands([plugin({ manifest: null })], run, ctx)).toEqual([]);
  });

  it('say so when there is no document to change', () => {
    const [first] = buildPluginCommands([plugin()], vi.fn(), { hasLayers: false, busy: false });
    expect(first.unavailable()).toMatch(/layered document/);
    const [busy] = buildPluginCommands([plugin()], vi.fn(), { hasLayers: true, busy: true });
    expect(busy.unavailable()).toMatch(/running/);
  });
});
