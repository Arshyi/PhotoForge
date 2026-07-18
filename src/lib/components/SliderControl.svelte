<script lang="ts">
  export let label: string;
  export let value: number;
  export let min: number;
  export let max: number;
  export let step: number;
  export let defaultValue: number;
  export let format: (value: number) => string = (input) => input.toFixed(2);
  export let onchange: (value: number) => void;

  function update(event: Event) {
    onchange(Number((event.currentTarget as HTMLInputElement).value));
  }
</script>

<div class="control">
  <div class="control-heading">
    <label for={label}>{label}</label>
    <button
      type="button"
      class:changed={Math.abs(value - defaultValue) > Number.EPSILON}
      title={`Reset ${label}`}
      aria-label={`Reset ${label}`}
      on:click={() => onchange(defaultValue)}
    >
      {format(value)}
    </button>
  </div>
  <input
    id={label}
    type="range"
    {min}
    {max}
    {step}
    {value}
    aria-valuetext={format(value)}
    on:input={update}
  />
</div>

<style>
  .control {
    display: grid;
    gap: 8px;
  }

  .control-heading {
    display: flex;
    justify-content: space-between;
    align-items: center;
  }

  label {
    font-size: 0.78rem;
    color: var(--ink-soft);
  }

  button {
    min-width: 45px;
    border: 0;
    border-radius: 5px;
    padding: 3px 5px;
    background: transparent;
    color: var(--ink-faint);
    font: 600 0.68rem/1 var(--font-mono);
    cursor: pointer;
  }

  button:hover,
  button.changed {
    color: var(--accent-bright);
    background: var(--accent-dim);
  }

  input {
    width: 100%;
    height: 4px;
    appearance: none;
    border-radius: 999px;
    background: var(--line-strong);
    cursor: pointer;
  }

  input::-webkit-slider-thumb {
    width: 14px;
    height: 14px;
    appearance: none;
    border: 2px solid var(--surface);
    border-radius: 50%;
    background: var(--accent);
    box-shadow: 0 0 0 1px var(--accent);
  }

  input:focus-visible {
    outline: 2px solid var(--accent-bright);
    outline-offset: 5px;
  }
</style>
