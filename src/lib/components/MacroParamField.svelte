<script lang="ts">
  import { validateSelector } from '../automation/macros';
  import { blendModes } from '../layers/types';
  import type { LayerSelector, ParamSpec } from '../operations/types';
  import SelectorField from './SelectorField.svelte';

  /**
   * One parameter of one registered operation, drawn from the registry's own
   * description of it. A control only offers values of the kind the parameter has; the
   * checks in the macro module and, last, the backend decide whether the whole step is
   * acceptable.
   */
  export let spec: ParamSpec;
  export let value: unknown;
  export let idPrefix = 'param';
  export let onchange: (value: unknown) => void = () => {};

  $: id = `${idPrefix}-${spec.name}`;
  $: kind = spec.kind;

  let jsonText = '';
  let jsonError = '';
  let jsonFor: unknown = undefined;
  // Text the person is typing is theirs until it parses; the value in force changes only then.
  $: if (value !== jsonFor && (kind.kind === 'editOperation' || kind.kind === 'editOperations' || kind.kind === 'json')) {
    jsonFor = value;
    jsonText = JSON.stringify(value ?? null, null, 2);
    jsonError = '';
  }

  /**
   * The selector to draw. A half-made one — "the layer named…" with no name yet — is
   * drawn as itself, so choosing it does not snap the control back; whether it is
   * complete enough to use is the macro's problem list to say, not the control's.
   */
  function selectorOf(candidate: unknown): LayerSelector {
    if (validateSelector(candidate) === null) return candidate as LayerSelector;
    const record = candidate as { type?: unknown; name?: unknown; id?: unknown } | null;
    if (record?.type === 'name') return { type: 'name', name: typeof record.name === 'string' ? record.name : '' };
    if (record?.type === 'id') return { type: 'id', id: typeof record.id === 'string' ? record.id : '' };
    return { type: 'active' };
  }

  function fromJson(event: Event) {
    jsonText = (event.currentTarget as HTMLTextAreaElement).value;
    try {
      const parsed = JSON.parse(jsonText);
      jsonError = '';
      jsonFor = parsed;
      onchange(parsed);
    } catch {
      jsonError = 'That is not valid JSON, so the value has not changed.';
    }
  }

  function fromNumber(event: Event) {
    const text = (event.currentTarget as HTMLInputElement).value;
    if (text.trim() === '') return;
    const number = Number(text);
    if (Number.isFinite(number)) onchange(number);
  }

  $: list = Array.isArray(value) ? value : [];
</script>

<div class="field">
  {#if kind.kind === 'bool'}
    <label class="check" for={id}>
      <input {id} type="checkbox" checked={value === true} on:change={(event) => onchange(event.currentTarget.checked)} />
      <span>{spec.name}</span>
    </label>
  {:else}
    <label for={id}>{spec.name}{spec.required ? '' : ' (optional)'}</label>
    {#if kind.kind === 'selector' || kind.kind === 'optionalSelector'}
      <SelectorField value={selectorOf(value)} label={spec.name} {id} onchange={(next) => onchange(next)} />
    {:else if kind.kind === 'selectors'}
      <ul class="selectors" aria-label={spec.name}>
        {#each list as entry, index}
          <li>
            <SelectorField
              value={selectorOf(entry)}
              label={`${spec.name} ${index + 1}`}
              id={index === 0 ? id : `${id}-${index}`}
              onchange={(next) => onchange(list.map((existing, position) => (position === index ? next : existing)))}
            />
            <button
              type="button"
              aria-label={`Remove ${spec.name} ${index + 1}`}
              disabled={list.length <= 1}
              on:click={() => onchange(list.filter((_, position) => position !== index))}
            >Remove</button>
          </li>
        {/each}
      </ul>
      <button type="button" class="add" disabled={list.length >= 100} on:click={() => onchange([...list, { type: 'active' }])}>Add layer</button>
    {:else if kind.kind === 'text'}
      <input {id} type="text" maxlength={kind.maxChars} value={typeof value === 'string' ? value : ''} on:input={(event) => onchange(event.currentTarget.value)} />
    {:else if kind.kind === 'number' || kind.kind === 'integer'}
      <input
        {id}
        type="number"
        min={kind.min}
        max={kind.max}
        step={kind.kind === 'integer' ? 1 : 'any'}
        value={typeof value === 'number' ? value : ''}
        on:change={fromNumber}
      />
      <span class="fine">{kind.min} to {kind.max}</span>
    {:else if kind.kind === 'blendMode'}
      <select {id} value={String(value)} on:change={(event) => onchange(event.currentTarget.value)}>
        {#each blendModes as mode}
          <option value={mode.id}>{mode.label}</option>
        {/each}
      </select>
    {:else}
      <textarea {id} rows="5" spellcheck="false" aria-describedby={jsonError ? `${id}-error` : undefined} value={jsonText} on:input={fromJson}></textarea>
      {#if kind.kind !== 'json'}
        <span class="fine">Edits are written as JSON, in the form PhotoForge saves them. They are checked before the macro can run.</span>
      {/if}
      {#if jsonError}<span id={`${id}-error`} class="problem" role="alert">{jsonError}</span>{/if}
    {/if}
  {/if}
  {#if spec.description && kind.kind !== 'bool'}<span class="fine">{spec.description}</span>{/if}
</div>

<style>
  .field { display: grid; gap: 4px; font-size: 0.64rem; }
  label { color: var(--ink-soft); }
  .check { display: flex; gap: 6px; align-items: center; }
  select, input[type='text'], input[type='number'], textarea { padding: 5px 7px; border: 1px solid var(--line-strong); border-radius: 6px; color: var(--ink); background: var(--surface-raised); font: inherit; }
  textarea { font-family: var(--font-mono); font-size: 0.6rem; resize: vertical; }
  .selectors { display: grid; gap: 6px; margin: 0; padding: 0; list-style: none; }
  .selectors li { display: flex; gap: 6px; align-items: center; }
  .add, .selectors button { justify-self: start; padding: 4px 8px; border: 1px solid var(--line-strong); border-radius: 6px; color: var(--ink-soft); background: var(--surface-raised); font-size: 0.6rem; cursor: pointer; }
  .fine { color: var(--ink-faint); font-size: 0.58rem; line-height: 1.4; }
  .problem { padding: 5px 8px; border-radius: 6px; background: rgba(224, 121, 90, 0.16); }
  button:disabled { opacity: 0.5; }
</style>
