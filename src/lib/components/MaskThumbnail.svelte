<script lang="ts">
  import { onDestroy, onMount } from 'svelte';
  import {
    maskThumbnailCache,
    thumbnailContentKey,
    type MaskThumbnailCache
  } from '../selections/thumbnails';
  import type { MaskSnapshot } from '../selections/types';

  export let mask: MaskSnapshot;
  export let label: string;
  export let targetWidth = 40;
  export let targetHeight = 32;
  export let cache: MaskThumbnailCache = maskThumbnailCache;

  let host: HTMLSpanElement;
  let canvas: HTMLCanvasElement;
  let visible = false;
  let failed = false;
  let renderedKey = '';
  let pendingKey = '';
  let observer: IntersectionObserver | undefined;
  let timeout: ReturnType<typeof setTimeout> | undefined;
  let idleHandle: number | undefined;
  let generation = 0;
  let destroyed = false;

  $: contentKey = thumbnailContentKey(mask, targetWidth, targetHeight);
  $: if (visible && contentKey !== renderedKey && contentKey !== pendingKey) {
    scheduleRender(contentKey);
  }

  onMount(() => {
    if (typeof IntersectionObserver === 'function') {
      observer = new IntersectionObserver((entries) => {
        if (entries.some((entry) => entry.isIntersecting || entry.intersectionRatio > 0)) {
          visible = true;
          observer?.disconnect();
          observer = undefined;
        }
      });
      observer.observe(host);
    } else {
      visible = true;
    }
  });

  onDestroy(() => {
    destroyed = true;
    generation += 1;
    observer?.disconnect();
    cancelScheduledRender();
  });

  function scheduleRender(expectedKey: string) {
    cancelScheduledRender();
    const ownGeneration = ++generation;
    pendingKey = expectedKey;
    failed = false;
    const render = () => {
      timeout = undefined;
      idleHandle = undefined;
      if (destroyed || ownGeneration !== generation || expectedKey !== contentKey) return;
      try {
        const thumbnail = cache.get(mask, targetWidth, targetHeight);
        const context = canvas.getContext('2d');
        if (!context) throw new Error('Canvas rendering is unavailable.');
        canvas.width = thumbnail.width;
        canvas.height = thumbnail.height;
        const image = context.createImageData(thumbnail.width, thumbnail.height);
        image.data.set(thumbnail.pixels);
        context.putImageData(image, 0, 0);
        renderedKey = expectedKey;
      } catch {
        failed = true;
        renderedKey = expectedKey;
      } finally {
        if (ownGeneration === generation) pendingKey = '';
      }
    };

    if (typeof window.requestIdleCallback === 'function') {
      idleHandle = window.requestIdleCallback(render, { timeout: 200 });
    } else {
      timeout = setTimeout(render, 0);
    }
  }

  function cancelScheduledRender() {
    if (timeout !== undefined) clearTimeout(timeout);
    if (idleHandle !== undefined && typeof window.cancelIdleCallback === 'function') {
      window.cancelIdleCallback(idleHandle);
    }
    timeout = undefined;
    idleHandle = undefined;
  }
</script>

<span
  bind:this={host}
  class="mask-thumbnail"
  class:failed
  role="img"
  aria-label={`Mask thumbnail for ${label}`}
  aria-busy={visible && renderedKey !== contentKey}
  style={`--thumbnail-width:${targetWidth}px;--thumbnail-height:${targetHeight}px`}
>
  <canvas bind:this={canvas} aria-hidden="true"></canvas>
  {#if failed}<span class="fallback" aria-hidden="true">!</span>{/if}
</span>

<style>
  .mask-thumbnail {
    display: grid;
    place-items: center;
    width: var(--thumbnail-width);
    height: var(--thumbnail-height);
    overflow: hidden;
    border: 1px solid rgba(255, 255, 255, .1);
    border-radius: 5px;
    background: repeating-conic-gradient(#252823 0 25%, #1b1d1a 0 50%) 50% / 8px 8px;
  }

  canvas {
    display: block;
    max-width: 100%;
    max-height: 100%;
  }

  .failed {
    color: #e19a91;
    background: var(--surface-raised);
  }

  .fallback {
    font-weight: 800;
  }
</style>
