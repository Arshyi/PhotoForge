<script lang="ts">
  import { onMount } from 'svelte';
  import { invoke } from '@tauri-apps/api/core';
  import type {
    ComponentConfiguration,
    ComponentSnapshot,
    EngineProvider,
    ModelDiscoveryResult,
    OllamaConnectionResult,
    OllamaModelDiscoveryResult,
    PlannerProvider,
    PluginScanResult
  } from '../types/editor';
  import { errorMessage } from '../utils/format';
  import {
    capabilityLabels,
    componentStatus,
    formatMemoryEstimate,
    splitModelDirectories
  } from '../utils/components';
  import {
    formatOllamaModelSize,
    formatOllamaModifiedDate,
    modelCapabilitySummary,
    normalizeOllamaConfiguration,
    resetOllamaConfiguration
  } from '../utils/ollama';

  let snapshot: ComponentSnapshot | null = null;
  let configuration: ComponentConfiguration | null = null;
  let modelDirectories = '';
  let loading = true;
  let busy = '';
  let message = '';
  let error = '';
  let modelResult: ModelDiscoveryResult | null = null;
  let ollamaModels: OllamaModelDiscoveryResult | null = null;
  let pluginResult: PluginScanResult | null = null;

  onMount(() => void refresh());

  async function refresh() {
    loading = true;
    error = '';
    try {
      setSnapshot(await invoke<ComponentSnapshot>('get_component_snapshot'));
    } catch (reason) {
      error = errorMessage(reason);
    } finally {
      loading = false;
    }
  }

  function setSnapshot(next: ComponentSnapshot) {
    snapshot = next;
    configuration = { ...next.configuration, modelDirectories: [...next.configuration.modelDirectories] };
    modelDirectories = configuration.modelDirectories.join('\n');
  }

  async function selectPlanner(provider: PlannerProvider) {
    if (!configuration || provider === configuration.activePlanner) return;
    await run('planner', async () => {
      setSnapshot(await invoke<ComponentSnapshot>('select_planner_provider', { provider }));
      message = 'Planner selection updated.';
    });
  }

  async function selectEngine(provider: EngineProvider) {
    if (!configuration || provider === configuration.activeEngine) return;
    await run('engine', async () => {
      setSnapshot(await invoke<ComponentSnapshot>('select_restoration_engine', { provider }));
      message = 'Restoration engine selection updated.';
    });
  }

  async function saveConfiguration() {
    if (!configuration) return;
    const next = normalizeOllamaConfiguration({
      ...configuration,
      modelDirectories: splitModelDirectories(modelDirectories)
    });
    await run('save', async () => {
      setSnapshot(
        await invoke<ComponentSnapshot>('update_component_configuration', { configuration: next })
      );
      message = 'Component settings saved locally.';
    });
  }

  async function testConnection() {
    await run('connection', async () => {
      const result = await invoke<OllamaConnectionResult>('test_ollama_connection');
      message = `${result.message} ${result.responseTimeMs.toFixed(1)} ms`;
    });
  }

  async function refreshOllamaModels() {
    await run('ollama-models', async () => {
      ollamaModels = await invoke<OllamaModelDiscoveryResult>('refresh_ollama_models');
      message = ollamaModels.message;
      if (
        configuration?.ollamaSelectedModel &&
        !ollamaModels.models.some((model) => model.name === configuration?.ollamaSelectedModel)
      ) {
        configuration.ollamaSelectedModel = null;
      }
    });
  }

  function resetOllamaDefaults() {
    if (!configuration) return;
    configuration = resetOllamaConfiguration(configuration);
    message = 'Ollama settings reset. Save locally to persist them.';
  }

  async function discoverModels() {
    await run('models', async () => {
      modelResult = await invoke<ModelDiscoveryResult>('discover_models');
      message = modelResult.message;
    });
  }

  async function scanPlugins() {
    await run('plugins', async () => {
      pluginResult = await invoke<PluginScanResult>('scan_plugins');
      message = pluginResult.message;
    });
  }

  async function run(name: string, action: () => Promise<void>) {
    busy = name;
    error = '';
    message = '';
    try {
      await action();
    } catch (reason) {
      error = errorMessage(reason);
    } finally {
      busy = '';
    }
  }
