<script lang="ts">
  import { onDestroy, onMount } from 'svelte';
  import RegionSelector from './RegionSelector.svelte';
  import { errorMessage, formatBytes } from '../utils/format';
  import { cancelSourcePreview, sourcePreviewImage } from '../source/commands';
  import { reducedSize } from '../source/region';
  import {
    optionOf,
    type OpenSelection,
    type Rect,
    type SourceAdmission,
    type SourcePreview
  } from '../source/types';

  export let admission: SourceAdmission;
  export let onopen: (selection: OpenSelection) => void = () => {};
  export let oncancel: () => void = () => {};
  /** Injectable so the dialog can be driven without a backend. */
  export let loadPreview: (path: string, maxEdge: number) => Promise<SourcePreview> = sourcePreviewImage;
  export let cancelPreview: () => Promise<void> = cancelSourcePreview;
  /**
   * Go straight to choosing a region, with this one selected. Used by "Change
   * Source Region", where the user already has a region and wants a different one.
   */
  export let startInRegion = false;
  export let initialRect: Rect | null = null;

  type Phase = 'choose' | 'region' | 'reduced';
  let phase: Phase = 'choose';
  let previewState: 'loading' | 'ready' | 'unavailable' = 'loading';
  let preview: SourcePreview | null = null;
  let previewMessage = '';
  let previewRequest = 0;
  let scaleChoice = 'fit';
  let dialogElement: HTMLDialogElement;

  $: report = admission.report;
  $: regionOption = optionOf(report, 'openRegion');
  $: reducedOption = optionOf(report, 'openReduced');
  $: sourceMp = (admission.width * admission.height) / 1_000_000;

  /** The scales on offer: the largest that fits, and round ones at or below it. */
  $: scales = (() => {
    if (!reducedOption) return [] as { id: string; label: string; scale: number }[];
    const max = reducedOption.maxScale;
    const list = [{ id: 'fit', label: `Largest that fits (${Math.floor(max * 100)}%)`, scale: max }];
    for (const percent of [50, 25, 10]) {
      if (percent / 100 < max - 1e-9) list.push({ id: String(percent), label: `${percent}%`, scale: percent / 100 });
    }
    return list;
  })();
  $: chosen = scales.find((entry) => entry.id === scaleChoice) ?? scales[0] ?? null;
  $: reduced = chosen ? reducedSize(admission.width, admission.height, chosen.scale) : null;

  function explain(): string {
    const verdict = report.verdict;
    const needs = formatBytes(report.fullPeakBytes);
    const budget = formatBytes(report.budgetBytes);
    switch (verdict.kind) {
      case 'regionRequired':
        return `Opening it whole would need about ${needs} of memory, and PhotoForge may use ${budget} on this machine. You can open a part of it at full resolution, or a smaller copy of all of it.`;
      case 'reducedCopyRecommended':
        return `Opening it whole would need about ${needs}, and PhotoForge may use ${budget}. This format cannot be opened a part at a time on this machine, but a smaller copy of all of it can.`;
      case 'insufficientResources':
        return verdict.shortfall === 'momentary'
          ? `It would fit PhotoForge's ${budget} budget, but there is not enough free memory right now${
              report.availableBytes === null ? '' : ` (${formatBytes(report.availableBytes)} free)`
            }. Closing other applications and trying again would change that.`
          : `No way of opening this file that stays within PhotoForge's ${budget} memory budget is available. Raising the budget in Settings may help.`;
      case 'unsafe':
        switch (verdict.refusal.kind) {
          case 'empty':
            return 'This file describes an image with no pixels.';
          case 'implausibleDimensions':
            return `This file claims dimensions (${verdict.refusal.width.toLocaleString('en-US')} × ${verdict.refusal.height.toLocaleString('en-US')}) that no image can have.`;
          default:
            return 'This file claims far more image data than its size could possibly hold, so it is not a valid image and was not opened.';
        }
      default:
        return '';
    }
  }

  async function enterRegion() {
    phase = 'region';
    previewState = 'loading';
    preview = null;
    previewMessage = '';
    const own = ++previewRequest;
    try {
      const made = await loadPreview(admission.path, 1400);
      if (own !== previewRequest) return;
      preview = made;
      previewState = 'ready';
    } catch (error) {
      if (own !== previewRequest) return;
      previewState = 'unavailable';
      previewMessage = errorMessage(error);
    }
  }

  function leaveRegion() {
    // Anything still decoding holds the CPU job gate, so stop it rather than
    // leave it to finish for a screen nobody is looking at.
    previewRequest += 1;
    void cancelPreview().catch(() => undefined);
    phase = 'choose';
  }

  function cancel() {
    previewRequest += 1;
    void cancelPreview().catch(() => undefined);
    oncancel();
  }

  function confirmRegion(rect: Rect) {
    previewRequest += 1;
    onopen({ kind: 'region', rect });
  }

  function confirmReduced() {
    if (reduced) onopen({ kind: 'reduced', width: reduced.width, height: reduced.height });
  }

  function keys(event: KeyboardEvent) {
    if (event.key === 'Escape') {
      event.preventDefault();
      event.stopPropagation();
      phase === 'region' ? leaveRegion() : cancel();
    }
  }

  // The first thing on offer is the safest to land on: Enter does what the
  // interface recommends, and Escape still cancels.
  onMount(() => {
    if (startInRegion && optionOf(report, 'openRegion')) {
      void enterRegion();
      return;
    }
    (dialogElement?.querySelector('.choice, .modal-actions button') as HTMLElement | null)?.focus();
  });
  onDestroy(() => {
    previewRequest += 1;
  });
