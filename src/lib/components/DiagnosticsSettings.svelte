<script lang="ts">
  import { onMount } from 'svelte';
  import { invoke } from '@tauri-apps/api/core';
  import type {
    ComponentDiagnostics,
    ComponentPerformanceMetrics,
    OllamaDiagnostics,
    RenderBackendMode,
    RenderBackendPolicy,
    RenderCacheStats,
    RenderDiagnostics
  } from '../types/editor';
  import { formatNanoseconds } from '../utils/components';
  import { errorMessage, formatBytes } from '../utils/format';

  let diagnostics: ComponentDiagnostics | null = null;
  let performance: ComponentPerformanceMetrics | null = null;
  let ollama: OllamaDiagnostics | null = null;
  let renderer: RenderDiagnostics | null = null;
  let renderPolicy: RenderBackendPolicy | null = null;
  let defaultCacheBytes: number | null = null;
  let cacheBudgetMb = 256;
  let rendererBusy = false;
  let loading = true;
  let measuring = false;
  let error = '';

  onMount(() => void refresh());

  async function refresh() {
    loading = true;
    error = '';
    try {
      const [componentResult, ollamaResult, rendererResult, policyResult, defaultResult] = await Promise.allSettled([
        invoke<ComponentDiagnostics>('get_component_diagnostics'),
        invoke<OllamaDiagnostics>('get_ollama_diagnostics'),
        invoke<RenderDiagnostics>('render_diagnostics'),
        invoke<RenderBackendPolicy>('get_render_backend_mode'),
        invoke<number>('default_render_cache_budget')
      ]);
      const failures: string[] = [];
      if (componentResult.status === 'fulfilled') diagnostics = componentResult.value;
      else failures.push(errorMessage(componentResult.reason));
      if (ollamaResult.status === 'fulfilled') ollama = ollamaResult.value;
      else failures.push(errorMessage(ollamaResult.reason));
      if (rendererResult.status === 'fulfilled' && validRenderDiagnostics(rendererResult.value)) {
        renderer = rendererResult.value;
        cacheBudgetMb = Math.round(renderer.cache.capacityBytes / (1024 * 1024));
      } else if (rendererResult.status === 'rejected') failures.push(errorMessage(rendererResult.reason));
      if (policyResult.status === 'fulfilled' && validRenderPolicy(policyResult.value)) renderPolicy = policyResult.value;
      else if (policyResult.status === 'rejected') failures.push(errorMessage(policyResult.reason));
      if (defaultResult.status === 'fulfilled' && Number.isSafeInteger(defaultResult.value)) defaultCacheBytes = defaultResult.value;
      else if (defaultResult.status === 'rejected') failures.push(errorMessage(defaultResult.reason));
      error = [...new Set(failures)].join(' ');
    } catch (reason) {
      error = errorMessage(reason);
    } finally {
      loading = false;
    }
  }

  async function setBackendMode(event: Event) {
    if (rendererBusy) return;
    const requested = (event.currentTarget as HTMLSelectElement).value as RenderBackendMode;
    const previous = renderPolicy;
    rendererBusy = true; error = '';
    try {
      const result = await invoke<RenderBackendPolicy>('set_render_backend_mode', { mode: requested });
      if (!validRenderPolicy(result)) throw new Error('The renderer returned an invalid backend policy.');
      renderPolicy = result;
      renderer = await invoke<RenderDiagnostics>('render_diagnostics');
    } catch (reason) {
      renderPolicy = previous;
      error = errorMessage(reason);
    } finally { rendererBusy = false; }
  }

  async function setCacheBudget(bytes = Math.round(cacheBudgetMb * 1024 * 1024)) {
    if (rendererBusy || !Number.isSafeInteger(bytes) || bytes < 0 || bytes > 4 * 1024 * 1024 * 1024) return;
    rendererBusy = true; error = '';
    try { replaceCache(await invoke<RenderCacheStats>('set_render_cache_budget', { bytes })); }
    catch (reason) { error = errorMessage(reason); }
    finally { rendererBusy = false; }
  }

  async function clearCache() {
    if (rendererBusy) return;
    rendererBusy = true; error = '';
    try { replaceCache(await invoke<RenderCacheStats>('clear_render_cache')); }
    catch (reason) { error = errorMessage(reason); }
    finally { rendererBusy = false; }
  }

  function replaceCache(cache: RenderCacheStats) {
    if (!renderer || !validCache(cache)) throw new Error('The renderer returned invalid cache diagnostics.');
    renderer = { ...renderer, cache };
    cacheBudgetMb = Math.round(cache.capacityBytes / (1024 * 1024));
  }

  function validCache(value: unknown): value is RenderCacheStats {
    if (!value || typeof value !== 'object') return false;
    const cache = value as Record<string, unknown>;
    return ['hits', 'misses', 'evictions', 'refusals', 'entries', 'bytes', 'capacityBytes', 'diskEntries', 'diskBytes', 'diskCapacityBytes']
      .every((key) => typeof cache[key] === 'number' && Number.isFinite(cache[key]) && Number(cache[key]) >= 0);
  }

  function validRenderDiagnostics(value: unknown): value is RenderDiagnostics {
    if (!value || typeof value !== 'object') return false;
    const item = value as Record<string, unknown>;
    return Number.isFinite(item.tileSize) && Number.isFinite(item.maxThreads) && validCache(item.cache) &&
      typeof item.gpuCompiled === 'boolean' && typeof item.gpuAvailable === 'boolean';
  }

  function validRenderPolicy(value: unknown): value is RenderBackendPolicy {
    if (!value || typeof value !== 'object') return false;
    const item = value as Record<string, unknown>;
    return ['auto', 'cpu', 'gpu'].includes(String(item.mode)) && ['cpu', 'gpu'].includes(String(item.effectiveBackend)) &&
      ['not_probed', 'available', 'unavailable', 'unhealthy'].includes(String(item.adapterStatus)) &&
      typeof item.active === 'boolean' && typeof item.hardDisabled === 'boolean';
  }

  async function measure() {
    measuring = true;
    error = '';
    try {
      performance = await invoke<ComponentPerformanceMetrics>('measure_component_performance', {
        samples: 250
      });
    } catch (reason) {
      error = errorMessage(reason);
    } finally {
      measuring = false;
    }
  }
