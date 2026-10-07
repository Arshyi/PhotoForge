<script lang="ts">
  import { accepts, stepOf } from '../plugins/params';
  import type { ParamDecl } from '../plugins/types';

  /**
   * The form a plugin's declared parameters become. A plugin cannot draw anything of
   * its own here: every control is one of four kinds the manifest can name, and each
   * only offers values the declaration accepts. The backend checks them again.
   */
  export let params: ParamDecl[] = [];
  export let values: Record<string, number> = {};
  export let idPrefix = 'param';
  export let onchange: (values: Record<string, number>) => void = () => {};

  function set(id: string, value: number) {
    values = { ...values, [id]: value };
    onchange(values);
  }

  function fromInput(param: ParamDecl, event: Event) {
    const input = event.currentTarget as HTMLInputElement;
    const value = Number(input.value);
    // Text that is not a number, or is out of range, is left in the box and not
    // taken: the value in force does not change until the entry is one the plugin accepts.
    if (input.value.trim() !== '' && accepts(param, value)) set(param.id, value);
  }
</script>

<div class="params">
  {#each params as param (param.id)}
    {@const inputId = `${idPrefix}-${param.id}`}
    <div class="param">
      {#if param.type === 'bool'}
        <label class="check" for={inputId}>
          <input
            id={inputId}
            type="checkbox"
            checked={values[param.id] === 1}
            on:change={(event) => set(param.id, event.currentTarget.checked ? 1 : 0)}
          />
          <span>{param.title}</span>
        </label>
      {:else if param.type === 'choice'}
        <label for={inputId}>{param.title}</label>
        <select
          id={inputId}
          value={String(values[param.id])}
          on:change={(event) => set(param.id, Number(event.currentTarget.value))}
        >
          {#each param.options as option, index}
            <option value={String(index)}>{option}</option>
          {/each}
        </select>
      {:else}
        <label for={inputId}>{param.title}</label>
        <div class="range">
          <input
            type="range"
            aria-label={`${param.title} slider`}
            min={param.min}
            max={param.max}
            step={stepOf(param)}
            value={values[param.id]}
            on:input={(event) => set(param.id, Number(event.currentTarget.value))}
          />
          <input
            id={inputId}
            type="number"
            inputmode="decimal"
            min={param.min}
            max={param.max}
            step={stepOf(param)}
            value={values[param.id]}
            on:change={(event) => fromInput(param, event)}
          />
        </div>
      {/if}
      {#if param.description}<small>{param.description}</small>{/if}
    </div>
  {/each}
</div>

<style>
  .params { display: grid; gap: 10px; }
  .param { display: grid; gap: 4px; font-size: 0.66rem; }
  .param label { color: var(--ink-soft); }
  .check { display: flex; gap: 8px; align-items: center; cursor: pointer; }
  .range { display: grid; grid-template-columns: 1fr 84px; gap: 8px; align-items: center; }
  input[type='number'], select { padding: 5px 7px; border: 1px solid var(--line-strong); border-radius: 6px; color: var(--ink); background: var(--surface-raised); font-size: 0.64rem; }
  input:focus-visible, select:focus-visible { outline: 2px solid var(--accent); outline-offset: 1px; }
  small { color: var(--ink-faint); font-size: 0.58rem; }
</style>
