<script lang="ts">
  import { onDestroy, onMount } from 'svelte';
  import { drawMaskOverlay } from '../selections/overlays';
  import type {
    MaskSnapshot,
    OverlaySettings,
    Point,
    SelectionGesture,
    SelectionTool
  } from '../selections/types';

  export let originalUrl: string | null;
  export let previewUrl: string | null;
  export let filename = '';
  export let comparison = false;
  export let comparisonPosition = 50;
  export let comparisonMode: 'swipe' | 'split' | 'blink' | 'difference' = 'swipe';
  export let splitComparison = false;
  export let gridOverlay = false;
  export let crosshair = false;
  export let zoom = 100;
  export let processing = false;
  export let stale = false;
  export let onopen: () => void;
  export let oncomparisonchange: (value: number) => void;
  export let imageWidth = 0;
  export let imageHeight = 0;
  export let selectionTool: SelectionTool = 'none';
  export let activeMask: MaskSnapshot | null = null;
  export let overlaySettings: OverlaySettings = {
    visible: false,
    mode: 'color',
    opacity: 0.4,
    color: '#ef5b5b'
  };
  export let brushDiameter = 48;
  export let fixedAspect = false;
  export let fromCenter = false;
  export let onselectiongesture: (gesture: SelectionGesture) => void = () => undefined;
  export let onselectioncancel: () => void = () => undefined;

  let processedImage: HTMLImageElement;
  let overlayCanvas: HTMLCanvasElement;
  let interactionLayer: HTMLButtonElement;
  let dragging = false;
  let dragPoints: Point[] = [];
  let polygonPoints: Point[] = [];
  let hoverPoint: Point | null = null;
  let gestureShift = false;
  let gestureAlt = false;
  let antFrame = 0;
  let antTimer: ReturnType<typeof setInterval> | undefined;

  $: overlayKey = `${previewUrl}:${activeMask?.checksum ?? ''}:${JSON.stringify(overlaySettings)}:${antFrame}`;
  $: if (overlayKey && overlayCanvas && processedImage) redrawOverlay();
  $: if (selectionTool !== 'polygon' && polygonPoints.length) polygonPoints = [];
  $: previewBounds = dragPoints.length >= 2 ? selectionBounds(dragPoints[0], dragPoints.at(-1) as Point) : null;

  onMount(() => {
    antTimer = setInterval(() => {
      if (overlaySettings.visible && overlaySettings.mode === 'marching_ants') antFrame = (antFrame + 1) % 8;
    }, 140);
  });

  onDestroy(() => {
    if (antTimer) clearInterval(antTimer);
  });

  function redrawOverlay() {
    if (!processedImage?.naturalWidth || !overlayCanvas) return;
    try {
      drawMaskOverlay(
        overlayCanvas,
        activeMask,
        overlaySettings,
        processedImage.naturalWidth,
        processedImage.naturalHeight,
        antFrame
      );
    } catch {
      const context = overlayCanvas.getContext('2d');
      context?.clearRect(0, 0, overlayCanvas.width, overlayCanvas.height);
    }
  }

  function imagePoint(event: PointerEvent | MouseEvent): Point {
    const bounds = interactionLayer.getBoundingClientRect();
    return {
      x: Math.max(0, Math.min(imageWidth, ((event.clientX - bounds.left) / bounds.width) * imageWidth)),
      y: Math.max(0, Math.min(imageHeight, ((event.clientY - bounds.top) / bounds.height) * imageHeight))
    };
  }

  function handlePointerDown(event: PointerEvent) {
    if (selectionTool === 'none' || selectionTool === 'polygon' || !imageWidth || !imageHeight) return;
    event.preventDefault();
    interactionLayer.focus();
    const point = imagePoint(event);
    hoverPoint = point;
    gestureShift = event.shiftKey;
    gestureAlt = event.altKey;
    if (selectionTool === 'magic_wand' || selectionTool === 'color_range') {
      onselectiongesture({ tool: selectionTool, points: [point], shiftKey: event.shiftKey, altKey: event.altKey });
      return;
    }
    dragging = true;
    dragPoints = [point];
    interactionLayer.setPointerCapture(event.pointerId);
  }

  function handlePointerMove(event: PointerEvent) {
    if (!imageWidth || !imageHeight) return;
    const point = imagePoint(event);
    hoverPoint = point;
    if (!dragging) return;
    if (selectionTool === 'rectangle' || selectionTool === 'ellipse') {
      dragPoints = [dragPoints[0], point];
    } else {
      const previous = dragPoints.at(-1) as Point;
      const minimum = Math.max(0.35, selectionTool === 'brush' || selectionTool === 'eraser' ? brushDiameter * 0.04 : 0.75);
      if (Math.hypot(point.x - previous.x, point.y - previous.y) >= minimum) {
        dragPoints = [...dragPoints, point];
      }
    }
  }

  function handlePointerUp(event: PointerEvent) {
    if (!dragging) return;
    const point = imagePoint(event);
    const points =
      selectionTool === 'rectangle' || selectionTool === 'ellipse'
        ? pointsFromBounds(selectionBounds(dragPoints[0], point))
        : [...dragPoints, point];
    dragging = false;
    dragPoints = [];
    interactionLayer.releasePointerCapture(event.pointerId);
    if (points.length >= (selectionTool === 'freehand' ? 3 : 1)) {
      onselectiongesture({
        tool: selectionTool,
        points,
        shiftKey: gestureShift || event.shiftKey,
        altKey: gestureAlt || event.altKey
      });
    }
  }

  function handlePolygonClick(event: MouseEvent) {
    if (selectionTool !== 'polygon' || event.detail === 0) return;
    event.preventDefault();
    interactionLayer.focus();
    if (event.detail >= 2) {
      if (polygonPoints.length >= 3) {
        onselectiongesture({
          tool: 'polygon',
          points: polygonPoints,
          shiftKey: event.shiftKey,
          altKey: event.altKey
        });
        polygonPoints = [];
      }
      return;
    }
    polygonPoints = [...polygonPoints, imagePoint(event)];
  }

  function handleSelectionKey(event: KeyboardEvent) {
    if (selectionTool !== 'polygon') {
      if (event.key === 'Escape' && dragging) cancelGesture();
      return;
    }
    if (event.key === 'Enter' && polygonPoints.length >= 3) {
      event.preventDefault();
      onselectiongesture({ tool: 'polygon', points: polygonPoints, shiftKey: event.shiftKey, altKey: event.altKey });
      polygonPoints = [];
    } else if (event.key === 'Backspace' && polygonPoints.length) {
      event.preventDefault();
      polygonPoints = polygonPoints.slice(0, -1);
    } else if (event.key === 'Escape') {
      event.preventDefault();
      polygonPoints = [];
      onselectioncancel();
    }
  }

  function cancelGesture() {
    dragging = false;
    dragPoints = [];
    polygonPoints = [];
    onselectioncancel();
  }

  function selectionBounds(start: Point, end: Point): { left: number; top: number; right: number; bottom: number } {
    let deltaX = end.x - start.x;
    let deltaY = end.y - start.y;
    if (fixedAspect) {
      const size = Math.max(Math.abs(deltaX), Math.abs(deltaY));
      deltaX = Math.sign(deltaX || 1) * size;
      deltaY = Math.sign(deltaY || 1) * size;
    }
    if (fromCenter) {
      return {
        left: Math.max(0, start.x - Math.abs(deltaX)),
        top: Math.max(0, start.y - Math.abs(deltaY)),
        right: Math.min(imageWidth, start.x + Math.abs(deltaX)),
        bottom: Math.min(imageHeight, start.y + Math.abs(deltaY))
      };
    }
    return {
      left: Math.max(0, Math.min(start.x, start.x + deltaX)),
      top: Math.max(0, Math.min(start.y, start.y + deltaY)),
      right: Math.min(imageWidth, Math.max(start.x, start.x + deltaX)),
      bottom: Math.min(imageHeight, Math.max(start.y, start.y + deltaY))
    };
  }

  function pointsFromBounds(bounds: { left: number; top: number; right: number; bottom: number }): Point[] {
    return [
      { x: bounds.left, y: bounds.top },
      { x: bounds.right, y: bounds.bottom }
    ];
  }