</script>

<section class="component-settings" aria-labelledby="components-heading">
  <div class="settings-intro">
    <span>Extensible · optional</span>
    <h2 id="components-heading">Components</h2>
    <p>The built-in Rule Planner remains the default. Ollama is an optional local planning adapter; unavailable future components cannot be activated.</p>
  </div>

  {#if loading}
    <p class="settings-state" role="status">Reading the local component registry…</p>
  {:else if snapshot && configuration}
    <div class="provider-selectors">
      <label>
        <span>Active planner</span>
        <select
          aria-label="Active planner"
          value={configuration.activePlanner}
          disabled={Boolean(busy)}
          on:change={(event) => selectPlanner(event.currentTarget.value as PlannerProvider)}
        >
          {#each snapshot.planners as planner}
            <option value={planner.id} disabled={!planner.installed}>{planner.name}{planner.installed ? '' : ' — unavailable'}</option>
          {/each}
        </select>
      </label>
      <label>
        <span>Active restoration engine</span>
        <select
          aria-label="Active restoration engine"
          value={configuration.activeEngine}
          disabled={Boolean(busy)}
          on:change={(event) => selectEngine(event.currentTarget.value as EngineProvider)}
        >
          {#each snapshot.engines as engine}
            <option value={engine.id} disabled={!engine.installed}>{engine.name}{engine.installed ? '' : ' — unavailable'}</option>
          {/each}
        </select>
      </label>
    </div>

    <h3 class="settings-section-title">Registered planners</h3>
    <div class="component-grid" aria-label="Registered planners">
      {#each snapshot.planners as planner}
        <article class:active={planner.active} class:unavailable={!planner.installed}>
          <div class="component-title">
            <div><strong>{planner.name}</strong><small>{planner.provider} · {planner.version}</small></div>
            <em>{componentStatus(planner)}</em>
          </div>
          <p>{formatMemoryEstimate(planner.memoryEstimateMb)} · {planner.loaded ? 'Loaded' : 'Lazy'}</p>
          <div class="capability-list">
            {#each capabilityLabels(planner.capabilities) as capability}<span>{capability}</span>{/each}
          </div>
          {#if planner.unavailableReason}<p class="component-reason">{planner.unavailableReason}</p>{/if}
        </article>
      {/each}
    </div>

    <h3 class="settings-section-title">Registered restoration engines</h3>
    <div class="component-grid" aria-label="Registered restoration engines">
      {#each snapshot.engines as engine}
        <article class:active={engine.active} class:unavailable={!engine.installed}>
          <div class="component-title">
            <div><strong>{engine.name}</strong><small>{engine.provider} · {engine.version}</small></div>
            <em>{componentStatus(engine)}</em>
          </div>
          <p>{formatMemoryEstimate(engine.memoryEstimateMb)} · {engine.loaded ? 'Loaded' : 'Lazy'}</p>
          <div class="capability-list">
            {#each capabilityLabels(engine.capabilities) as capability}<span>{capability}</span>{/each}
          </div>
          {#if engine.unavailableReason}<p class="component-reason">{engine.unavailableReason}</p>{/if}
        </article>
      {/each}
    </div>

    <div class="configuration-form">
      <h3>Local configuration</h3>
      <label>
        <span>Ollama endpoint <small>Local loopback only; never contacted automatically.</small></span>
        <div class="field-action">
          <input aria-label="Ollama endpoint" bind:value={configuration.plannerEndpoint} />
          <button type="button" disabled={Boolean(busy)} on:click={testConnection}>{busy === 'connection' ? 'Testing…' : 'Test Connection'}</button>
        </div>
      </label>
      <label>
        <span>Request and response timeout <small>250–120,000 milliseconds</small></span>
        <input aria-label="Ollama timeout" type="number" min="250" max="120000" step="250" bind:value={configuration.ollamaTimeoutMs} />
      </label>
      <label>
        <span>Maximum response size <small>1,024–2,097,152 bytes</small></span>
        <input aria-label="Maximum response size" type="number" min="1024" max="2097152" step="1024" bind:value={configuration.ollamaMaxResponseBytes} />
      </label>
      <label>
        <span>Maximum generated operations <small>1–8 validated operations</small></span>
        <input aria-label="Maximum generated operations" type="number" min="1" max="8" step="1" bind:value={configuration.ollamaMaxOperations} />
      </label>
      <label>
        <span>Planner model <small>Only models already installed in Ollama are listed.</small></span>
        <select aria-label="Planner model" bind:value={configuration.ollamaSelectedModel}>
          <option value={null}>Choose an installed model</option>
          {#if ollamaModels}
            {#each ollamaModels.models as model}<option value={model.name}>{model.name}</option>{/each}
          {:else if configuration.ollamaSelectedModel}
            <option value={configuration.ollamaSelectedModel}>{configuration.ollamaSelectedModel}</option>
          {/if}
        </select>
      </label>
      <label>
        <span>Model directories <small>One local directory per line; files are inspected as metadata only.</small></span>
        <textarea aria-label="Model directories" rows="3" bind:value={modelDirectories}></textarea>
      </label>
      <label>
        <span>Plugin manifest directory <small>JSON manifests are validated, never executed.</small></span>
        <input aria-label="Plugin manifest directory" bind:value={configuration.pluginDirectory} />
      </label>
      <label>
        <span>Initialization timeout <small>100–30,000 milliseconds</small></span>
        <input aria-label="Initialization timeout" type="number" min="100" max="30000" step="100" bind:value={configuration.initializationTimeoutMs} />
      </label>
      <div class="configuration-actions">
        <button class="primary" type="button" disabled={Boolean(busy)} on:click={saveConfiguration}>{busy === 'save' ? 'Saving…' : 'Save locally'}</button>
        <button type="button" disabled={Boolean(busy)} on:click={refreshOllamaModels}>{busy === 'ollama-models' ? 'Refreshing…' : 'Refresh Models'}</button>
        <button type="button" disabled={Boolean(busy)} on:click={resetOllamaDefaults}>Reset Ollama defaults</button>
        <button type="button" disabled={Boolean(busy)} on:click={discoverModels}>{busy === 'models' ? 'Scanning…' : 'Discover Models'}</button>
        <button type="button" disabled={Boolean(busy)} on:click={scanPlugins}>{busy === 'plugins' ? 'Scanning…' : 'Validate Plugins'}</button>
      </div>
    </div>

    {#if ollamaModels}
      <div class="scan-result" aria-label="Ollama model discovery result">
        <strong>{ollamaModels.message}</strong>
        {#each ollamaModels.models as model}
          <p><span>{model.name} · {formatOllamaModelSize(model.sizeBytes)}</span><small>{formatOllamaModifiedDate(model.modifiedAt)} · {modelCapabilitySummary(model.capabilities)}</small></p>
        {/each}
      </div>
    {/if}

    {#if modelResult}
      <div class="scan-result" aria-label="Model discovery result">
        <strong>{modelResult.message}</strong>
        {#each modelResult.models as model}
          <p><span>{model.name} · {model.format} · {formatMemoryEstimate(model.memoryEstimateMb)}</span><small>{model.unavailableReason}</small></p>
        {/each}
      </div>
    {/if}

    {#if pluginResult}
      <div class="scan-result" aria-label="Plugin validation result">
        <strong>{pluginResult.message}</strong>
        {#each pluginResult.records as record}
          <p><span>{record.manifest?.name ?? record.manifestPath}</span><small>{record.valid ? 'Manifest valid · execution disabled' : record.error}</small></p>
        {/each}
      </div>
    {/if}
  {/if}

  {#if message}<p class="settings-message" role="status">{message}</p>{/if}
  {#if error}<p class="settings-error" role="alert">{error}</p>{/if}
</section>
