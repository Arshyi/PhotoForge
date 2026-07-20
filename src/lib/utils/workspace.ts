import type { ShortcutBinding, WorkspaceLayout } from '../types/editor';

export const WORKSPACE_STORAGE_KEY = 'photoforge.workspaces.v1';
export const SHORTCUT_STORAGE_KEY = 'photoforge.shortcuts.v1';

export const defaultShortcuts: ShortcutBinding[] = [
  { action: 'Open image', keys: 'Ctrl+O' },
  { action: 'Export image', keys: 'Ctrl+S' },
  { action: 'Undo', keys: 'Ctrl+Z' },
  { action: 'Redo', keys: 'Ctrl+Y' },
  { action: 'Compare', keys: 'C' },
  { action: 'Zoom in', keys: '+' },
  { action: 'Zoom out', keys: '-' },
  { action: 'Pixel inspector', keys: 'I' },
  { action: 'Crop', keys: 'R' },
  { action: 'Straighten', keys: 'T' }
];

export const defaultWorkspace: WorkspaceLayout = {
  schemaVersion: 1,
  name: 'Professional Editing',
  leftPanelWidth: 240,
  rightPanelWidth: 360,
  collapsedSections: [],
  activePanel: 'tools',
  highContrast: false,
  uiScale: 1
};

export function normalizeShortcut(keys: string): string {
  const compact = keys.toLocaleLowerCase().replace(/\s+/g, '');
  if (compact === '+' || compact === '-') return compact;
  return compact.split('+').filter(Boolean).join('+');
}

export function shortcutConflicts(bindings: ShortcutBinding[]): string[] {
  const seen = new Map<string, string>();
  const conflicts: string[] = [];
  for (const binding of bindings) {
    const key = normalizeShortcut(binding.keys);
    if (!key) {
      conflicts.push(`${binding.action} has no shortcut.`);
      continue;
    }
    const previous = seen.get(key);
    if (previous) conflicts.push(`${previous} and ${binding.action} both use ${binding.keys}.`);
    else seen.set(key, binding.action);
  }
  return conflicts;
}

export function loadShortcuts(storage: Pick<Storage, 'getItem'> = localStorage): ShortcutBinding[] {
  try {
    const value = JSON.parse(storage.getItem(SHORTCUT_STORAGE_KEY) ?? 'null');
    return Array.isArray(value) && shortcutConflicts(value).length === 0 ? value : structuredClone(defaultShortcuts);
  } catch {
    return structuredClone(defaultShortcuts);
  }
}

export function saveShortcuts(bindings: ShortcutBinding[], storage: Pick<Storage, 'setItem'> = localStorage) {
  const conflicts = shortcutConflicts(bindings);
  if (conflicts.length) throw new Error(conflicts.join(' '));
  storage.setItem(SHORTCUT_STORAGE_KEY, JSON.stringify(bindings));
}

export function loadWorkspaces(storage: Pick<Storage, 'getItem'> = localStorage): WorkspaceLayout[] {
  try {
    const value = JSON.parse(storage.getItem(WORKSPACE_STORAGE_KEY) ?? '[]');
    return Array.isArray(value) ? value.filter(validWorkspace).slice(0, 20) : [];
  } catch {
    return [];
  }
}

export function saveWorkspaces(layouts: WorkspaceLayout[], storage: Pick<Storage, 'setItem'> = localStorage) {
  storage.setItem(WORKSPACE_STORAGE_KEY, JSON.stringify(layouts.filter(validWorkspace).slice(0, 20)));
}

export function validWorkspace(layout: WorkspaceLayout): boolean {
  return layout?.schemaVersion === 1 && Boolean(layout.name?.trim()) && layout.leftPanelWidth >= 180 && layout.leftPanelWidth <= 800 && layout.rightPanelWidth >= 240 && layout.rightPanelWidth <= 900 && layout.uiScale >= 0.75 && layout.uiScale <= 2;
}

export function applyWorkspace(layout: WorkspaceLayout, root: HTMLElement = document.documentElement) {
  if (!validWorkspace(layout)) throw new Error('Workspace layout is invalid.');
  root.style.setProperty('--ui-scale', String(layout.uiScale));
  root.style.setProperty('--professional-panel-width', `${layout.rightPanelWidth}px`);
  root.dataset.highContrast = String(layout.highContrast);
}

export function exportSettings(layouts: WorkspaceLayout[], shortcuts: ShortcutBinding[]): string {
  return JSON.stringify({ schemaVersion: 1, layouts, shortcuts }, null, 2);
}

export function importSettings(json: string): { layouts: WorkspaceLayout[]; shortcuts: ShortcutBinding[] } {
  const value = JSON.parse(json) as { schemaVersion?: number; layouts?: WorkspaceLayout[]; shortcuts?: ShortcutBinding[] };
  if (value.schemaVersion !== 1 || !Array.isArray(value.layouts) || !Array.isArray(value.shortcuts)) throw new Error('Unsupported workspace settings file.');
  if (!value.layouts.every(validWorkspace)) throw new Error('Workspace settings contain an invalid layout.');
  const conflicts = shortcutConflicts(value.shortcuts);
  if (conflicts.length) throw new Error(conflicts.join(' '));
  return { layouts: value.layouts, shortcuts: value.shortcuts };
}