</script>

<section class="diagnostics-settings" aria-labelledby="diagnostics-heading">
  <div class="settings-intro">
    <span>Local visibility</span>
    <h2 id="diagnostics-heading">Component diagnostics</h2>
    <p>Registry state and validation failures are reported locally. This page sends no telemetry.</p>
  </div>

  {#if loading}
    <p class="settings-state" role="status">Reading diagnostics…</p>
  {:else if diagnostics}
    <div class="diagnostic-summary">
      <div><span>Application</span><strong>PhotoForge {diagnostics.applicationVersion}</strong></div>
      <div><span>Loaded</span><strong>{diagnostics.loadedComponents.length}</strong></div>
      <div><span>Unavailable</span><strong>{diagnostics.unavailableComponents.length}</strong></div>
    </div>

    <div class="diagnostic-section">
      <h3>Registered planners</h3>
      <p>{diagnostics.registeredPlanners.join(' · ')}</p>
    </div>
    <div class="diagnostic-section">
      <h3>Registered restoration engines</h3>
      <p>{diagnostics.registeredEngines.join(' · ')}</p>
    </div>
    <div class="diagnostic-section">
      <h3>Loaded components</h3>
      <p>{diagnostics.loadedComponents.join(' · ') || 'None'}</p>
    </div>
    <div class="diagnostic-section">
      <h3>Unavailable components</h3>
      <p>{diagnostics.unavailableComponents.join(' · ') || 'None'}</p>
    </div>
    <div class="diagnostic-section">
      <h3>Initialization failures</h3>
      <p>{diagnostics.initializationFailures.join(' · ') || 'None recorded'}</p>
    </div>
    <div class="diagnostic-section">
      <h3>Plugin validation errors</h3>
      <p>{diagnostics.pluginValidationErrors.join(' · ') || 'None recorded'}</p>
    </div>
    <div class="diagnostic-section path-section">
      <h3>Configuration path</h3>
      <p>{diagnostics.configurationPath}</p>
    </div>

    {#if ollama}
      <div class="ollama-diagnostics" aria-label="Ollama diagnostics">
        <h3>Ollama Planner</h3>
        <div class="diagnostic-summary">
          <div><span>Connection</span><strong>{ollama.connected ? 'Connected' : 'Disconnected'}</strong></div>
          <div><span>Model selected</span><strong>{ollama.modelSelected ?? 'None'}</strong></div>
          <div><span>Planner version</span><strong>{ollama.plannerVersion}</strong></div>
        </div>
        <div class="diagnostic-section"><h3>Last error</h3><p>{ollama.lastError ?? 'None recorded'}</p></div>
        <div class="performance-results">
          <div><span>Connection latency</span><strong>{ollama.connectionLatencyMs?.toFixed(2) ?? '—'} ms</strong></div>
          <div><span>Generation latency</span><strong>{ollama.generationLatencyMs?.toFixed(2) ?? '—'} ms</strong></div>
          <div><span>Validation latency</span><strong>{ollama.validationLatencyMs?.toFixed(2) ?? '—'} ms</strong></div>
          <div><span>Rule planner latency</span><strong>{ollama.rulePlannerLatencyMs?.toFixed(2) ?? '—'} ms</strong></div>
          <div><span>Comparison latency</span><strong>{ollama.comparisonLatencyMs?.toFixed(2) ?? '—'} ms</strong></div>
          <div><span>Last response</span><strong>{ollama.lastResponseTimeMs?.toFixed(2) ?? '—'} ms</strong></div>
        </div>
        <div class="diagnostic-counters">
          <span>Successful <strong>{ollama.successfulPlans}</strong></span>
          <span>Rejected <strong>{ollama.rejectedPlans}</strong></span>
          <span>Validation failures <strong>{ollama.validationFailures}</strong></span>
          <span>Cancelled <strong>{ollama.cancelledPlans}</strong></span>
        </div>
        <p>{ollama.localClientMemoryEstimateMb} MB client estimate. {ollama.memoryNote}</p>
      </div>
    {/if}

    <div class="performance-panel">
      <div><h3>Local overhead measurement</h3><p>Runs built-in registry, planner, and factory calls only.</p></div>
      <button type="button" disabled={measuring} on:click={measure}>{measuring ? 'Measuring…' : 'Measure'}</button>
    </div>
    {#if performance}
      <div class="performance-results" aria-label="Component performance results">
        <div><span>Registry lookup</span><strong>{formatNanoseconds(performance.registryLookupAverageNs)}</strong></div>
        <div><span>Planner dispatch</span><strong>{formatNanoseconds(performance.plannerDispatchAverageNs)}</strong></div>
        <div><span>Built-in loading / factory</span><strong>{formatNanoseconds(performance.componentFactoryAverageNs)}</strong></div>
        <p>{performance.samples} samples. {performance.note}</p>
      </div>
    {/if}
  {/if}

  {#if !loading && renderer}
    <section class="renderer-panel" aria-label="Render engine diagnostics">
      <div class="performance-panel">
        <div>
          <h3>Render engine</h3>
          <p>The tiled CPU compositor is authoritative. GPU acceleration is limited to eligible wide Gaussian blurs.</p>
        </div>
        <button type="button" disabled={rendererBusy} on:click={refresh}>Refresh</button>
      </div>

      {#if renderPolicy}
        <label class="renderer-mode">
          <span>Acceleration policy</span>
          <select aria-label="Render acceleration policy" value={renderPolicy.mode} disabled={rendererBusy} on:change={setBackendMode}>
            <option value="auto">Auto · measured threshold</option>
            <option value="cpu">CPU only</option>
            <option value="gpu" disabled={!renderer.gpuCompiled || renderPolicy.hardDisabled}>Prefer GPU for eligible blur</option>
          </select>
        </label>
        <div class="diagnostic-summary">
          <div><span>Current policy path</span><strong>{renderPolicy.effectiveBackend.toUpperCase()}</strong></div>
          <div><span>Adapter</span><strong>{renderPolicy.adapterStatus.replace('_', ' ')}</strong></div>
          <div><span>Policy</span><strong>{renderPolicy.active ? 'Active' : 'Inactive'}</strong></div>
        </div>
        {#if renderPolicy.fallbackReason}<p class="renderer-note" role="status">{renderPolicy.fallbackReason}</p>{/if}
      {/if}

      <div class="diagnostic-summary">
        <div><span>Tile edge</span><strong>{renderer.tileSize} px</strong></div>
        <div><span>Worker ceiling</span><strong>{renderer.maxThreads}</strong></div>
        <div><span>GPU build</span><strong>{renderer.gpuCompiled ? 'Included' : 'Not included'}</strong></div>
      </div>
      {#if renderer.gpu}
        <div class="diagnostic-section">
          <h3>Compute adapter</h3>
          <p>{renderer.gpu.adapter} · {renderer.gpu.backend} · {renderer.gpu.deviceType}</p>
          <small>{renderer.gpu.driver || 'Driver details unavailable'} · buffer limit {formatBytes(renderer.gpu.maxBufferBytes)}</small>
        </div>
      {:else if renderer.gpuNote}
        <p class="renderer-note">{renderer.gpuNote}</p>
      {/if}
      {#if renderer.gpuStats}
        <div class="diagnostic-counters" aria-label="GPU operation counters">
          <span>Completed <strong>{renderer.gpuStats.dispatches}</strong></span>
          <span>Processed <strong>{renderer.gpuStats.megapixels} MP</strong></span>
          <span>CPU decisions <strong>{renderer.gpuStats.declined}</strong></span>
          <span>Fallbacks <strong>{renderer.gpuStats.failures}</strong></span>
        </div>
      {/if}

      <div class="cache-panel" aria-label="Render tile cache">
        <div>
          <h3>Render tile cache</h3>
          <p>{renderer.cache.entries} entries · {formatBytes(renderer.cache.bytes)} used of {formatBytes(renderer.cache.capacityBytes)}</p>
          <small>{renderer.cache.hits + renderer.cache.misses
            ? `${((renderer.cache.hits / (renderer.cache.hits + renderer.cache.misses)) * 100).toFixed(1)}% hit rate`
            : 'No cache lookups yet'} · {renderer.cache.evictions} evictions · {renderer.cache.refusals} oversized refusals</small>
        </div>
        {#if renderer.cache.diskCapacityBytes > 0}
          <div class="disk-cache-summary">
            <span>Optional disk tier</span>
            <strong>{renderer.cache.diskEntries} entries · {formatBytes(renderer.cache.diskBytes)} used of {formatBytes(renderer.cache.diskCapacityBytes)}</strong>
          </div>
        {/if}
        <label>
          <span>Render cache budget (MiB)</span>
          <input aria-label="Render cache budget (MiB)" type="number" min="0" max="4096" step="64" bind:value={cacheBudgetMb} disabled={rendererBusy} />
        </label>
        <div class="cache-actions">
          <button type="button" disabled={rendererBusy || cacheBudgetMb < 0 || cacheBudgetMb > 4096} on:click={() => setCacheBudget()}>Apply cache budget</button>
          <button type="button" disabled={rendererBusy || defaultCacheBytes === null} on:click={() => defaultCacheBytes !== null && setCacheBudget(defaultCacheBytes)}>Use default</button>
          <button type="button" disabled={rendererBusy || renderer.cache.entries === 0} on:click={clearCache}>Clear render cache</button>
        </div>
        {#if renderer.cache.diskCapacityBytes > 0}
          <p class="renderer-note">Cached tiles contain local image pixels. The optional disk tier is local, checksummed, bounded, disposable, and cleared with this action; it is enabled only with <code>PHOTOFORGE_RENDER_CACHE_DIR</code> or <code>PHOTOFORGE_ENABLE_DISK_CACHE</code>.</p>
        {:else}
          <p class="renderer-note">Cached tiles contain local image pixels in memory. The disk tier is disabled by default; memory entries are discarded when PhotoForge exits.</p>
        {/if}
      </div>
    </section>
  {/if}

  {#if error}<p class="settings-error" role="alert">{error}</p>{/if}
</section>

<style>
  .renderer-panel, .cache-panel { display: grid; gap: 9px; }
  .renderer-panel { margin-top: 16px; padding-top: 14px; border-top: 1px solid var(--line); }
  .renderer-mode { display: grid; gap: 5px; color: var(--ink-soft); font-size: .68rem; }
  .renderer-mode select, .cache-panel input { min-width: 0; padding: 7px; border: 1px solid var(--line); border-radius: 6px; color: var(--ink); background: var(--surface-raised); }
  .renderer-note { margin: 0; color: var(--ink-faint); font-size: .61rem; line-height: 1.45; }
  .cache-panel { padding: 10px; border: 1px solid var(--line); border-radius: 8px; background: var(--surface-soft); }
  .cache-panel h3, .cache-panel p { margin: 0; }
  .disk-cache-summary { display: grid; gap: 3px; color: var(--ink-soft); font-size: .63rem; }
  .disk-cache-summary strong { color: var(--ink); font-weight: 600; }
  .cache-panel > label { display: grid; gap: 4px; color: var(--ink-soft); font-size: .63rem; }
  .cache-actions { display: flex; flex-wrap: wrap; gap: 5px; }
  .cache-actions button { padding: 6px 8px; }
</style>