</script>

<div class="modal-backdrop" role="presentation">
  <dialog
    open
    bind:this={dialogElement}
    class={`modal oversized-modal ${phase === 'region' ? 'wide' : ''}`}
    aria-labelledby="oversized-title"
    on:keydown={keys}
  >
    <div class="modal-heading">
      <div>
        <span>Large image</span>
        <h1 id="oversized-title">
          {phase === 'region'
            ? 'Choose a region'
            : phase === 'reduced'
              ? 'Open a reduced copy'
              : 'This image is too large to open whole'}
        </h1>
      </div>
      <button type="button" aria-label="Cancel opening this image" on:click={cancel}>×</button>
    </div>

    {#if phase === 'choose'}
      <div class="body">
        <dl class="facts">
          <dt>File</dt>
          <dd>{admission.filename}</dd>
          <dt>Size</dt>
          <dd>
            {admission.width.toLocaleString('en-US')} × {admission.height.toLocaleString('en-US')} px ·
            {sourceMp.toFixed(1)} MP · {formatBytes(admission.fileBytes)} on disk
          </dd>
        </dl>
        <p class="explain" data-testid="explanation">{explain()}</p>

        <div class="choices">
          {#if regionOption}
            <button type="button" class="choice primary" on:click={enterRegion}>
              <strong>Choose Region…</strong>
              <span>
                Open part of it at full resolution — up to
                {(regionOption.maxRegionPixels / 1_000_000).toFixed(1)} MP.
                {#if regionOption.decode.kind === 'transientFull'}
                  This format has to be read whole to cut a region out, so opening takes as long as reading the file.
                {:else if regionOption.decode.kind === 'segments'}
                  Only the strips or tiles that cover the region are decoded. The compressed file is still read
                  whole, and a camera RAW has no reduced copy.
                {/if}
              </span>
            </button>
          {/if}
          {#if reducedOption}
            <button type="button" class="choice" on:click={() => (phase = 'reduced')}>
              <strong>Open Reduced Copy…</strong>
              <span>Open all of it at lower resolution, up to {Math.floor(reducedOption.maxScale * 100)}% of its size.</span>
            </button>
          {/if}
        </div>

        {#if !regionOption && !reducedOption}
          <p class="none" role="status" data-testid="no-options">No way of opening this file is available.</p>
        {/if}

        <details class="why">
          <summary>Why not open it at full resolution, out of core?</summary>
          <p>{report.outOfCore}</p>
        </details>

        <div class="modal-actions">
          <button type="button" on:click={cancel}>Cancel</button>
        </div>
      </div>
    {:else if phase === 'region'}
      {#if startInRegion}
        <p class="change-note" data-testid="change-note">
          This opens the file again as a new document at the region you choose. Edits made to the
          current document are not carried across, and you will be asked before it is replaced.
        </p>
      {/if}
      <RegionSelector
        initial={initialRect}
        {admission}
        {previewState}
        {preview}
        {previewMessage}
        onconfirm={confirmRegion}
        oncancel={cancel}
        onback={leaveRegion}
      />
    {:else}
      <div class="body">
        <p class="explain">
          A reduced copy has <strong>less resolution than the file</strong>. The original is not changed, and the
          document records that it is a reduced copy of it.
        </p>
        <fieldset class="scales">
          <legend>Size</legend>
          {#each scales as entry (entry.id)}
            <label>
              <input type="radio" name="scale" value={entry.id} bind:group={scaleChoice} />
              {entry.label}
            </label>
          {/each}
        </fieldset>
        {#if reduced}
          <p class="result" data-testid="reduced-size">
            Result: {reduced.width.toLocaleString('en-US')} × {reduced.height.toLocaleString('en-US')} px ·
            {((reduced.width * reduced.height) / 1_000_000).toFixed(1)} MP
          </p>
        {/if}
        <div class="modal-actions">
          <button type="button" on:click={() => (phase = 'choose')}>Back</button>
          <button type="button" on:click={cancel}>Cancel</button>
          <button type="button" class="primary" disabled={!reduced} on:click={confirmReduced}>Open reduced copy</button>
        </div>
      </div>
    {/if}
  </dialog>
</div>

<style>
  .oversized-modal { width: min(560px, 100%); }
  .oversized-modal.wide { width: min(1040px, 100%); }
  .body { display: grid; gap: 14px; padding: 16px 21px 21px; }
  .facts { display: grid; grid-template-columns: auto 1fr; gap: 4px 12px; margin: 0; font-size: 0.68rem; }
  .facts dt { color: var(--ink-faint); }
  .facts dd { margin: 0; color: var(--ink); }
  .change-note { margin: 0 0 8px; color: var(--ink-soft); font-size: 0.66rem; line-height: 1.5; }
  .explain { margin: 0; color: var(--ink-soft); font-size: 0.68rem; line-height: 1.55; }
  .choices { display: grid; gap: 8px; }
  .choice { display: grid; gap: 4px; padding: 12px 14px; border: 1px solid var(--line-strong); border-radius: 9px; color: var(--ink); background: var(--surface-raised); text-align: left; cursor: pointer; }
  .choice.primary { border-color: var(--accent); }
  .choice strong { font-size: 0.74rem; }
  .choice span { color: var(--ink-soft); font-size: 0.64rem; line-height: 1.45; }
  .choice:focus-visible { outline: 2px solid var(--accent); outline-offset: 2px; }
  .none { margin: 0; padding: 10px; border-radius: 7px; background: rgba(224, 121, 90, 0.16); font-size: 0.68rem; }
  .why { color: var(--ink-faint); font-size: 0.62rem; line-height: 1.5; }
  .why summary { cursor: pointer; }
  .scales { display: grid; gap: 8px; margin: 0; padding: 10px 12px; border: 1px solid var(--line-strong); border-radius: 8px; font-size: 0.68rem; }
  .scales legend { padding: 0 6px; color: var(--ink-faint); }
  .scales label { display: flex; gap: 8px; align-items: center; cursor: pointer; }
  .result { margin: 0; font-size: 0.68rem; font-variant-numeric: tabular-nums; }
</style>
