<script lang="ts">
  import type { LayerSelector } from '../operations/types';

  /** How a step or a condition names a layer. Six ways, and nothing that computes. */
  export let value: LayerSelector = { type: 'active' };
  export let label = 'Layer';
  export let id = 'selector';
  export let onchange: (value: LayerSelector) => void = () => {};

  const TYPES: { type: LayerSelector['type']; label: string }[] = [
    { type: 'active', label: 'The selected layer' },
    { type: 'name', label: 'The layer named…' },
    { type: 'id', label: 'The layer with identifier…' },
    { type: 'last_created', label: 'The layer the last step made' },
    { type: 'top', label: 'The top layer' },
    { type: 'bottom', label: 'The bottom layer' }
  ];

  function chosen(type: LayerSelector['type']) {
    if (type === 'name') onchange({ type, name: '' });
    else if (type === 'id') onchange({ type, id: '' });
    else onchange({ type } as LayerSelector);
  }
</script>

<div class="selector">
  <select {id} aria-label={label} value={value.type} on:change={(event) => chosen(event.currentTarget.value as LayerSelector['type'])}>
    {#each TYPES as option}
      <option value={option.type}>{option.label}</option>
    {/each}
  </select>
  {#if value.type === 'name'}
    <input
      type="text"
      aria-label={`${label}: layer name`}
      maxlength="120"
      value={value.name}
      on:input={(event) => onchange({ type: 'name', name: event.currentTarget.value })}
    />
  {:else if value.type === 'id'}
    <input
      type="text"
      aria-label={`${label}: layer identifier`}
      maxlength="64"
      value={value.id}
      on:input={(event) => onchange({ type: 'id', id: event.currentTarget.value })}
    />
    <span class="fine">An identifier names a layer of this document only.</span>
  {/if}
</div>

<style>
  .selector { display: flex; flex-wrap: wrap; gap: 6px; align-items: center; font-size: 0.64rem; }
  select, input[type='text'] { padding: 5px 7px; border: 1px solid var(--line-strong); border-radius: 6px; color: var(--ink); background: var(--surface-raised); font: inherit; }
  .fine { color: var(--ink-faint); font-size: 0.58rem; }
</style>
