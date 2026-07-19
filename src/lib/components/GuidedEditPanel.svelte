<script lang="ts">
  import { onMount } from 'svelte';
  import { invoke } from '@tauri-apps/api/core';
  import type {
    ComponentSnapshot,
    EditOperation,
    EditPlan,
    GuidedHistoryEntry,
    GuidedPlanner,
    GuidedSettings,
    OllamaConnectionResult,
    OllamaDiagnostics,
    OllamaModelDiscoveryResult,
    OllamaPlanResult,
    PlanResult,
    PlanValidationReport,
    PlannerComparisonResult
  } from '../types/editor';
  import { errorMessage } from '../utils/format';
  import {
    confidenceLabel,
    defaultGuidedSettings,
    loadRecentRequests,
    movePlanOperation,
    planValueControl,
    rememberRecentRequest,
    removePlanOperation,
    suggestedPrompts,
    updatePlanOperation,
    withPlanValue
  } from '../utils/guided';
  import { operationLabels } from '../utils/operations';

  export let documentId = 0;
  export let ready = false;
  export let disabled = false;
  export let settings: GuidedSettings = defaultGuidedSettings;
  export let configurationRevision = 0;
  export let onapply: (operations: EditOperation[]) => void = () => undefined;
  export let onmessage: (message: string, kind?: 'error' | 'success') => void = () => undefined;

  let request = '';
  let recentRequests: GuidedHistoryEntry[] = [];
  let planner: GuidedPlanner = 'rule';
  let selectedModel: string | null = null;
  let ollamaConfigured = false;
  let maximumOperations = 8;
  let ollamaConnected = false;
  let providerBusy = '';
  let providerMessage = '';
  let plan: EditPlan | null = null;
  let inspectorOpen = false;
  let planning = false;
  let applying = false;
  let generation = 0;
  let activeRequestId = 0;
  let activeProvider: GuidedPlanner = 'rule';
  let planTime = 0;
  let observedDocumentId = documentId;
  let observedConfigurationRevision = configurationRevision;
  let validationReport: PlanValidationReport | null = null;
  let reportOpen = false;
  let rawOpen = false;
  let comparison: PlannerComparisonResult | null = null;
  let ollamaError = '';

  $: ollamaConfigured = Boolean(selectedModel);

  onMount(() => {
    if (settings.rememberPromptHistory) recentRequests = loadRecentRequests();
    void loadPlannerState();
  });

  $: if (documentId !== observedDocumentId) {
    observedDocumentId = documentId;
    cancelPlan();
    comparison = null;
  }

  $: if (configurationRevision !== observedConfigurationRevision) {
    observedConfigurationRevision = configurationRevision;
    void loadPlannerState();
  }

  async function loadPlannerState() {
    try {
      const [snapshot, diagnostics] = await Promise.all([
        invoke<ComponentSnapshot>('get_component_snapshot'),
        invoke<OllamaDiagnostics>('get_ollama_diagnostics')
      ]);
      selectedModel = snapshot.configuration.ollamaSelectedModel;
      maximumOperations = snapshot.configuration.ollamaMaxOperations;
      ollamaConnected = diagnostics.connected;
      planner = snapshot.configuration.activePlanner === 'ollama' && selectedModel ? 'ollama' : 'rule';
    } catch {
      // Rule Planner remains available even when optional component state cannot be read.
    }
  }

  async function generatePlan(requestOverride?: string) {
    const plannedRequest = (requestOverride ?? request).trim();
    if (!plannedRequest) {
      onmessage('Enter a guided editing request.', 'error');
      return;
    }
    if (!ready || disabled) {
      onmessage('Open an image and wait for local analysis before planning.', 'error');
      return;
    }
    if (planner === 'ollama' && !ollamaConfigured) {
      onmessage('Choose an installed Ollama model in Components first.', 'error');
      return;
    }
    request = plannedRequest;
    const ownGeneration = ++generation;
    activeRequestId = ownGeneration;
    activeProvider = planner;
    planning = true;
    ollamaError = '';
    comparison = null;
    validationReport = null;
    rawOpen = false;
    reportOpen = false;
    try {
      if (planner === 'ollama') {
        const result = await invoke<OllamaPlanResult>('generate_ollama_plan', {
          request: plannedRequest,
          documentId,
          requestId: ownGeneration
        });
        if (ownGeneration !== generation || result.documentId !== documentId) return;
        validationReport = result.validationReport;
        planTime = result.totalTimeMs;
        if (result.isCurrent && result.plan) {
          plan = result.plan;
          inspectorOpen = settings.autoOpenPlanInspector;
          ollamaConnected = true;
          remember(plannedRequest, 'Ollama');
        } else {
          plan = null;
          ollamaError = result.error ?? 'Ollama returned a plan that could not be validated.';
          reportOpen = true;
          onmessage(ollamaError, 'error');
        }
      } else {
        const result = await invoke<PlanResult>('generate_edit_plan', {
          request: plannedRequest,
          documentId,
          requestId: ownGeneration
        });
        if (
          result.isCurrent &&
          result.requestId === generation &&
          result.documentId === documentId &&
          result.plan
        ) {
          plan = result.plan;
          planTime = result.processingTimeMs;
          inspectorOpen = settings.autoOpenPlanInspector;
          remember(plannedRequest, 'Rule');
        }
      }
    } catch (error) {
      if (ownGeneration === generation) {
        const message = errorMessage(error);
        if (planner === 'ollama') ollamaError = message;
        onmessage(message, 'error');
      }
    } finally {
      if (ownGeneration === generation) planning = false;
    }
  }

  function remember(prompt: string, provider: GuidedHistoryEntry['provider']) {
    if (settings.rememberPromptHistory) {
      recentRequests = rememberRecentRequest(recentRequests, prompt, provider);
    }
  }

  async function applyPlan() {
    if (!plan || applying || plan.operations.length === 0) return;
    applying = true;
    try {
      const validated = await invoke<EditPlan>('validate_guided_plan', { plan });
      onapply(validated.operations);
      onmessage(`Applied ${validated.operations.length} reviewed guided edits.`);
      plan = null;
      inspectorOpen = false;
    } catch (error) {
      onmessage(errorMessage(error), 'error');
    } finally {
      applying = false;
    }
  }

  function cancelPlan() {
    const cancelledRequest = activeRequestId;
    const shouldCancelRemote = planning && activeProvider === 'ollama';
    generation += 1;
    planning = false;
    plan = null;
    inspectorOpen = false;
    if (shouldCancelRemote) {
      void invoke('cancel_ollama_plan', { requestId: cancelledRequest }).catch(() => undefined);
    }
  }

  function handlePromptInput(event: Event) {
    const next = (event.currentTarget as HTMLTextAreaElement).value;
    if (planning && next !== request) cancelPlan();
    request = next;
  }

  async function testConnection() {
    providerBusy = 'connection';
    providerMessage = '';
    try {
      const result = await invoke<OllamaConnectionResult>('test_ollama_connection');
      ollamaConnected = result.connected;
      providerMessage = `${result.message} ${result.responseTimeMs.toFixed(1)} ms`;
    } catch (error) {
      ollamaConnected = false;
      providerMessage = errorMessage(error);
    } finally {
      providerBusy = '';
    }
  }

  async function refreshModels() {
    providerBusy = 'models';
    providerMessage = '';
    try {
      const result = await invoke<OllamaModelDiscoveryResult>('refresh_ollama_models');
      providerMessage = result.message;
      ollamaConnected = true;
      if (selectedModel && !result.models.some((model) => model.name === selectedModel)) {
        selectedModel = null;
        planner = 'rule';
      }
    } catch (error) {
      ollamaConnected = false;
      providerMessage = errorMessage(error);
    } finally {
      providerBusy = '';
    }
  }

  async function validateRawJson() {
    if (!validationReport?.originalResponse) return;
    validationReport = await invoke<PlanValidationReport>('validate_ollama_json', {
      rawJson: validationReport.originalResponse,
      maximumOperations
    });
    reportOpen = true;
  }

  async function comparePlans() {
    const plannedRequest = request.trim();
    if (!plannedRequest || !ready || !ollamaConfigured) return;
    const ownGeneration = ++generation;
    activeRequestId = ownGeneration;
    activeProvider = 'ollama';
    planning = true;
    comparison = null;
    ollamaError = '';
    try {
      comparison = await invoke<PlannerComparisonResult>('compare_planners', {
        request: plannedRequest,
        documentId,
        requestId: ownGeneration
      });
      validationReport = comparison.validationReport;
    } catch (error) {
      if (ownGeneration === generation) onmessage(errorMessage(error), 'error');
    } finally {
      if (ownGeneration === generation) planning = false;
    }
  }

  function useRulePlanner() {
    planner = 'rule';
    ollamaError = '';
    void generatePlan();
  }

  function chooseSuggestion(prompt: string) {
    request = prompt;
    void generatePlan(prompt);
  }

  function chooseHistory(entry: GuidedHistoryEntry) {
    request = entry.prompt;
    planner = entry.provider === 'Ollama' && ollamaConfigured ? 'ollama' : 'rule';
  }

  function removeOperation(index: number) {
    if (plan) plan = removePlanOperation(plan, index);
  }

  function moveOperation(index: number, delta: -1 | 1) {
    if (plan) plan = movePlanOperation(plan, index, delta);
  }

  function updateValue(index: number, operation: EditOperation, value: number) {
    if (plan) plan = updatePlanOperation(plan, index, withPlanValue(operation, value));
  }

  function handleRequestKey(event: KeyboardEvent) {
    if (event.key === 'Escape' && (plan || planning)) {
      event.preventDefault();
      cancelPlan();
      return;
    }
    if (event.key !== 'Enter' || event.shiftKey) return;
    event.preventDefault();
    if (event.ctrlKey || event.metaKey) {
      if (plan) void applyPlan();
      return;
    }
    void generatePlan();
  }
