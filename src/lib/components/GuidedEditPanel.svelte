<script lang="ts">
  import { onMount } from 'svelte';
  import { invoke } from '@tauri-apps/api/core';
  import type {
    EditOperation,
    EditPlan,
    GuidedSettings,
    PlanResult
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
  export let onapply: (operations: EditOperation[]) => void = () => undefined;
  export let onmessage: (message: string, kind?: 'error' | 'success') => void = () => undefined;

  let request = '';
  let recentRequests: string[] = [];
  let plan: EditPlan | null = null;
  let inspectorOpen = false;
  let planning = false;
  let applying = false;
  let generation = 0;
  let planTime = 0;
  let observedDocumentId = documentId;

  onMount(() => {
    if (settings.rememberPromptHistory) recentRequests = loadRecentRequests();
  });

  $: if (documentId !== observedDocumentId) {
    observedDocumentId = documentId;
    generation += 1;
    plan = null;
    inspectorOpen = false;
    planning = false;
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
    request = plannedRequest;
    const ownGeneration = ++generation;
    planning = true;
    try {
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
        if (settings.rememberPromptHistory) {
          recentRequests = rememberRecentRequest(recentRequests, plannedRequest);
        }
      }
    } catch (error) {
      if (ownGeneration === generation) onmessage(errorMessage(error), 'error');
    } finally {
      if (ownGeneration === generation) planning = false;
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
    generation += 1;
    planning = false;
    plan = null;
    inspectorOpen = false;
  }

  function chooseSuggestion(prompt: string) {
    request = prompt;
    void generatePlan(prompt);
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
    <em>Rule-based · local</em>
  </div>
  <p class="guided-intro">
    Describe the result. PhotoForge proposes only existing deterministic operations for your review.
  </p>

  <div class="guided-request">
    <label for="guided-request-input">Editing request</label>
    <textarea
      id="guided-request-input"
      aria-describedby="guided-request-help"
      bind:value={request}
      rows="3"
      maxlength="1000"
      placeholder="Make this darker but bring out the writing"
      disabled={disabled}
      on:keydown={handleRequestKey}
    ></textarea>
    <small id="guided-request-help">Enter plans · Ctrl+Enter applies a reviewed plan · Escape cancels</small>
  </div>
  <button
    class="plan-button"
    type="button"
    disabled={disabled || !ready || planning || !request.trim()}
    on:click={() => generatePlan()}
  >
    {planning ? 'Planning locally…' : 'Generate Plan'}
  </button>

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
          <button type="button" title={recent} on:click={() => (request = recent)}>{recent}</button>
        {/each}
      </div>
    </details>
  {/if}

  {#if plan}
    <article class="plan-card" aria-label="Guided edit plan">
      <header>
        <div><span>Typed edit plan</span><small>{planTime.toFixed(2)} ms</small></div>
        <button type="button" on:click={() => (inspectorOpen = true)} disabled={inspectorOpen}>Edit Plan</button>
      </header>
      <h3>Summary</h3>
      <p>{plan.summary}</p>

      {#if settings.showConfidence}
        <div class="plan-confidence">
          <span>Confidence</span>
          <strong>{confidenceLabel(plan.confidence)} · {Math.round(plan.confidence * 100)}%</strong>
          <small>Heuristic rule-match strength, not AI certainty.</small>
        </div>
      {/if}

      {#if settings.showWarnings && plan.warnings.length}
        <div class="plan-warnings" role="note">
          <strong>Warnings</strong>
          <ul>
            {#each plan.warnings as warning}<li>{warning}</li>{/each}
          </ul>
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

      {#if plan.operations.length === 0}
        <p class="empty-plan" role="alert">Add a new request or cancel; an empty plan cannot be applied.</p>
      {/if}
      <footer>
        <button type="button" class="apply-plan" disabled={applying || plan.operations.length === 0} on:click={applyPlan}>
          {applying ? 'Validating…' : 'Apply'}
        </button>
        <button type="button" on:click={cancelPlan}>Cancel</button>
      </footer>
    </article>
  {/if}

  <p class="truth-note">The planner never edits pixels, runs code, or contacts a service.</p>
</section>
