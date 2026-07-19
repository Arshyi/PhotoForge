<script lang="ts">
  import { onMount } from 'svelte';
  import { invoke } from '@tauri-apps/api/core';
  import type {
    ComponentDiagnostics,
    ComponentPerformanceMetrics,
    OllamaDiagnostics
  } from '../types/editor';
  import { formatNanoseconds } from '../utils/components';
  import { errorMessage } from '../utils/format';

  let diagnostics: ComponentDiagnostics | null = null;
  let performance: ComponentPerformanceMetrics | null = null;
  let ollama: OllamaDiagnostics | null = null;
  let loading = true;
  let measuring = false;
  let error = '';

  onMount(() => void refresh());

  async function refresh() {
    loading = true;
    error = '';
    try {
      [diagnostics, ollama] = await Promise.all([
        invoke<ComponentDiagnostics>('get_component_diagnostics'),
        invoke<OllamaDiagnostics>('get_ollama_diagnostics')
      ]);
    } catch (reason) {
      error = errorMessage(reason);
    } finally {
      loading = false;
    }
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

  {#if error}<p class="settings-error" role="alert">{error}</p>{/if}
</section>
