<script context="module" lang="ts">
  export interface RefineSelectionParameters {
    smooth: number;
    feather: number;
    contrast: number;
    shiftEdge: number;
  }

  export const REFINE_SELECTION_DEFAULTS: Readonly<RefineSelectionParameters> = Object.freeze({
    smooth: 3,
    feather: 2,
    contrast: 0,
    shiftEdge: 0
  });
</script>

<script lang="ts">
  import { onDestroy, onMount } from 'svelte';
  import {
    maskThumbnailCache,
    thumbnailContentKey,
    type MaskThumbnailBitmap
  } from '../selections/thumbnails';
  import type { MaskSnapshot } from '../selections/types';

  type ComparisonMode = 'split' | 'toggle';
  type ToggleView = 'before' | 'after';
  type PreviewBackground = 'original' | 'black' | 'white' | 'mask_only';

  export let originalMask: MaskSnapshot;
  export let previewMask: MaskSnapshot | null;
  export let originalImageUrl: string;
  export let busy = false;
  export let error = '';
  export let parameters: RefineSelectionParameters;
  export let onparameterschange: (parameters: RefineSelectionParameters) => void;
  export let onapply: () => void;
  export let oncancel: () => void;

  let dialog: HTMLDialogElement;
  let beforeCanvas: HTMLCanvasElement;
  let afterCanvas: HTMLCanvasElement;
  let comparisonMode: ComparisonMode = 'split';
  let toggleView: ToggleView = 'after';
  let background: PreviewBackground = 'original';
  let settled = false;
  let sessionKey = '';
  let renderedBeforeKey = '';
  let renderedAfterKey = '';
  let imageRevision = 0;
  let loadedImage: HTMLImageElement | null = null;
  let loadedImageUrl = '';
  let imageRequest = 0;

  $: nextSessionKey = `${originalMask.checksum}:${originalMask.width}x${originalMask.height}`;
  $: if (nextSessionKey !== sessionKey) {
    sessionKey = nextSessionKey;
    settled = false;
  }
  $: beforeRenderKey = `${thumbnailContentKey(originalMask, 240, 180)}:${background}:${imageRevision}`;
  $: afterRenderKey = previewMask
    ? `${thumbnailContentKey(previewMask, 240, 180)}:${background}:${imageRevision}`
    : `empty:${background}:${imageRevision}`;
  $: if (beforeCanvas && beforeRenderKey !== renderedBeforeKey) {
    drawPreview(beforeCanvas, originalMask, background);
    renderedBeforeKey = beforeRenderKey;
  }
  $: if (afterCanvas && afterRenderKey !== renderedAfterKey) {
    if (previewMask) drawPreview(afterCanvas, previewMask, background);
    else clearPreview(afterCanvas);
    renderedAfterKey = afterRenderKey;
  }
  $: if (originalImageUrl !== loadedImageUrl) loadOriginalImage(originalImageUrl);

  onMount(() => {
    openModal();
    dialog?.focus();
  });

  onDestroy(() => {
    imageRequest += 1;
    loadedImage = null;
    closeModal();
  });

  function updateParameter(
    key: keyof RefineSelectionParameters,
    input: number,
    minimum: number,
    maximum: number,
    integer = false
  ) {
    if (!Number.isFinite(input)) return;
    const bounded = Math.max(minimum, Math.min(maximum, integer ? Math.round(input) : input));
    onparameterschange({ ...parameters, [key]: bounded });
  }

  function resetParameters() {
    if (busy) return;
    onparameterschange({ ...REFINE_SELECTION_DEFAULTS });
  }

  function apply() {
    if (busy || !previewMask || settled) return;
    settled = true;
    closeModal();
    onapply();
  }

  function cancel() {
    if (settled) return;
    settled = true;
    closeModal();
    oncancel();
  }

  function openModal() {
    if (!dialog || dialog.open) return;
    try {
      if (typeof dialog.showModal === 'function') dialog.showModal();
      else dialog.setAttribute('open', '');
    } catch {
      dialog.setAttribute('open', '');
    }
  }

  function closeModal() {
    if (!dialog?.open) return;
    try {
      if (typeof dialog.close === 'function') dialog.close();
      else dialog.removeAttribute('open');
    } catch {
      dialog.removeAttribute('open');
    }
  }

  function handleDialogCancel(event: Event) {
    event.preventDefault();
    cancel();
  }

  function handleKeyboard(event: KeyboardEvent) {
    if (event.key === 'Escape') {
      event.preventDefault();
      cancel();
      return;
    }
    if (event.key !== 'Enter') return;
    const target = event.target;
    if (
      target instanceof HTMLElement &&
      target.matches('input, textarea, select, button, [contenteditable="true"]')
    ) {
      return;
    }
    if (!busy && previewMask && !settled) {
      event.preventDefault();
      apply();
    }
  }

  function loadOriginalImage(url: string) {
    loadedImageUrl = url;
    loadedImage = null;
    imageRevision += 1;
    const ownRequest = ++imageRequest;
    if (!/^(data:image\/|blob:)/i.test(url)) return;
    const image = new Image();
    image.onload = () => {
      if (ownRequest !== imageRequest || loadedImageUrl !== url) return;
      loadedImage = image;
      imageRevision += 1;
    };
    image.onerror = () => {
      if (ownRequest === imageRequest) loadedImage = null;
    };
    image.src = url;
  }

  function drawPreview(canvas: HTMLCanvasElement, mask: MaskSnapshot, mode: PreviewBackground) {
    try {
      const thumbnail = maskThumbnailCache.get(mask, 240, 180);
      canvas.width = thumbnail.width;
      canvas.height = thumbnail.height;
      const context = canvas.getContext('2d');
      if (!context) return;
      const source = loadedImage ? imagePixels(loadedImage, thumbnail.width, thumbnail.height) : null;
      const output = previewPixels(thumbnail, mode, source);
      const image = context.createImageData(thumbnail.width, thumbnail.height);
      image.data.set(output);
      context.putImageData(image, 0, 0);
    } catch {
      clearPreview(canvas);
    }
  }

  function imagePixels(image: HTMLImageElement, width: number, height: number): Uint8ClampedArray | null {
    try {
      const scratch = document.createElement('canvas');
      scratch.width = width;
      scratch.height = height;
      const context = scratch.getContext('2d');
      if (!context) return null;
      context.drawImage(image, 0, 0, width, height);
      return context.getImageData(0, 0, width, height).data;
    } catch {
      return null;
    }
  }

  function previewPixels(
    thumbnail: MaskThumbnailBitmap,
    mode: PreviewBackground,
    source: Uint8ClampedArray | null
  ): Uint8ClampedArray {
    const output = new Uint8ClampedArray(thumbnail.pixels.length);
    for (let index = 0; index < thumbnail.pixels.length; index += 4) {
      const coverage = thumbnail.pixels[index];
      const alpha = coverage / 255;
      const sourceRed = source?.[index] ?? 204;
      const sourceGreen = source?.[index + 1] ?? 204;
      const sourceBlue = source?.[index + 2] ?? 204;
      if (mode === 'mask_only' || (mode === 'original' && !source)) {
        output.set([coverage, coverage, coverage, 255], index);
      } else if (mode === 'original') {
        const overlay = 0.38 * alpha;
        output.set([
          Math.round(sourceRed * (1 - overlay) + 239 * overlay),
          Math.round(sourceGreen * (1 - overlay) + 91 * overlay),
          Math.round(sourceBlue * (1 - overlay) + 91 * overlay),
          255
        ], index);
      } else {
        const backgroundChannel = mode === 'white' ? 255 : 0;
        output.set([
          Math.round(sourceRed * alpha + backgroundChannel * (1 - alpha)),
          Math.round(sourceGreen * alpha + backgroundChannel * (1 - alpha)),
          Math.round(sourceBlue * alpha + backgroundChannel * (1 - alpha)),
          255
        ], index);
      }
    }
    return output;
  }

  function clearPreview(canvas: HTMLCanvasElement) {
    const context = canvas.getContext('2d');
    context?.clearRect(0, 0, canvas.width, canvas.height);
  }
