<script lang="ts">
  import { invoke } from '@tauri-apps/api/core';
  import type { ShortcutBinding, WorkspaceLayout } from '../types/editor';
  import { errorMessage } from '../utils/format';
  import {
    applyWorkspace,
    defaultShortcuts,
    defaultWorkspace,
    exportSettings,
    importSettings,
    loadShortcuts,
    loadWorkspaces,
    saveShortcuts,
    saveWorkspaces,
    shortcutConflicts
  } from '../utils/workspace';

  let layouts = loadWorkspaces();
  let shortcuts = loadShortcuts();
  let layout: WorkspaceLayout = structuredClone(layouts[0] ?? defaultWorkspace);
  let message = '';
  let error = '';

  $: conflicts = shortcutConflicts(shortcuts);

  async function saveLayout() {
    error = ''; message = '';
    try {
      layout = await invoke<WorkspaceLayout>('validate_workspace_layout', { layout });
      layouts = [structuredClone(layout), ...layouts.filter((value) => value.name !== layout.name)].slice(0, 20);
      saveWorkspaces(layouts);
      applyWorkspace(layout);
      message = 'Workspace saved locally and applied.';
    } catch (caught) { error = errorMessage(caught); }
  }

  function loadLayout(value: WorkspaceLayout) {
    layout = structuredClone(value);
    applyWorkspace(layout);
    message = `${layout.name} restored.`;
  }

  async function saveBindings() {
    error = ''; message = '';
    try {
      shortcuts = await invoke<ShortcutBinding[]>('validate_shortcut_bindings', { bindings: shortcuts });
      saveShortcuts(shortcuts);
      message = 'Keyboard shortcuts saved locally.';
    } catch (caught) { error = errorMessage(caught); }
  }

  function resetBindings() { shortcuts = structuredClone(defaultShortcuts); }

  function downloadSettings() {
    const url = URL.createObjectURL(new Blob([exportSettings(layouts, shortcuts)], { type: 'application/json' }));
    const anchor = document.createElement('a'); anchor.href = url; anchor.download = 'photoforge-workspace-settings.json'; anchor.click(); URL.revokeObjectURL(url);
  }

  async function importFile(event: Event) {
    const file = (event.currentTarget as HTMLInputElement).files?.[0];
    if (!file) return;
    try {
      const imported = importSettings(await file.text());
      layouts = imported.layouts; shortcuts = imported.shortcuts;
      saveWorkspaces(layouts); saveShortcuts(shortcuts);
      if (layouts[0]) loadLayout(layouts[0]);
      message = 'Workspace layouts and shortcuts imported.';
    } catch (caught) { error = errorMessage(caught); }
  }
</script>

<section class="workspace-settings">
  <div class="settings-intro"><span>Professional workspace</span><h2>Layouts and keyboard shortcuts</h2><p>Save panel dimensions, collapsed sections, high contrast, scale, and configurable shortcuts locally.</p></div>
  <div class="layout-editor">
    <h3>Workspace layout</h3>
    <label>Name<input bind:value={layout.name} maxlength="120" /></label>
    <div class="layout-grid"><label>Left panel<input type="number" min="180" max="800" bind:value={layout.leftPanelWidth} /></label><label>Right panel<input type="number" min="240" max="900" bind:value={layout.rightPanelWidth} /></label><label>UI scale<input type="number" min="0.75" max="2" step="0.05" bind:value={layout.uiScale} /></label></div>
    <label>Collapsed sections<input value={layout.collapsedSections.join(', ')} on:change={(event) => (layout.collapsedSections = event.currentTarget.value.split(',').map((value) => value.trim()).filter(Boolean))} placeholder="Curves, Metadata" /></label>
    <label class="checkbox"><input type="checkbox" bind:checked={layout.highContrast} /> High contrast compatibility mode</label>
    <button class="primary" on:click={saveLayout}>Save current workspace</button>
  </div>
  <div class="recent-layouts"><h3>Recent workspaces</h3>{#each layouts as saved}<button on:click={() => loadLayout(saved)}><span><strong>{saved.name}</strong><small>{saved.rightPanelWidth}px panel · {Math.round(saved.uiScale * 100)}% scale</small></span><b>Restore</b></button>{:else}<p>No saved layouts yet.</p>{/each}</div>
  <div class="shortcut-editor"><div class="shortcut-heading"><h3>Shortcut editor</h3><button on:click={resetBindings}>Reset defaults</button></div>{#each shortcuts as binding, index}<label><span>{binding.action}</span><input aria-label={`${binding.action} shortcut`} bind:value={shortcuts[index].keys} /></label>{/each}{#if conflicts.length}<ul class="conflicts">{#each conflicts as conflict}<li>{conflict}</li>{/each}</ul>{/if}<button class="primary" disabled={conflicts.length > 0} on:click={saveBindings}>Save shortcuts</button></div>
  <div class="settings-transfer"><button on:click={downloadSettings}>Export layouts & shortcuts</button><label>Import settings<input type="file" accept="application/json,.json" on:change={importFile} /></label></div>
  {#if message}<p class="settings-message" role="status">{message}</p>{/if}{#if error}<p class="settings-error" role="alert">{error}</p>{/if}
</section>

<style>
  .workspace-settings { display: grid; }
  .layout-editor, .shortcut-editor, .recent-layouts { display: grid; gap: 10px; padding: 16px 21px; border-bottom: 1px solid var(--line); }
  h3 { margin: 0; color: var(--accent); font: 700 .57rem/1 var(--font-mono); letter-spacing: .08em; text-transform: uppercase; }
  label { display: grid; gap: 5px; color: var(--ink-soft); font-size: .62rem; font-weight: 700; }
  input { min-width: 0; padding: 8px; border: 1px solid var(--line-strong); border-radius: 6px; color: var(--ink); background: var(--surface-soft); }
  input:focus-visible, button:focus-visible { outline: 2px solid var(--accent); outline-offset: 2px; }
  .layout-grid { display: grid; grid-template-columns: repeat(3, 1fr); gap: 7px; }
  .checkbox { display: flex; align-items: center; gap: 7px; }
  button, .settings-transfer label { padding: 8px 10px; border: 1px solid var(--line-strong); border-radius: 7px; color: var(--ink-soft); background: var(--surface-raised); font-size: .6rem; font-weight: 700; cursor: pointer; }
  button.primary { color: #152012; border-color: var(--accent); background: var(--accent); }
  button:disabled { opacity: .45; }
  .recent-layouts button { display: flex; justify-content: space-between; text-align: left; }
  .recent-layouts button span { display: grid; gap: 3px; }
  .recent-layouts small, .recent-layouts p { color: var(--ink-faint); font-size: .55rem; }
  .shortcut-heading { display: flex; justify-content: space-between; align-items: center; }
  .shortcut-editor label { grid-template-columns: 1fr 150px; align-items: center; }
  .conflicts { margin: 0; padding: 9px 9px 9px 28px; color: #e19a91; background: rgba(117,74,68,.14); font-size: .58rem; }
  .settings-transfer { display: flex; gap: 8px; padding: 16px 21px; }
  .settings-transfer label { position: relative; overflow: hidden; }
  .settings-transfer input { position: absolute; opacity: 0; inset: 0; cursor: pointer; }
</style>
