<script lang="ts">
  import { onMount } from 'svelte';
  import { resourceStatus, setMemoryBudget } from '../resources/client';
  import {
    describeBasis,
    describeLimits,
    gibibytes,
    manualRange,
    manualWarning,
    megapixels,
    snapBudget,
    waitsForNextDocument
  } from '../resources/budget';
  import type { ResourceStatus } from '../resources/types';
  import { errorMessage, formatBytes } from '../utils/format';

  /**
   * The memory budget: what this computer has, what PhotoForge may spend of it, and
   * the one setting that changes it. Every figure comes from the backend, and a figure
   * it could not measure is said to be unmeasured rather than shown as zero.
   */
  let status: ResourceStatus | null = null;
  let loading = true;
  let saving = false;
  let error = '';
  let message = '';
  let choice: 'automatic' | 'manual' = 'automatic';
  let manualBytes = 0;

  $: range = status ? manualRange(status) : null;
  $: warning = status && choice === 'manual' ? manualWarning(status, manualBytes) : null;
  $: unchanged =
    !status ||
    (choice === 'automatic' && status.budget.mode.mode === 'automatic') ||
    (choice === 'manual' && status.budget.mode.mode === 'manual' && status.budget.mode.bytes === manualBytes);

  onMount(() => void refresh());

  function adopt(next: ResourceStatus) {
    status = next;
    choice = next.budget.mode.mode;
    manualBytes = snapBudget(next.budget.mode.mode === 'manual' ? next.budget.mode.bytes : next.budget.bytes, manualRange(next));
  }

  async function refresh() {
    loading = true;
    error = '';
    try {
      adopt(await resourceStatus());
    } catch (reason) {
      error = errorMessage(reason);
    } finally {
      loading = false;
    }
  }

  function typed(event: Event) {
    if (!range) return;
    const value = Number((event.currentTarget as HTMLInputElement).value);
    if (Number.isFinite(value)) manualBytes = snapBudget(value * 1024 ** 3, range);
  }

  async function apply() {
    if (!status || saving || unchanged) return;
    saving = true;
    error = '';
    message = '';
    const requested = choice === 'automatic' ? ({ mode: 'automatic' } as const) : ({ mode: 'manual', bytes: manualBytes } as const);
    try {
      const [change, next] = await setMemoryBudget(requested);
      adopt(next);
      message = change.applied
        ? `Memory budget is now ${formatBytes(change.budget.bytes)}.`
        : `Saved. The budget stays at ${formatBytes(change.budget.bytes)} while this document is open, and the lower figure applies when you next open one.`;
    } catch (reason) {
      error = errorMessage(reason);
    } finally {
      saving = false;
    }
  }
</script>