</script>

<section class="tool-section guided-section" aria-labelledby="guided-heading">
  <div class="guided-heading">
    <h2 id="guided-heading"><span>◇</span> Guided Edit</h2>
    <em>{planner === 'rule' ? 'Rule · on-device' : 'Ollama · local endpoint'}</em>
  </div>
  <p class="guided-intro">
    Describe the result. Every provider can only propose existing deterministic operations for review.
  </p>

  <label class="planner-selector">
    <span>Planner</span>
    <select aria-label="Planner" bind:value={planner} disabled={planning || disabled}>
      <option value="rule">Rule Planner</option>
      <option value="ollama" disabled={!ollamaConfigured}>Ollama Planner</option>
    </select>
  </label>

  <div class="ollama-actions" aria-label="Ollama controls">
    <button type="button" disabled={Boolean(providerBusy)} on:click={testConnection}>
      {providerBusy === 'connection' ? 'Testing…' : 'Test Connection'}
    </button>
    <button type="button" disabled={Boolean(providerBusy)} on:click={refreshModels}>
      {providerBusy === 'models' ? 'Refreshing…' : 'Refresh Models'}
    </button>
    <span class:connected={ollamaConnected}>{ollamaConnected ? 'Connected' : 'Disconnected'}</span>
  </div>
  {#if selectedModel}<p class="planner-model">Planner model · <strong>{selectedModel}</strong></p>{/if}
  {#if providerMessage}<p class="provider-message" role="status">{providerMessage}</p>{/if}

  <div class="guided-request">
    <label for="guided-request-input">Editing request</label>
    <textarea
      id="guided-request-input"
      aria-describedby="guided-request-help"
      value={request}
      rows="3"
      maxlength="1000"
      placeholder="Make this darker but bring out the writing"
      disabled={disabled}
      on:input={handlePromptInput}
      on:keydown={handleRequestKey}
    ></textarea>
    <small id="guided-request-help">Enter plans · Ctrl+Enter applies a reviewed plan · Escape cancels</small>
  </div>
  <div class="plan-actions">
    <button
      class="plan-button"
      type="button"
      disabled={disabled || !ready || planning || !request.trim() || (planner === 'ollama' && !ollamaConfigured)}
      on:click={() => generatePlan()}
    >
      {planning ? 'Planning locally…' : 'Generate Plan'}
    </button>
    {#if planning}<button type="button" on:click={cancelPlan}>Cancel</button>{/if}
    <button type="button" disabled={disabled || !ready || planning || !request.trim() || !ollamaConfigured} on:click={comparePlans}>Compare Planners</button>
  </div>

  {#if ollamaError}
    <div class="planner-fallback" role="alert">
      <p>{ollamaError}</p>
      <button type="button" on:click={useRulePlanner}>Use Rule Planner Instead</button>
    </div>
  {/if}

  <details class="prompt-suggestions">
    <summary>Suggested prompts</summary>
    <div>
      {#each suggestedPrompts as prompt}
        <button type="button" disabled={disabled || !ready || planning} on:click={() => chooseSuggestion(prompt)}>
          {prompt}
        </button>
      {/each}
    </div>
  </details>

  {#if settings.rememberPromptHistory && recentRequests.length}
    <details class="recent-requests">
      <summary>Recent requests <span>{recentRequests.length}/25</span></summary>
      <div>
        {#each recentRequests as recent}
          <button type="button" title={recent.prompt} on:click={() => chooseHistory(recent)}>
            <small>{recent.provider}</small>{recent.prompt}
          </button>
        {/each}
      </div>
    </details>
  {/if}

  {#if comparison}
    <article class="planner-comparison" aria-label="Planner comparison">
      <header><strong>Planner comparison</strong><small>{comparison.totalTimeMs.toFixed(1)} ms total · no automatic winner</small></header>
      <div>
        {#each [comparison.rule, comparison.ollama] as candidate}
          <section>
            <h3>{candidate.provider} Planner <small>{candidate.executionTimeMs.toFixed(1)} ms</small></h3>
            {#if candidate.plan}
              <p>{candidate.plan.summary}</p>
              <p><strong>{Math.round(candidate.plan.confidence * 100)}%</strong> confidence · {candidate.plan.operations.length} operations</p>
              {#if candidate.plan.warnings.length}<ul>{#each candidate.plan.warnings as warning}<li>{warning}</li>{/each}</ul>{/if}
              <ol>{#each candidate.plan.operations as operation}<li>{operationLabels[operation.type]}</li>{/each}</ol>
            {:else}<p class="comparison-error">{candidate.error}</p>{/if}
          </section>
        {/each}
      </div>
    </article>
  {/if}

  {#if validationReport}
    <div class="raw-actions">
      <button type="button" on:click={() => (rawOpen = !rawOpen)}>View Raw JSON</button>
      <button type="button" on:click={validateRawJson}>Validate JSON</button>
      <button type="button" on:click={() => (reportOpen = !reportOpen)}>View Validation Report</button>
    </div>
    {#if rawOpen}
      <label class="raw-json-viewer">
        <span>Original response · read-only</span>
        <textarea aria-label="Original Ollama response" readonly rows="8" value={validationReport.originalResponse}></textarea>
      </label>
    {/if}
    {#if reportOpen}
      <article class="validation-report" aria-label="Validation report">
        <header><strong>{validationReport.valid ? 'Valid plan' : 'Rejected plan'}</strong><small>{validationReport.validationTimeMs.toFixed(2)} ms</small></header>
        <p>Rejected fields: {validationReport.rejectedFields.join(', ') || 'None'}</p>
        <ul>{#each validationReport.errors as error}<li>{error}</li>{/each}</ul>
        {#if validationReport.validatedResponse}<details><summary>Validated response</summary><pre>{validationReport.validatedResponse}</pre></details>{/if}
      </article>
    {/if}
  {/if}

  {#if plan}
    <article class="plan-card" aria-label="Guided edit plan">
      <header>
        <div><span>Validated {planner === 'ollama' ? 'Ollama' : 'Rule'} plan</span><small>{planTime.toFixed(2)} ms</small></div>
        <button type="button" on:click={() => (inspectorOpen = true)} disabled={inspectorOpen}>Edit Plan</button>
      </header>
      <h3>Summary</h3>
      <p>{plan.summary}</p>

      {#if settings.showConfidence}
        <div class="plan-confidence">
          <span>Confidence</span>
          <strong>{confidenceLabel(plan.confidence)} · {Math.round(plan.confidence * 100)}%</strong>
          <small>Provider-reported planning confidence, never permission to apply automatically.</small>
        </div>
      {/if}

      {#if settings.showWarnings && plan.warnings.length}
        <div class="plan-warnings" role="note">
          <strong>Warnings</strong>
          <ul>{#each plan.warnings as warning}<li>{warning}</li>{/each}</ul>
        </div>
      {/if}

      <h3>Operations</h3>
      <ol class="plan-operations" aria-label="Planned operations">
        {#each plan.operations as operation, index}
          {@const control = planValueControl(operation)}
          <li>
            <div class="plan-operation-heading">
              <span><b>{index + 1}</b><strong>{operationLabels[operation.type]}</strong></span>
              {#if inspectorOpen}
                <div>
                  <button type="button" aria-label={`Move ${operationLabels[operation.type]} up`} disabled={index === 0} on:click={() => moveOperation(index, -1)}>↑</button>
                  <button type="button" aria-label={`Move ${operationLabels[operation.type]} down`} disabled={index === plan.operations.length - 1} on:click={() => moveOperation(index, 1)}>↓</button>
                  <button type="button" aria-label={`Delete ${operationLabels[operation.type]}`} on:click={() => removeOperation(index)}>×</button>
                </div>
              {/if}
            </div>
            <p>{plan.operationExplanations[index]}</p>
            {#if inspectorOpen && control}
              <label class="plan-value">
                <span>{operationLabels[operation.type]} {control.noun}<output>{control.value.toFixed(2)}</output></span>
                <input
                  aria-label={`${operationLabels[operation.type]} ${control.noun}`}
                  type="range"
                  value={control.value}
                  min={control.min}
                  max={control.max}
                  step={control.step}
                  on:input={(event) => updateValue(index, operation, Number(event.currentTarget.value))}
                />
              </label>
            {/if}
          </li>
        {/each}
      </ol>

      {#if plan.operations.length === 0}<p class="empty-plan" role="alert">Add a new request or cancel; an empty plan cannot be applied.</p>{/if}
      <footer>
        <button type="button" class="apply-plan" disabled={applying || plan.operations.length === 0} on:click={applyPlan}>
          {applying ? 'Validating…' : 'Apply'}
        </button>
        <button type="button" on:click={cancelPlan}>Cancel</button>
      </footer>
    </article>
  {/if}

  <p class="truth-note">Only the deterministic engine can modify pixels. Ollama receives text metrics and the prompt only after an explicit action.</p>
</section>
