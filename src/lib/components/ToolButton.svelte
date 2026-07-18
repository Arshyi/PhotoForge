<script lang="ts">
  export let label: string;
  export let icon: string;
  export let disabled = false;
  export let primary = false;
  export let active = false;
  export let title = label;
  export let onclick: () => void;
</script>

<button
  type="button"
  class:primary
  class:active
  {disabled}
  {title}
  aria-label={label}
  on:click={() => !disabled && onclick()}
>
  <span class="icon" aria-hidden="true">{icon}</span>
  <span class="label">{label}</span>
</button>

<style>
  button {
    min-height: 38px;
    display: inline-flex;
    align-items: center;
    gap: 8px;
    border: 1px solid transparent;
    border-radius: 9px;
    padding: 0 11px;
    color: var(--ink-soft);
    background: transparent;
    font: inherit;
    font-size: 0.78rem;
    font-weight: 650;
    cursor: pointer;
    transition: 150ms ease;
  }

  button:hover:not(:disabled),
  button.active {
    color: var(--ink);
    border-color: var(--line-strong);
    background: var(--surface-raised);
  }

  button.primary {
    color: #142018;
    background: var(--accent);
    border-color: color-mix(in srgb, var(--accent) 78%, white);
  }

  button.primary:hover:not(:disabled) {
    color: #0d1710;
    background: var(--accent-bright);
  }

  button:focus-visible {
    outline: 2px solid var(--accent-bright);
    outline-offset: 2px;
  }

  button:disabled {
    opacity: 0.35;
    cursor: not-allowed;
  }

  .icon {
    width: 17px;
    height: 17px;
    display: grid;
    place-items: center;
    font-size: 1rem;
    line-height: 1;
  }

  @media (max-width: 840px) {
    .label {
      display: none;
    }

    button {
      padding: 0 10px;
    }
  }
</style>