<section class="resources" aria-labelledby="resources-heading">
  <div class="heading">
    <h2 id="resources-heading">Memory</h2>
    <button type="button" on:click={refresh} disabled={loading || saving}>Refresh</button>
  </div>
  <p class="fine">PhotoForge decides whether an image can be opened whole, as a region, or as a smaller copy from a memory budget. It never reads more of a file than that budget allows.</p>

  {#if loading && !status}
    <p role="status">Measuring…</p>
  {/if}
  {#if error}<p class="problem" role="alert">{error}</p>{/if}

  {#if status && range}
    <dl class="facts" aria-label="This computer">
      <div>
        <dt>Installed memory</dt>
        <dd>{status.system ? formatBytes(status.system.totalPhysical) : 'Could not be measured'}</dd>
      </div>
      <div>
        <dt>Free now</dt>
        <dd>{status.system ? formatBytes(status.system.availablePhysical) : 'Could not be measured'}</dd>
      </div>
      <div>
        <dt>PhotoForge is using</dt>
        <dd>{status.process ? formatBytes(status.process.privateBytes) : 'Could not be measured'}</dd>
      </div>
      <div>
        <dt>Pixels held for this document</dt>
        <dd>{formatBytes(status.residentPixelBytes)}</dd>
      </div>
      <div>
        <dt>Free disk space for the cache</dt>
        <dd>{status.diskFreeBytes === null ? 'Unknown' : formatBytes(status.diskFreeBytes)}</dd>
      </div>
      <div>
        <dt>Video memory</dt>
        <dd>Not measured</dd>
      </div>
    </dl>
    <p class="fine">{status.gpuMemory.reason}</p>

    <fieldset class="mode" disabled={saving}>
      <legend>Memory budget: {formatBytes(status.budget.bytes)}</legend>
      <p class="fine" data-testid="basis">{describeBasis(status.budget)}</p>
      <label class="choice">
        <input type="radio" name="budget-mode" value="automatic" bind:group={choice} />
        <span>Automatic <small>follows this computer's memory</small></span>
      </label>
      <label class="choice">
        <input type="radio" name="budget-mode" value="manual" bind:group={choice} />
        <span>Manual <small>a figure you choose</small></span>
      </label>
      {#if choice === 'manual'}
        <div class="manual">
          <label for="budget-slider">Budget</label>
          <input
            id="budget-slider"
            type="range"
            min={range.min}
            max={range.max}
            step={range.step}
            bind:value={manualBytes}
            aria-valuetext={`${gibibytes(manualBytes)} gigabytes`}
          />
          <div class="number">
            <input
              type="number"
              aria-label="Budget in gigabytes"
              min={gibibytes(range.min)}
              max={gibibytes(range.max)}
              step={gibibytes(range.step)}
              value={gibibytes(manualBytes)}
              on:change={typed}
            />
            <span aria-hidden="true">GB</span>
          </div>
          <p class="fine">From {formatBytes(range.min)} to {formatBytes(range.max)}, the most this computer can support.</p>
          {#if warning}<p class="warn" role="note">{warning}</p>{/if}
          {#if waitsForNextDocument(status, manualBytes)}
            <p class="fine">A lower budget applies when you next open a document, so the one open now is not suddenly judged by limits it was not opened under.</p>
          {/if}
        </div>
      {/if}
      <div class="actions">
        <button type="button" class="primary" on:click={apply} disabled={unchanged || saving}>{saving ? 'Saving…' : 'Apply'}</button>
      </div>
    </fieldset>

    {#if status.reductionPending}
      <p class="warn" role="note">A lower budget is saved but not in force yet. It applies when you next open a document.</p>
    {/if}
    {#if message}<p role="status">{message}</p>{/if}

    <p class="fine" data-testid="limits">{describeLimits(status)}</p>
    <dl class="facts" aria-label="Limits from this budget">
      <div><dt>Largest image opened whole</dt><dd>{megapixels(status.maxWorkingPixels)}</dd></div>
      <div><dt>Largest single job</dt><dd>{formatBytes(status.limits.jobBytes)}</dd></div>
      <div><dt>Pixels kept for editing and undo</dt><dd>{formatBytes(status.limits.storeBytes)}</dd></div>
    </dl>
  {/if}
</section>

<style>
  .resources { display: grid; gap: 10px; font-size: 0.66rem; }
  .heading { display: flex; justify-content: space-between; align-items: center; }
  h2 { margin: 0; font-size: 0.9rem; }
  .facts { display: grid; grid-template-columns: repeat(auto-fit, minmax(190px, 1fr)); gap: 6px 14px; margin: 0; }
  .facts > div { display: grid; gap: 1px; padding: 6px 8px; border: 1px solid var(--line); border-radius: 7px; }
  dt { color: var(--ink-faint); font-size: 0.58rem; }
  dd { margin: 0; color: var(--ink); }
  .mode { display: grid; gap: 8px; padding: 10px 12px; border: 1px solid var(--line); border-radius: 8px; }
  legend { padding: 0 4px; color: var(--ink-soft); }
  .choice { display: flex; gap: 8px; align-items: center; }
  .choice small { margin-left: 4px; color: var(--ink-faint); }
  .manual { display: grid; gap: 6px; margin-left: 22px; }
  .number { display: flex; gap: 6px; align-items: center; }
  input[type='number'] { width: 90px; padding: 4px 6px; border: 1px solid var(--line-strong); border-radius: 6px; color: var(--ink); background: var(--surface-raised); font: inherit; }
  button { padding: 5px 10px; border: 1px solid var(--line-strong); border-radius: 6px; color: var(--ink-soft); background: var(--surface-raised); font-size: 0.62rem; font-weight: 700; cursor: pointer; }
  button.primary { color: #152012; border-color: var(--accent); background: var(--accent); }
  button:disabled { opacity: 0.5; }
  .actions { display: flex; justify-content: flex-end; }
  .fine { margin: 0; color: var(--ink-faint); font-size: 0.6rem; line-height: 1.5; }
  .problem { margin: 0; padding: 8px 10px; border-radius: 7px; background: rgba(224, 121, 90, 0.16); }
  .warn { margin: 0; padding: 8px 10px; border-radius: 7px; background: rgba(200, 170, 80, 0.16); line-height: 1.5; }
</style>
