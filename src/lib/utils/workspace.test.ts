import { describe, expect, it } from 'vitest';
import type { ShortcutBinding, WorkspaceLayout } from '../types/editor';
import {
  applyWorkspace,
  defaultShortcuts,
  defaultWorkspace,
  exportSettings,
  importSettings,
  loadShortcuts,
  loadWorkspaces,
  normalizeShortcut,
  saveShortcuts,
  saveWorkspaces,
  shortcutConflicts,
  validWorkspace
} from './workspace';

describe('workspace and shortcuts', () => {
  it.each([
    ['Ctrl + O', 'ctrl+o'],
    ['CTRL+SHIFT+S', 'ctrl+shift+s'],
    ['  I ', 'i'],
    ['Alt + 1', 'alt+1']
  ])('normalizes %s', (input, expected) => expect(normalizeShortcut(input)).toBe(expected));

  it('detects conflicts case-insensitively', () => {
    expect(shortcutConflicts([{ action: 'A', keys: 'Ctrl+O' }, { action: 'B', keys: 'ctrl + o' }])).toHaveLength(1);
  });

  it('allows unique shortcuts', () => expect(shortcutConflicts(defaultShortcuts)).toEqual([]));

  it('detects empty shortcuts', () => expect(shortcutConflicts([{ action: 'A', keys: '' }])[0]).toMatch(/no shortcut/));

  it.each([
    [{ ...defaultWorkspace, schemaVersion: 2 }, false],
    [{ ...defaultWorkspace, name: '' }, false],
    [{ ...defaultWorkspace, leftPanelWidth: 10 }, false],
    [{ ...defaultWorkspace, rightPanelWidth: 1000 }, false],
    [{ ...defaultWorkspace, uiScale: 0.5 }, false],
    [defaultWorkspace, true]
  ] as [WorkspaceLayout, boolean][])('validates workspace bounds', (layout, valid) => expect(validWorkspace(layout)).toBe(valid));

  it('applies scale, width, and high contrast to root', () => {
    const root = document.createElement('div');
    applyWorkspace({ ...defaultWorkspace, rightPanelWidth: 420, uiScale: 1.25, highContrast: true }, root);
    expect(root.style.getPropertyValue('--ui-scale')).toBe('1.25');
    expect(root.style.getPropertyValue('--professional-panel-width')).toBe('420px');
    expect(root.dataset.highContrast).toBe('true');
  });

  it('rejects invalid workspace application', () => expect(() => applyWorkspace({ ...defaultWorkspace, uiScale: 9 })).toThrow());

  it('saves and loads workspace arrays', () => {
    let saved = '';
    saveWorkspaces([defaultWorkspace], { setItem: (_key, value) => (saved = value) });
    expect(loadWorkspaces({ getItem: () => saved })).toEqual([defaultWorkspace]);
  });

  it.each(['', '{', 'false', '{}'])('recovers from invalid workspace storage %s', (stored) => {
    expect(loadWorkspaces({ getItem: () => stored })).toEqual([]);
  });

  it('saves valid shortcuts', () => {
    let saved = '';
    saveShortcuts(defaultShortcuts, { setItem: (_key, value) => (saved = value) });
    expect(JSON.parse(saved)).toEqual(defaultShortcuts);
  });

  it('rejects shortcut conflicts before saving', () => {
    const bindings: ShortcutBinding[] = [{ action: 'A', keys: 'X' }, { action: 'B', keys: 'x' }];
    expect(() => saveShortcuts(bindings, { setItem: () => undefined })).toThrow(/both use/);
  });

  it('loads defaults when shortcut storage is invalid', () => {
    expect(loadShortcuts({ getItem: () => '{' })).toEqual(defaultShortcuts);
  });

  it('round trips exported settings', () => {
    const json = exportSettings([defaultWorkspace], defaultShortcuts);
    expect(importSettings(json)).toEqual({ layouts: [defaultWorkspace], shortcuts: defaultShortcuts });
  });

  it.each([
    '{}',
    '{"schemaVersion":2,"layouts":[],"shortcuts":[]}',
    '{"schemaVersion":1,"layouts":"bad","shortcuts":[]}',
    '{"schemaVersion":1,"layouts":[],"shortcuts":"bad"}'
  ])('rejects invalid settings import %s', (json) => expect(() => importSettings(json)).toThrow());
});