</script>

<svelte:window on:keydown={handleKeyboard} />

<div class="backdrop" role="presentation">
  <dialog
    bind:this={dialog}
    class="dialog"
    aria-modal="true"
    aria-labelledby="refine-selection-title"
    aria-describedby={error ? 'refine-selection-error' : undefined}
    aria-busy={busy}
    tabindex="-1"
    on:cancel={handleDialogCancel}
  >
    <header>
      <div><span>◐</span><h2 id="refine-selection-title">Refine Selection</h2></div>
      <button type="button" aria-label="Cancel refinement" disabled={settled} on:click={cancel}>×</button>
    </header>

    <div class="preview-toolbar" aria-label="Refinement preview controls">
      <div class="segmented" aria-label="Comparison mode">
        <button type="button" class:active={comparisonMode === 'split'} aria-pressed={comparisonMode === 'split'} on:click={() => (comparisonMode = 'split')}>Split</button>
        <button type="button" class:active={comparisonMode === 'toggle'} aria-pressed={comparisonMode === 'toggle'} on:click={() => (comparisonMode = 'toggle')}>Toggle</button>
      </div>
      {#if comparisonMode === 'toggle'}
        <div class="segmented" aria-label="Toggle preview">
          <button type="button" class:active={toggleView === 'before'} aria-pressed={toggleView === 'before'} on:click={() => (toggleView = 'before')}>Show before</button>
          <button type="button" class:active={toggleView === 'after'} aria-pressed={toggleView === 'after'} on:click={() => (toggleView = 'after')}>Show after</button>
        </div>
      {/if}
      <div class="backgrounds" aria-label="Preview background">
        {#each [['original', 'Original'], ['black', 'Black'], ['white', 'White'], ['mask_only', 'Mask only']] as option}
          <button
            type="button"
            class:active={background === option[0]}
            aria-pressed={background === option[0]}
            on:click={() => (background = option[0] as PreviewBackground)}
          >{option[1]}</button>
        {/each}
      </div>
    </div>

    <div class:single={comparisonMode === 'toggle'} class="preview-grid" data-background={background} aria-live="polite">
      <article class:hidden={comparisonMode === 'toggle' && toggleView !== 'before'} class="preview-card before">
        <strong>Before</strong>
        <div class="canvas-frame" role="img" aria-label="Selection before refinement">
          <canvas bind:this={beforeCanvas} aria-hidden="true"></canvas>
        </div>
      </article>
      <article class:hidden={comparisonMode === 'toggle' && toggleView !== 'after'} class="preview-card after">
        <strong>After</strong>
        <div class="canvas-frame" role="img" aria-label="Selection after refinement">
          <canvas bind:this={afterCanvas} aria-hidden="true"></canvas>
        </div>
        {#if !previewMask}<span class="preview-pending">{busy ? 'Updating preview…' : 'Preview unavailable'}</span>{/if}
      </article>
    </div>

    <div class="controls">
      <label>
        <span>Smooth <output>{parameters.smooth}px</output></span>
        <input aria-label="Refine smooth" type="range" min="0" max="128" step="1" value={parameters.smooth} disabled={busy || settled} on:input={(event) => updateParameter('smooth', Number(event.currentTarget.value), 0, 128, true)} />
      </label>
      <label>
        <span>Feather <output>{parameters.feather}px</output></span>
        <input aria-label="Refine feather" type="range" min="0" max="256" step="1" value={parameters.feather} disabled={busy || settled} on:input={(event) => updateParameter('feather', Number(event.currentTarget.value), 0, 256, true)} />
      </label>
      <label>
        <span>Contrast <output>{Math.round(parameters.contrast * 100)}%</output></span>
        <input aria-label="Refine contrast" type="range" min="-1" max="1" step="0.05" value={parameters.contrast} disabled={busy || settled} on:input={(event) => updateParameter('contrast', Number(event.currentTarget.value), -1, 1)} />
      </label>
      <label>
        <span>Shift edge <output>{parameters.shiftEdge}px</output></span>
        <input aria-label="Refine shift edge" type="range" min="-256" max="256" step="1" value={parameters.shiftEdge} disabled={busy || settled} on:input={(event) => updateParameter('shiftEdge', Number(event.currentTarget.value), -256, 256, true)} />
      </label>
    </div>

    {#if error}<p id="refine-selection-error" class="error" role="alert">{error}</p>{/if}

    <footer>
      <button type="button" disabled={busy || settled} on:click={resetParameters}>Reset</button>
      <span></span>
      <button type="button" disabled={settled} on:click={cancel}>Cancel</button>
      <button class="primary" type="button" aria-label="Apply" disabled={busy || !previewMask || settled} on:click={apply}>{busy ? 'Updating…' : 'Apply'}</button>
    </footer>
  </dialog>
</div>

<style>
  .backdrop { display: contents; }
  .dialog { gap: 12px; width: min(820px, 94vw); max-height: 92vh; overflow: auto; margin: auto; padding: 14px; border: 1px solid var(--line-strong); border-radius: 12px; color: var(--ink); background: var(--surface); box-shadow: 0 28px 90px rgba(0,0,0,.65); }
  .dialog[open] { display: grid; }
  .dialog:not([open]) { display: none; }
  .dialog::backdrop { background: rgba(6, 7, 6, .78); backdrop-filter: blur(5px); }
  .dialog:focus { outline: none; }
  header, header > div, footer, .preview-toolbar, .segmented, .backgrounds, label > span { display: flex; align-items: center; gap: 7px; }
  header { justify-content: space-between; }
  header h2 { margin: 0; font-size: .88rem; }
  header span { color: var(--accent); font-size: 1.1rem; }
  button { border: 1px solid var(--line); border-radius: 6px; padding: 7px 9px; color: var(--ink-soft); background: var(--surface-raised); font: inherit; cursor: pointer; }
  button:hover:not(:disabled), button.active { border-color: var(--accent); color: var(--accent-bright); }
  button:focus-visible, input:focus-visible { outline: 2px solid var(--accent); outline-offset: 2px; }
  button:disabled, input:disabled { opacity: .42; cursor: not-allowed; }
  .preview-toolbar { flex-wrap: wrap; justify-content: space-between; }
  .segmented, .backgrounds { gap: 3px; }
  .segmented button, .backgrounds button { padding: 6px 8px; font-size: .58rem; }
  .preview-grid { display: grid; grid-template-columns: 1fr 1fr; gap: 10px; min-height: 220px; }
  .preview-grid.single { grid-template-columns: 1fr; }
  .preview-card { position: relative; display: grid; place-items: center; min-width: 0; min-height: 220px; overflow: hidden; border: 1px solid var(--line); border-radius: 8px; background: #151713; }
  .preview-card.hidden { display: none; }
  .preview-card strong { position: absolute; z-index: 2; top: 8px; left: 8px; padding: 4px 6px; border-radius: 4px; color: white; background: rgba(0,0,0,.58); font-size: .58rem; }
  .canvas-frame { display: grid; place-items: center; max-width: 100%; max-height: 320px; }
  .preview-card canvas { display: block; max-width: 100%; max-height: 320px; }
  .preview-pending { position: absolute; color: var(--ink-faint); font-size: .65rem; }
  .controls { display: grid; grid-template-columns: 1fr 1fr; gap: 10px; padding: 10px; border: 1px solid var(--line); border-radius: 8px; background: var(--surface-soft); }
  .controls label { display: grid; gap: 5px; color: var(--ink-soft); font-size: .63rem; }
  .controls label > span { justify-content: space-between; }
  .controls output { color: var(--ink); font-family: var(--font-mono); }
  .controls input { width: 100%; }
  .error { margin: 0; padding: 8px; border-radius: 6px; color: #f0a5a0; background: rgba(190,70,60,.12); font-size: .66rem; }
  footer { display: grid; grid-template-columns: auto 1fr auto auto; }
  footer .primary { border-color: var(--accent); color: #172014; background: var(--accent); font-weight: 800; }
  @media (max-width: 650px) { .preview-grid, .controls { grid-template-columns: 1fr; } .preview-card { min-height: 170px; } }
</style>
