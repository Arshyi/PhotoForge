<script lang="ts">
  import type { Layer, LayerDocument, SmartLinkStatus } from '../layers/types';

  export let document: LayerDocument;
  export let layer: Layer;
  export let disabled = false;
  export let busy = false;
  export let linkStatus: SmartLinkStatus | null = null;
  export let onedit: () => void = () => {};
  export let onduplicateindependent: () => void = () => {};
  export let onchecklinks: () => void = () => {};
  /** False verifies original file bytes; true is explicit replacement consent. */
  export let onrelink: (acceptChanged: boolean) => void = () => {};
  export let onrasterize: () => void = () => {};

  $: sourceId = layer.content.type === 'smart_object' ? layer.content.sourceId : '';
  $: source = document.smartSources?.[sourceId];
  $: unavailable = disabled || busy || layer.locked;
</script>

<section class="smart-panel" aria-label="Smart object contents">
  <h3>Smart object</h3>
  {#if !source}
    <p role="alert">The editable source is missing. No replacement has been substituted.</p>
  {:else}
    <p>{source.width} × {source.height} source · {source.layers.length} top-level layers</p>
    <p class="hint">Duplicates share this source. Each instance keeps its own transform, opacity and mask.</p>
    <div class="actions">
      <button type="button" disabled={unavailable} on:click={onedit}>Edit contents</button>
      <button type="button" disabled={unavailable} on:click={onduplicateindependent}>Make independent copy</button>
    </div>
    {#if source.link}
      <p class="link-path" title={source.link.path}>Linked: {source.link.path}</p>
      <p role="status" class:warning={linkStatus?.state === 'missing' || linkStatus?.state === 'changed'}>
        {#if !linkStatus}Link not checked. Stored content is still available.
        {:else if linkStatus.state === 'available'}Verified: the linked file matches its recorded SHA-256.
        {:else if linkStatus.state === 'missing'}The linked file is missing or unreadable. Stored content is preserved.
        {:else if linkStatus.state === 'changed'}The linked file changed. Stored content is preserved until you accept a replacement.
        {:else}Embedded content is in use.{/if}
      </p>
      {#if linkStatus?.detail}<p class="hint">{linkStatus.detail}</p>{/if}
      <div class="actions">
        <button type="button" disabled={disabled || busy} on:click={onchecklinks}>Check link</button>
        <button type="button" disabled={unavailable} on:click={() => onrelink(false)}>Relink same file</button>
        <button type="button" disabled={unavailable} on:click={() => onrelink(true)}>Replace with different file…</button>
      </div>
      <p class="hint">Relink verifies the original bytes. Replace explicitly accepts new content for every shared instance. Editing contents embeds the result; PhotoForge never modifies the external file.</p>
    {:else}
      <p>Embedded source · no external file required.</p>
    {/if}
    <button type="button" disabled={unavailable} on:click={onrasterize}>Rasterize this instance</button>
    <p class="hint">Rasterizing is explicit and undoable. Other instances remain editable.</p>
  {/if}
</section>

<style>
  .smart-panel { display: grid; gap: 8px; }
  h3, p { margin: 0; }
  .actions { display: flex; flex-wrap: wrap; gap: 6px; }
  .hint { font-size: .82rem; opacity: .8; }
  .link-path { overflow-wrap: anywhere; font-size: .85rem; }
  .warning { color: #e6b859; }
</style>
