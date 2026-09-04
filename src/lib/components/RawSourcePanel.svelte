<script lang="ts">
  import type { RawDevelopmentParameters, RawLayerSource } from '../types/editor';
  import SliderControl from './SliderControl.svelte';
  export let source: RawLayerSource;
  export let disabled = false;
  export let onapply: (parameters: RawDevelopmentParameters) => Promise<void>;
  let draft: RawDevelopmentParameters;
  let sourceKey = '';
  $: if (JSON.stringify(source) !== sourceKey) {
    sourceKey = JSON.stringify(source);
    draft = structuredClone(source.parameters);
  }
  const tones = ['contrast', 'highlights', 'shadows', 'whites', 'blacks'] as const;
</script>

<details class="raw-source">
  <summary>Develop selected RAW source</summary>
  <p>{source.reference.filename} · full-resolution float</p>
  <p>Re-reads the hash-verified source. Applying creates one undoable edit.</p>
  {#if draft}
    <fieldset {disabled}>
      <SliderControl label="Source exposure" value={draft.exposureEv} min={-8} max={8} step={0.01} defaultValue={0} format={(v) => `${v.toFixed(2)} EV`} onchange={(v) => { draft.exposureEv = v; }} />
      {#each tones as tone}
        <SliderControl label={`Source ${tone}`} value={draft[tone]} min={-1} max={1} step={0.01} defaultValue={0} format={(v) => `${Math.round(v * 100)}%`} onchange={(v) => { draft[tone] = v; }} />
      {/each}
      <button on:click={() => { draft.whiteBalance = { mode: 'asShot', multipliers: [1, 1, 1] }; }}>Use camera white balance</button>
      <button on:click={() => { draft.whiteBalance = { mode: 'auto' }; }}>Estimate gray-world white balance</button>
      <p>White balance: {draft.whiteBalance.mode}</p>
      <button on:click={() => onapply(structuredClone(draft))}>Apply source development</button>
    </fieldset>
  {/if}
</details>

<style>
  .raw-source { padding: 12px; border-bottom: 1px solid var(--border, #444); }
  p { font-size: 12px; overflow-wrap: anywhere; }
  fieldset { padding: 0; border: 0; min-width: 0; }
  button { margin: 4px; }
</style>
