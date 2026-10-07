<script lang="ts">
  import { onMount } from 'svelte';
  import ParamForm from './ParamForm.svelte';
  import { describeLocality, firstProblem, initialValues } from '../plugins/params';
  import type { FilterDecl, FilterRun, PluginSummary } from '../plugins/types';

  /** The plugins whose filters can be run right now. */
  export let plugins: PluginSummary[] = [];
  /** The selected layer, for the destructive choice. */
  export let layer: { name: string; pixel: boolean; locked: boolean } | null = null;
  /** An adjustment layer needs a linear float document. */
  export let linearDocument = true;
  export let remembered: (plugin: string, key: string) => Promise<Record<string, number>> = async () => ({});
  export let onrun: (run: FilterRun) => void = () => {};
  export let oncancel: () => void = () => {};

  interface Choice {
    key: string;
    plugin: PluginSummary;
    filter: FilterDecl;
  }

  let dialogElement: HTMLDialogElement;
  let choiceKey = '';
  let values: Record<string, number> = {};
  let loaded = '';

  $: choices = plugins.flatMap((plugin) =>
    (plugin.manifest?.filters ?? []).map((filter) => ({ key: `${plugin.id}\u0000${filter.id}`, plugin, filter }) as Choice)
  );
  $: if (!choiceKey && choices[0]) choiceKey = choices[0].key;
  $: chosen = choices.find((choice) => choice.key === choiceKey) ?? null;
  $: if (chosen && loaded !== chosen.key) void load(chosen);
  $: problem = chosen ? firstProblem(chosen.filter.parameters, values) : null;
  $: pixelsReason = !layer
    ? 'Select a layer first.'
    : !layer.pixel
      ? `${layer.name} is not a pixel layer.`
      : layer.locked
        ? `${layer.name} is locked.`
        : '';

  async function load(choice: Choice) {
    loaded = choice.key;
    let earlier: Record<string, number> = {};
    try {
      earlier = await remembered(choice.plugin.id, `filter:${choice.filter.id}`);
    } catch {
      earlier = {};
    }
    // A choice made while this was loading wins.
    if (loaded === choice.key) values = initialValues(choice.filter.parameters, earlier);
  }

  function run(mode: FilterRun['mode']) {
    if (!chosen || problem) return;
    onrun({ mode, plugin: chosen.plugin.id, filter: chosen.filter.id, values });
  }

  function keys(event: KeyboardEvent) {
    if (event.key === 'Escape') {
      event.preventDefault();
      event.stopPropagation();
      oncancel();
    }
  }

  onMount(() => {
    dialogElement?.querySelector<HTMLElement>('select, button')?.focus();
  });
</script>

<div class="modal-backdrop" role="presentation">
  <dialog open bind:this={dialogElement} class="modal filter-modal" aria-labelledby="filter-title" on:keydown={keys}>
    <div class="modal-heading">
      <div>
        <span>Plugin</span>
        <h1 id="filter-title">Run a plugin filter</h1>
      </div>
      <button type="button" aria-label="Cancel plugin filter" on:click={oncancel}>×</button>
    </div>
    <div class="body">
      {#if !choices.length}
        <p class="none" role="status">No installed plugin has a filter that can run. Open Plugins to install or turn one on.</p>
      {:else}
        <label for="filter-choice">Filter</label>
        <select id="filter-choice" bind:value={choiceKey}>
          {#each choices as choice}
            <option value={choice.key}>{choice.plugin.manifest?.name ?? choice.plugin.id} — {choice.filter.title}</option>
          {/each}
        </select>
        {#if chosen}
          {#if chosen.filter.description}<p class="fine">{chosen.filter.description}</p>{/if}
          <p class="fine" data-testid="locality">{describeLocality(chosen.filter.locality)}.
            {#if chosen.filter.locality.kind === 'global'}Very large images may be refused, because the plugin needs room for all of it at once.{/if}
          </p>
          <ParamForm params={chosen.filter.parameters ?? []} {values} idPrefix="filter-param" onchange={(next) => (values = next)} />
          {#if problem}<p class="problem" role="alert">{problem}</p>{/if}
        {/if}
      {/if}
      <div class="modal-actions">
        <button type="button" on:click={oncancel}>Cancel</button>
        <button
          type="button"
          disabled={!chosen || Boolean(pixelsReason) || Boolean(problem)}
          title={pixelsReason || `Bake the filter into ${layer?.name ?? 'the layer'}'s pixels`}
          on:click={() => run('pixels')}
        >Apply to {layer?.name ?? 'layer'}</button>
        <button
          type="button"
          class="primary"
          disabled={!chosen || Boolean(problem) || !linearDocument}
          title={linearDocument ? 'Add a layer that keeps the filter live' : 'Convert the document to linear float first'}
          on:click={() => run('adjustment')}
        >Add as adjustment layer</button>
      </div>
      {#if chosen && !linearDocument}
        <p class="fine" role="status">Plugin adjustment layers need a linear float document. Convert it in Color and precision.</p>
      {/if}
      {#if chosen && pixelsReason}<p class="fine">{pixelsReason}</p>{/if}
    </div>
  </dialog>
</div>

<style>
  .filter-modal { width: min(480px, 100%); }
  .body { display: grid; gap: 10px; padding: 14px 21px 21px; font-size: 0.66rem; }
  label { color: var(--ink-soft); }
  select { padding: 6px 8px; border: 1px solid var(--line-strong); border-radius: 6px; color: var(--ink); background: var(--surface-raised); }
  .fine { margin: 0; color: var(--ink-faint); font-size: 0.6rem; line-height: 1.5; }
  .problem { margin: 0; padding: 8px 10px; border-radius: 7px; background: rgba(224, 121, 90, 0.16); }
  .none { margin: 0; padding: 10px; border-radius: 7px; background: rgba(120, 130, 120, 0.16); line-height: 1.5; }
  button:disabled { opacity: 0.5; }
</style>
