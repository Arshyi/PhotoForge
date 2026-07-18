<script lang="ts">
  export let originalUrl: string | null;
  export let previewUrl: string | null;
  export let filename = '';
  export let comparison = false;
  export let comparisonPosition = 50;
  export let zoom = 100;
  export let processing = false;
  export let stale = false;
  export let onopen: () => void;
  export let oncomparisonchange: (value: number) => void;
</script>

<section class="stage" aria-label="Image preview">
  {#if previewUrl}
    <div class="canvas-shell" class:processing class:stale>
      <div class="canvas" style={`--zoom: ${zoom / 100}`}>
        <img class="processed" src={previewUrl} alt={`Edited preview of ${filename}`} draggable="false" />
        {#if comparison && originalUrl}
          <div class="before" style={`width: ${comparisonPosition}%`}>
            <img src={originalUrl} alt={`Original preview of ${filename}`} draggable="false" />
          </div>
          <div class="divider" style={`left: ${comparisonPosition}%`} aria-hidden="true">
            <span>↔</span>
          </div>
          <span class="badge before-badge">Before</span>
          <span class="badge after-badge">After</span>
        {/if}
      </div>
      {#if processing}
        <div class="processing-pill"><span></span> Forging preview</div>
      {/if}
      {#if comparison}
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

  @media (max-width: 760px) {
    .stage { padding: 16px; }
    img { max-width: 88vw; max-height: calc(100vh - 240px); }
  }
</style>