</script>

<section class="stage" aria-label="Image preview">
  {#if previewUrl}
    <div class="canvas-shell" class:processing class:stale>
      {#if comparison && originalUrl && splitComparison}
        <div class="split-canvas" style={`--zoom: ${zoom / 100}`}>
          <figure>
            <img src={originalUrl} alt={`Original preview of ${filename}`} draggable="false" />
            <figcaption>Before · original orientation</figcaption>
          </figure>
          <figure>
            <img src={previewUrl} alt={`Edited preview of ${filename}`} draggable="false" />
            <figcaption>After · transformed</figcaption>
          </figure>
        </div>
      {:else}
        <div class="canvas" class:blink={comparison && comparisonMode === 'blink'} class:difference={comparison && comparisonMode === 'difference'} style={`--zoom: ${zoom / 100}`}>
          <img bind:this={processedImage} class="processed" src={previewUrl} alt={`Edited preview of ${filename}`} draggable="false" on:load={redrawOverlay} />
          {#if comparison && originalUrl}
            <div class="before" style={`width: ${comparisonMode === 'swipe' ? comparisonPosition : 100}%`}>
              <img src={originalUrl} alt={`Original preview of ${filename}`} draggable="false" />
            </div>
            {#if comparisonMode === 'swipe'}<div class="divider" style={`left: ${comparisonPosition}%`} aria-hidden="true"><span>↔</span></div>{/if}
            <span class="badge before-badge">Before</span>
            <span class="badge after-badge">After</span>
          {/if}
          <canvas bind:this={overlayCanvas} class="mask-overlay" aria-hidden="true"></canvas>
          {#if selectionTool !== 'none' && imageWidth > 0 && imageHeight > 0}
            <button
              type="button"
              bind:this={interactionLayer}
              class="selection-layer"
              class:painting={selectionTool === 'brush' || selectionTool === 'eraser'}
              aria-label={`${selectionTool.replace('_', ' ')} selection canvas`}
              on:pointerdown={handlePointerDown}
              on:pointermove={handlePointerMove}
              on:pointerup={handlePointerUp}
              on:pointercancel={cancelGesture}
              on:click={handlePolygonClick}
              on:keydown={handleSelectionKey}
            >
              <svg viewBox={`0 0 ${imageWidth} ${imageHeight}`} preserveAspectRatio="none" aria-hidden="true">
                {#if previewBounds && selectionTool === 'rectangle'}
                  <rect class="selection-preview" x={previewBounds.left} y={previewBounds.top} width={previewBounds.right - previewBounds.left} height={previewBounds.bottom - previewBounds.top}></rect>
                {:else if previewBounds && selectionTool === 'ellipse'}
                  <ellipse class="selection-preview" cx={(previewBounds.left + previewBounds.right) / 2} cy={(previewBounds.top + previewBounds.bottom) / 2} rx={(previewBounds.right - previewBounds.left) / 2} ry={(previewBounds.bottom - previewBounds.top) / 2}></ellipse>
                {:else if dragPoints.length > 1}
                  <polyline class="selection-preview" points={dragPoints.map((point) => `${point.x},${point.y}`).join(' ')}></polyline>
                {/if}
                {#if polygonPoints.length}
                  <polyline class="selection-preview" points={[...polygonPoints, ...(hoverPoint ? [hoverPoint] : [])].map((point) => `${point.x},${point.y}`).join(' ')}></polyline>
                  {#each polygonPoints as point}<circle class="polygon-node" cx={point.x} cy={point.y} r={Math.max(2, imageWidth / 500)}></circle>{/each}
                {/if}
                {#if hoverPoint && (selectionTool === 'brush' || selectionTool === 'eraser')}
                  <circle class="brush-cursor" cx={hoverPoint.x} cy={hoverPoint.y} r={brushDiameter / 2}></circle>
                {/if}
              </svg>
            </button>
          {/if}
          {#if gridOverlay}<div class="editing-grid" aria-label="Composition grid overlay"></div>{/if}
          {#if crosshair}<div class="crosshair" aria-label="Pixel inspector crosshair"><i></i><b></b></div>{/if}
        </div>
      {/if}
      {#if processing}
        <div class="processing-pill"><span></span> Forging preview</div>
      {/if}
      {#if comparison && !splitComparison && comparisonMode === 'swipe'}
        <label class="comparison-slider">
          <span class="sr-only">Before and after divider</span>
          <input
            type="range"
            min="0"
            max="100"
            value={comparisonPosition}
            on:input={(event) =>
              oncomparisonchange(Number((event.currentTarget as HTMLInputElement).value))}
          />
        </label>
      {/if}
    </div>
  {:else}
    <button class="empty-state" type="button" on:click={onopen}>
      <span class="empty-icon" aria-hidden="true">
        <span class="sun"></span>
        <span class="mountain one"></span>
        <span class="mountain two"></span>
      </span>
      <strong>Bring a photo to the forge</strong>
      <span>Drop a PNG, JPEG, or WebP here</span>
      <em>or choose an image</em>
      <small>Your image stays on this device.</small>
    </button>
  {/if}
</section>

<style>
  .stage {
    min-width: 0;
    min-height: 0;
    position: relative;
    display: grid;
    place-items: center;
    overflow: auto;
    padding: 28px;
    background:
      radial-gradient(circle at 50% 42%, rgba(192, 231, 126, 0.035), transparent 34%),
      var(--workspace);
  }

  .stage::before {
    content: '';
    position: absolute;
    inset: 0;
    pointer-events: none;
    opacity: 0.2;
    background-image:
      linear-gradient(var(--grid) 1px, transparent 1px),
      linear-gradient(90deg, var(--grid) 1px, transparent 1px);
    background-size: 28px 28px;
  }

  .canvas-shell {
    position: relative;
    min-width: min-content;
    margin: auto;
  }

  .canvas {
    position: relative;
    width: fit-content;
    line-height: 0;
    transform: scale(var(--zoom));
    transform-origin: center;
    transition: transform 120ms ease;
    box-shadow: 0 24px 70px rgba(0, 0, 0, 0.48), 0 0 0 1px rgba(255,255,255,0.08);
    background: repeating-conic-gradient(#20221e 0 25%, #292c27 0 50%) 50% / 18px 18px;
  }

  .split-canvas {
    display: grid;
    grid-template-columns: repeat(2, minmax(0, 1fr));
    gap: 16px;
    align-items: center;
    transform: scale(var(--zoom));
    transform-origin: center;
    transition: transform 120ms ease;
  }

  .split-canvas figure {
    min-width: 0;
    position: relative;
    display: grid;
    place-items: center;
    margin: 0;
    overflow: hidden;
    border: 1px solid rgba(255, 255, 255, 0.08);
    border-radius: 8px;
    background: repeating-conic-gradient(#20221e 0 25%, #292c27 0 50%) 50% / 18px 18px;
    box-shadow: 0 20px 55px rgba(0, 0, 0, 0.4);
  }

  .split-canvas img {
    max-width: min(34vw, 520px);
    max-height: calc(100vh - 245px);
  }

  .split-canvas figcaption {
    position: absolute;
    left: 9px;
    bottom: 9px;
    padding: 6px 8px;
    border-radius: 5px;
    color: white;
    background: rgba(16, 18, 15, 0.78);
    font: 700 0.58rem/1 var(--font-mono);
    letter-spacing: 0.04em;
    text-transform: uppercase;
    backdrop-filter: blur(8px);
  }

  img {
    display: block;
    max-width: min(72vw, 1100px);
    max-height: calc(100vh - 195px);
    width: auto;
    height: auto;
    object-fit: contain;
    user-select: none;
  }

  .before {
    position: absolute;
    inset: 0 auto 0 0;
    overflow: hidden;
  }

  .canvas.blink .before { animation: blink-compare 1s steps(1, end) infinite; }
  .canvas.difference .before { mix-blend-mode: difference; filter: saturate(2.8) contrast(1.45); opacity: .92; }

  .editing-grid, .crosshair { position: absolute; z-index: 8; inset: 0; pointer-events: none; }
  .mask-overlay, .selection-layer { position: absolute; z-index: 7; inset: 0; width: 100%; height: 100%; }
  .mask-overlay { pointer-events: none; image-rendering: auto; }
  .selection-layer { z-index: 9; padding: 0; border: 0; border-radius: 0; cursor: crosshair; touch-action: none; outline: none; background: transparent; }
  .selection-layer svg { display: block; width: 100%; height: 100%; }
  .selection-layer.painting { cursor: none; }
  .selection-preview { fill: rgba(192, 231, 126, .12); stroke: white; stroke-width: max(1px, .08%); stroke-dasharray: 7 5; vector-effect: non-scaling-stroke; pointer-events: none; }
  .polygon-node { fill: var(--accent); stroke: #10120f; stroke-width: 1; vector-effect: non-scaling-stroke; pointer-events: none; }
  .brush-cursor { fill: rgba(192, 231, 126, .12); stroke: white; stroke-width: 1; vector-effect: non-scaling-stroke; pointer-events: none; }
  .editing-grid {
    background-image:
      linear-gradient(90deg, transparent 33.2%, rgba(255,255,255,.7) 33.3%, transparent 33.5%, transparent 66.5%, rgba(255,255,255,.7) 66.7%, transparent 66.8%),
      linear-gradient(transparent 33.2%, rgba(255,255,255,.7) 33.3%, transparent 33.5%, transparent 66.5%, rgba(255,255,255,.7) 66.7%, transparent 66.8%);
    box-shadow: inset 0 0 0 1px rgba(255,255,255,.75);
  }
  .crosshair i, .crosshair b { position: absolute; display: block; background: rgba(255,255,255,.9); box-shadow: 0 0 2px black; }
  .crosshair i { top: 50%; left: 0; right: 0; height: 1px; }
  .crosshair b { left: 50%; top: 0; bottom: 0; width: 1px; }

  .before img {
    max-width: none;
    width: var(--comparison-image-width, auto);
    height: 100%;
  }

  .divider {
    position: absolute;
    top: 0;
    bottom: 0;
    width: 2px;
    transform: translateX(-1px);
    background: rgba(255,255,255,0.9);
    box-shadow: 0 0 18px rgba(0,0,0,0.55);
  }

  .divider span {
    position: absolute;
    top: 50%;
    left: 50%;
    display: grid;
    place-items: center;
    width: 32px;
    height: 32px;
    transform: translate(-50%, -50%);
    border: 2px solid white;
    border-radius: 50%;
    background: rgba(20, 22, 18, 0.9);
    color: white;
    font: 16px/1 system-ui;
  }

  .badge {
    position: absolute;
    top: 12px;
    padding: 6px 8px;
    border-radius: 5px;
    color: white;
    background: rgba(16, 18, 15, 0.7);
    font: 700 0.62rem/1 var(--font-mono);
    letter-spacing: 0.08em;
    text-transform: uppercase;
    backdrop-filter: blur(8px);
  }

  .before-badge { left: 12px; }
  .after-badge { right: 12px; }

  .comparison-slider {
    position: absolute;
    inset: 0;
  }

  .comparison-slider input {
    width: 100%;
    height: 100%;
    margin: 0;
    opacity: 0;
    cursor: ew-resize;
  }

  .processing-pill {
    position: absolute;
    left: 50%;
    bottom: 16px;
    display: flex;
    align-items: center;
    gap: 8px;
    transform: translateX(-50%);
    padding: 8px 11px;
    border: 1px solid var(--line-strong);
    border-radius: 999px;
    color: var(--ink-soft);
    background: rgba(23, 25, 21, 0.92);
    font-size: 0.7rem;
    box-shadow: 0 8px 24px rgba(0,0,0,0.3);
  }

  .processing-pill span {
    width: 7px;
    height: 7px;
    border-radius: 50%;
    background: var(--accent);
    box-shadow: 0 0 0 0 rgba(192, 231, 126, 0.45);
    animation: pulse 1.1s infinite;
  }

  .canvas-shell.stale .canvas { opacity: 0.82; }

  .empty-state {
    position: relative;
    z-index: 1;
    width: min(520px, 90%);
    min-height: 380px;
    display: flex;
    flex-direction: column;
    align-items: center;
    justify-content: center;
    gap: 9px;
    border: 1px dashed var(--line-strong);
    border-radius: 20px;
    color: var(--ink-soft);
    background: rgba(24, 26, 22, 0.68);
    font: inherit;
    cursor: pointer;
    transition: 180ms ease;
  }

  .empty-state:hover,
  .empty-state:focus-visible {
    border-color: var(--accent);
    background: rgba(29, 32, 26, 0.86);
    transform: translateY(-2px);
    outline: none;
  }

  .empty-state strong {
    margin-top: 20px;
    color: var(--ink);
    font-family: var(--font-display);
    font-size: 1.42rem;
    letter-spacing: -0.02em;
  }

  .empty-state span { font-size: 0.84rem; }
  .empty-state em {
    padding: 8px 13px;
    border-radius: 8px;
    color: #162019;
    background: var(--accent);
    font-size: 0.74rem;
    font-style: normal;
    font-weight: 800;
  }
  .empty-state small { margin-top: 18px; color: var(--ink-faint); }

  .empty-icon {
    width: 78px;
    height: 64px;
    position: relative;
    overflow: hidden;
    border: 2px solid var(--line-strong);
    border-radius: 12px;
    background: var(--surface-raised);
  }

  .empty-icon .sun {
    position: absolute;
    top: 12px;
    right: 14px;
    width: 12px;
    height: 12px;
    border-radius: 50%;
    background: var(--accent);
  }

  .mountain {
    position: absolute;
    bottom: -17px;
    width: 54px;
    height: 54px;
    transform: rotate(45deg);
    border-radius: 5px;
    background: var(--line-strong);
  }
  .mountain.one { left: 7px; }
  .mountain.two { right: -12px; bottom: -26px; background: #3b4036; }

  .sr-only {
    position: absolute;
    width: 1px;
    height: 1px;
    padding: 0;
    margin: -1px;
    overflow: hidden;
    clip: rect(0, 0, 0, 0);
    white-space: nowrap;
    border: 0;
  }

  @keyframes pulse {
    70% { box-shadow: 0 0 0 8px rgba(192, 231, 126, 0); }
    100% { box-shadow: 0 0 0 0 rgba(192, 231, 126, 0); }
  }

  @keyframes blink-compare { 0%, 49% { opacity: 1; } 50%, 100% { opacity: 0; } }

  @media (max-width: 760px) {
    .stage { padding: 16px; }
    img { max-width: 88vw; max-height: calc(100vh - 240px); }
    .split-canvas { grid-template-columns: 1fr; }
    .split-canvas img { max-width: 78vw; max-height: calc((100vh - 280px) / 2); }
  }
</style>
