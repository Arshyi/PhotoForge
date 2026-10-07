<script lang="ts">
  import { factValue } from '../plugins/facts';
  import type { LayerDocument } from '../layers/types';
  import type { PluginSummary, RequirementStatus } from '../plugins/types';

  /** Every installed plugin. */
  export let plugins: PluginSummary[] = [];
  export let document: LayerDocument | null = null;
  /** What the open document needs that cannot be had right now. */
  export let unavailable: RequirementStatus[] = [];
  export let busy = false;
  export let onfilter: () => void = () => {};
  export let onmanage: () => void = () => {};
  export let oncommand: (plugin: PluginSummary, command: string) => void = () => {};

  $: live = plugins.filter((plugin) => plugin.availability.kind === 'available' && plugin.manifest);
  $: filterCount = live
    .filter((plugin) => plugin.granted.includes('filter.pixels'))
    .reduce((count, plugin) => count + (plugin.manifest?.filters?.length ?? 0), 0);
  $: panels = live
    .filter((plugin) => plugin.granted.includes('ui.panel'))
    .flatMap((plugin) => (plugin.manifest?.panels ?? []).map((panel) => ({ plugin, panel })));
  $: tools = live
    .filter((plugin) => plugin.granted.includes('ui.tool') && plugin.granted.includes('document.operations'))
    .flatMap((plugin) =>
      (plugin.manifest?.tools ?? []).map((tool) => ({
        plugin,
        tool,
        command: plugin.manifest?.commands?.find((command) => command.id === tool.command)
      }))
    );

  function canRun(plugin: PluginSummary): boolean {
    return !busy && Boolean(document) && plugin.granted.includes('document.operations');
  }
</script>

<section class="tool-section plugins-section" aria-labelledby="plugins-heading">
  <h2 id="plugins-heading"><span>⧉</span> Plugins</h2>

  {#if unavailable.length}
    <div class="needs" role="alert" data-testid="needs-plugins">
      <strong>This document needs {unavailable.length === 1 ? 'a plugin' : 'plugins'} that cannot be used:</strong>
      <ul>
        {#each unavailable as status (status.plugin + status.sha256)}
          <li>{status.message}</li>
        {/each}
      </ul>
      <p>The layers that need {unavailable.length === 1 ? 'it' : 'them'} are left out of the preview and refuse to export. Nothing has been changed in the document.</p>
    </div>
  {/if}

  <div class="buttons">
    <button type="button" disabled={busy || !filterCount || !document} on:click={onfilter}>
      Run a plugin filter… <small>{filterCount}</small>
    </button>
    <button type="button" on:click={onmanage}>Manage plugins…</button>
  </div>

  {#if tools.length}
    <div class="tools" role="group" aria-label="Plugin tools">
      {#each tools as { plugin, tool, command } (plugin.id + tool.id)}
        <button type="button" disabled={!canRun(plugin)} title={tool.description || command?.description} on:click={() => oncommand(plugin, tool.command)}>
          {tool.title}
        </button>
      {/each}
    </div>
  {/if}

  {#each panels as { plugin, panel } (plugin.id + panel.id)}
    <section class="plugin-panel" aria-label={`${panel.title}, from ${plugin.manifest?.name}`}>
      <h3>{panel.title}</h3>
      <dl>
        {#each panel.rows as row}
          {#if row.kind === 'text'}
            <p class="line">{row.text}</p>
          {:else if row.kind === 'fact'}
            <div class="fact">
              <dt>{row.label}</dt>
              <dd>{plugin.granted.includes('document.read') ? factValue(document, row.fact) : 'Not allowed'}</dd>
            </div>
          {:else}
            <button type="button" class="line" disabled={!canRun(plugin)} on:click={() => oncommand(plugin, row.command)}>
              {row.label}
            </button>
          {/if}
        {/each}
      </dl>
      <small>Shown by {plugin.manifest?.name}. A plugin can show only the facts it was allowed to read.</small>
    </section>
  {/each}

  {#if !plugins.length}
    <p class="empty">No plugins installed.</p>
  {/if}
</section>

<style>
  .buttons, .tools { display: flex; flex-wrap: wrap; gap: 6px; margin-bottom: 8px; }
  button { padding: 6px 10px; border: 1px solid var(--line-strong); border-radius: 7px; color: var(--ink-soft); background: var(--surface-raised); font-size: 0.62rem; font-weight: 700; cursor: pointer; }
  button:disabled { opacity: 0.5; cursor: default; }
  button:focus-visible { outline: 2px solid var(--accent); outline-offset: 2px; }
  button small { margin-left: 4px; color: var(--ink-faint); }
  .needs { margin-bottom: 8px; padding: 9px 11px; border-radius: 8px; background: rgba(224, 121, 90, 0.16); font-size: 0.62rem; line-height: 1.5; }
  .needs ul { margin: 4px 0; padding-left: 16px; }
  .needs p { margin: 0; color: var(--ink-soft); }
  .plugin-panel { margin-top: 8px; padding: 8px 10px; border: 1px solid var(--line-strong); border-radius: 8px; }
  .plugin-panel h3 { margin: 0 0 6px; font-size: 0.68rem; }
  .plugin-panel dl { display: grid; gap: 4px; margin: 0; font-size: 0.62rem; }
  .fact { display: flex; justify-content: space-between; gap: 10px; }
  .fact dt { color: var(--ink-faint); }
  .fact dd { margin: 0; text-align: right; font-variant-numeric: tabular-nums; }
  .line { margin: 0; }
  .plugin-panel small, .empty { color: var(--ink-faint); font-size: 0.56rem; }
</style>
