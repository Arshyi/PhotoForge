<script lang="ts">
  import { describeCondition } from '../automation/macros';
  import type { Condition, LayerSelector } from '../operations/types';
  import SelectorField from './SelectorField.svelte';

  /**
   * The one decision a step can carry: run only if a question about the document, asked
   * when the step is reached, is answered yes. There are five questions and "not", and
   * nothing that computes.
   */
  export let value: Condition | null = null;
  export let id = 'condition';
  export let onchange: (value: Condition | null) => void = () => {};

  type Simple = Exclude<Condition, { kind: 'not' }>;
  const LAYER_KINDS = ['pixel', 'group', 'adjustment', 'shape', 'text', 'smart_object'] as const;

  const KINDS: { kind: Simple['kind']; label: string }[] = [
    { kind: 'layer_exists', label: 'a layer exists' },
    { kind: 'layer_count_at_least', label: 'the document has at least this many layers' },
    { kind: 'active_layer_kind', label: 'the selected layer is of a kind' },
    { kind: 'precision', label: 'the document has a precision' }
  ];

  $: negated = value?.kind === 'not';
  $: inner = (value?.kind === 'not' ? value.condition : value) as Condition | null;
  // The editor writes one `not` around one question. A deeper condition (from a file) is
  // shown in words and can be removed, but not half-edited into something else.
  $: editable = inner === null || inner.kind !== 'not';
  $: simple = editable ? (inner as Simple | null) : null;

  function wrap(condition: Simple | null, not: boolean): Condition | null {
    if (!condition) return null;
    return not ? { kind: 'not', condition } : condition;
  }

  function fresh(kind: Simple['kind'] | ''): Simple | null {
    switch (kind) {
      case 'layer_exists':
        return { kind, selector: { type: 'active' } };
      case 'layer_count_at_least':
        return { kind, count: 1 };
      case 'active_layer_kind':
        return { kind, layerKind: 'pixel' };
      case 'precision':
        return { kind, precision: 'linear_srgb_f32' };
      default:
        return null;
    }
  }

  function chooseKind(kind: string) {
    onchange(wrap(fresh(kind as Simple['kind'] | ''), negated));
  }

  function count(event: Event) {
    const number = Number((event.currentTarget as HTMLInputElement).value);
    if (Number.isInteger(number) && number >= 1 && number <= 512) onchange(wrap({ kind: 'layer_count_at_least', count: number }, negated));
  }
</script>

<fieldset class="condition">
  <legend>Run this step</legend>
  {#if !editable}
    <p class="fine">Only if {describeCondition(value as Condition)}.</p>
    <button type="button" on:click={() => onchange(null)}>Remove condition</button>
  {:else}
    <label for={`${id}-kind`} class="sr-only">Condition</label>
    <select id={`${id}-kind`} value={simple?.kind ?? ''} on:change={(event) => chooseKind(event.currentTarget.value)}>
      <option value="">always</option>
      {#each KINDS as option}
        <option value={option.kind}>only if {option.label}</option>
      {/each}
    </select>
    {#if simple?.kind === 'layer_exists'}
      <SelectorField
        value={simple.selector}
        label="Layer in the condition"
        id={`${id}-selector`}
        onchange={(selector: LayerSelector) => onchange(wrap({ kind: 'layer_exists', selector }, negated))}
      />
    {:else if simple?.kind === 'layer_count_at_least'}
      <label for={`${id}-count`}>Layers</label>
      <input id={`${id}-count`} type="number" min="1" max="512" step="1" value={simple.count} on:change={count} />
    {:else if simple?.kind === 'active_layer_kind'}
      <label for={`${id}-layer-kind`}>Kind</label>
      <select
        id={`${id}-layer-kind`}
        value={simple.layerKind}
        on:change={(event) => onchange(wrap({ kind: 'active_layer_kind', layerKind: event.currentTarget.value as (typeof LAYER_KINDS)[number] }, negated))}
      >
        {#each LAYER_KINDS as layerKind}
          <option value={layerKind}>{layerKind.replace('_', ' ')}</option>
        {/each}
      </select>
    {:else if simple?.kind === 'precision'}
      <label for={`${id}-precision`}>Precision</label>
      <select
        id={`${id}-precision`}
        value={simple.precision}
        on:change={(event) => onchange(wrap({ kind: 'precision', precision: event.currentTarget.value as 'linear_srgb_f32' | 'legacy_srgb8' }, negated))}
      >
        <option value="linear_srgb_f32">linear float</option>
        <option value="legacy_srgb8">legacy 8-bit</option>
      </select>
    {/if}
    {#if simple}
      <label class="check" for={`${id}-not`}>
        <input id={`${id}-not`} type="checkbox" checked={negated} on:change={(event) => onchange(wrap(simple, event.currentTarget.checked))} />
        <span>Reverse it: run the step only if this is not true</span>
      </label>
      <p class="fine">Asked when the step is reached, after the steps before it have run. If it is not true, the step is skipped.</p>
    {/if}
  {/if}
</fieldset>

<style>
  .condition { display: grid; gap: 6px; margin: 0; padding: 8px 10px; border: 1px solid var(--line); border-radius: 8px; font-size: 0.64rem; }
  legend { padding: 0 4px; color: var(--ink-soft); }
  label { color: var(--ink-soft); }
  .check { display: flex; gap: 6px; align-items: center; }
  select, input[type='number'] { padding: 5px 7px; border: 1px solid var(--line-strong); border-radius: 6px; color: var(--ink); background: var(--surface-raised); font: inherit; }
  button { justify-self: start; padding: 4px 8px; border: 1px solid var(--line-strong); border-radius: 6px; color: var(--ink-soft); background: var(--surface-raised); font-size: 0.6rem; cursor: pointer; }
  .fine { margin: 0; color: var(--ink-faint); font-size: 0.58rem; line-height: 1.4; }
  .sr-only { position: absolute; width: 1px; height: 1px; overflow: hidden; clip: rect(0 0 0 0); white-space: nowrap; }
</style>
