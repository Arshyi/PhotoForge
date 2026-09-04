<script lang="ts">
  import { onMount } from 'svelte';
  import { invoke } from '@tauri-apps/api/core';
  import { errorMessage, formatBytes } from '../utils/format';
  import type { InferenceStatus } from '../types/editor';

  let status: InferenceStatus | null = null;
  let error = '';
  let busy = false;

  onMount(() => void refresh());

  async function refresh() {
    error = '';
    try {
      status = await invoke<InferenceStatus>('inference_status');
    } catch (reason) {
      error = errorMessage(reason);
    }
  }

  async function remove(id: string) {
    if (busy) return;
    busy = true;
    error = '';
    try {
      status = await invoke<InferenceStatus>('remove_inference_model', { id });
    } catch (reason) {
      error = errorMessage(reason);
    } finally {
      busy = false;
    }
  }

  const capabilityLabels: Record<string, string> = {
    superResolution: 'Super Resolution',
    denoise: 'Denoise',
    deblur: 'Deblur',
    artifactRemoval: 'Artifact Removal',
    segmentation: 'Segmentation',
    faceRestoration: 'Face Restoration',
    inpainting: 'Inpainting'
  };
</script>

<section class="model-manager" aria-label="Local inference models">
  <h3>Local inference models</h3>

  {#if error}
    <p class="error" role="alert">{error}</p>
  {/if}

  {#if status}
    <dl class="summary">
      <dt>Runtime</dt>
      <dd>{status.runtime}</dd>
      <dt>Model folder</dt>
      <dd class="path">{status.modelDirectory}</dd>
    </dl>

    <p class="note">
      PhotoForge never downloads models. Nothing here runs unless you install a model yourself,
      and the editor is complete without one.
    </p>

    {#if status.classicalOnly}
      <p class="note classical" data-testid="classical-only">
        No local model is in use. Every restoration tool in PhotoForge is deterministic and
        runs without one.
      </p>
    {/if}

    <h4>Installed</h4>
    {#if status.installed.length === 0}
      <p class="empty">No models installed.</p>
    {:else}
      <ul class="installed">
        {#each status.installed as model (model.id)}
          <li>
            <div class="identity">
              <strong>{model.name}</strong>
              <span class="meta">{capabilityLabels[model.capability] ?? model.capability}</span>
              <span class="meta">v{model.version}</span>
              {#if model.scale > 1}<span class="meta">{model.scale}x</span>{/if}
            </div>
            <div class="meta-row">
              <span>{formatBytes(model.fileBytes)}</span>
              <span>Tile {model.tileSize} px, overlap {model.tileOverlap} px</span>
              <span>Licence: {model.license || 'not stated'}</span>
            </div>
            <code class="hash" title="SHA-256 of the installed file">{model.sha256}</code>
            <button type="button" disabled={busy} on:click={() => remove(model.id)}>
              Remove {model.name}
            </button>
          </li>
        {/each}
      </ul>
    {/if}

    <h4>Capabilities</h4>
    <ul class="capabilities">
      {#each status.capabilities as capability (capability.capability)}
        <li class:available={capability.available}>
          <span class="name">{capabilityLabels[capability.capability] ?? capability.capability}</span>
          <span class="state">{capability.available ? 'Available' : 'Unavailable'}</span>
          {#if !capability.available}
            <span class="reason">{capability.reason}</span>
          {/if}
        </li>
      {/each}
    </ul>
  {:else if !error}
    <p class="empty">Reading the model store…</p>
  {/if}
</section>

<style>
  .model-manager {
    display: grid;
    gap: 0.6rem;
  }
  h3,
  h4 {
    margin: 0;
  }
  .summary {
    display: grid;
    grid-template-columns: auto 1fr;
    gap: 0.2rem 0.8rem;
    margin: 0;
  }
  .summary dt {
    opacity: 0.75;
  }
  .summary dd {
    margin: 0;
  }
  .path {
    word-break: break-all;
    font-family: ui-monospace, monospace;
    font-size: 0.8rem;
  }
  .note {
    margin: 0;
    font-size: 0.82rem;
    opacity: 0.8;
  }
  .classical {
    opacity: 1;
  }
  .empty {
    margin: 0;
    opacity: 0.7;
  }
  .installed,
  .capabilities {
    list-style: none;
    margin: 0;
    padding: 0;
    display: grid;
    gap: 0.5rem;
  }
  .installed li {
    display: grid;
    gap: 0.25rem;
    padding: 0.5rem;
    border: 1px solid var(--panel-border, rgba(255, 255, 255, 0.15));
    border-radius: 0.35rem;
  }
  .identity {
    display: flex;
    gap: 0.5rem;
    align-items: baseline;
    flex-wrap: wrap;
  }
  .meta,
  .meta-row {
    font-size: 0.8rem;
    opacity: 0.78;
  }
  .meta-row {
    display: flex;
    gap: 0.8rem;
    flex-wrap: wrap;
  }
  .hash {
    font-size: 0.7rem;
    word-break: break-all;
    opacity: 0.65;
  }
  .capabilities li {
    display: flex;
    gap: 0.5rem;
    align-items: baseline;
    flex-wrap: wrap;
    font-size: 0.85rem;
  }
  .capabilities .name {
    min-width: 9rem;
  }
  .capabilities .state {
    opacity: 0.7;
  }
  .capabilities li.available .state {
    opacity: 1;
    font-weight: 600;
  }
  .capabilities .reason {
    flex: 1 1 12rem;
    opacity: 0.7;
  }
  .error {
    margin: 0;
    color: var(--error, #ff8080);
  }
</style>
