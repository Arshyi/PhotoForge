<script lang="ts">
  import { onMount, tick } from 'svelte';
  import { invoke } from '@tauri-apps/api/core';
  import { getCurrentWebview } from '@tauri-apps/api/webview';
  import { open, save } from '@tauri-apps/plugin-dialog';
  import ImageStage from './lib/components/ImageStage.svelte';
  import SliderControl from './lib/components/SliderControl.svelte';
  import StatusBar from './lib/components/StatusBar.svelte';
  import ToolButton from './lib/components/ToolButton.svelte';
  import { EditHistory } from './lib/stores/history';
  import type {
    EditOperation,
    ExportResult,
    ImageMetadata,
    OpenImageResult,
    OperationType,
    PreviewResult
  } from './lib/types/editor';
  import { errorMessage, formatBytes } from './lib/utils/format';
  import { presets, replaceOperation, valueFor } from './lib/utils/operations';

  const history = new EditHistory();
  let operations: EditOperation[] = [];
  let metadata: ImageMetadata | null = null;
  let originalUrl: string | null = null;
  let previewUrl: string | null = null;
  let zoom = 100;
  let comparison = false;
  let comparisonPosition = 50;
  let processing = false;
  let previewCurrent = true;
  let processingTime = 0;
  let requestId = 0;
  let documentId = 0;
  let activeOpenRequest = 0;
  let renderTimer: ReturnType<typeof setTimeout> | undefined;
  let renderInFlight = false;
  let previewQueued = false;
  let opening = false;
  let exporting = false;
  let settingsOpen = false;
  let toast = '';
  let toastKind: 'error' | 'success' = 'success';
  let toastTimer: ReturnType<typeof setTimeout> | undefined;
  let settingsCloseButton: HTMLButtonElement;

  $: canUndo = history.canUndo;
  $: canRedo = history.canRedo;
  $: comparisonUsesSplitView = valueFor(operations, 'rotate', 0) % 360 !== 0;

  onMount(() => {
    let unlisten: (() => void) | undefined;
    getCurrentWebview()
      .onDragDropEvent((event) => {
        if (event.payload.type === 'drop' && event.payload.paths[0]) {
          void loadPath(event.payload.paths[0]);
        }
      })
      .then((cleanup) => (unlisten = cleanup))
      .catch(() => undefined);

    const handleKeys = (event: KeyboardEvent) => {
      if (event.key === 'Escape' && settingsOpen) {
        event.preventDefault();
        closeSettings();
        return;
      }
      if (!(event.ctrlKey || event.metaKey)) return;
      if (event.key.toLowerCase() === 'o') {
        event.preventDefault();
        void chooseImage();
      } else if (event.key.toLowerCase() === 's') {
        event.preventDefault();
        void exportImage();
      } else if (event.key.toLowerCase() === 'z' && event.shiftKey) {
        event.preventDefault();
        redo();
      } else if (event.key.toLowerCase() === 'z') {
        event.preventDefault();
        undo();
      } else if (event.key.toLowerCase() === 'y') {
        event.preventDefault();
        redo();
      }
    };
    window.addEventListener('keydown', handleKeys);

    return () => {
      unlisten?.();
      window.removeEventListener('keydown', handleKeys);
      if (renderTimer) clearTimeout(renderTimer);
      if (toastTimer) clearTimeout(toastTimer);
      previewQueued = false;
    };
  });

  function notify(message: string, kind: 'error' | 'success' = 'success') {
    toast = message;
    toastKind = kind;
    if (toastTimer) clearTimeout(toastTimer);
    toastTimer = setTimeout(() => (toast = ''), 4200);
  }

  async function chooseImage() {
    if (opening) return;
    try {
      const path = await open({
        multiple: false,
        directory: false,
        title: 'Open a photo',
        filters: [{ name: 'Images', extensions: ['png', 'jpg', 'jpeg', 'webp'] }]
      });
      if (typeof path === 'string') await loadPath(path);
    } catch (error) {
      notify(errorMessage(error), 'error');
    }
  }

  async function loadPath(path: string) {
    const ownOpenRequest = ++requestId;
    activeOpenRequest = ownOpenRequest;
    opening = true;
    processing = true;
    previewCurrent = false;
    previewQueued = false;
    if (renderTimer) clearTimeout(renderTimer);
    try {
      const result = await invoke<OpenImageResult>('open_image', {
        path,
        requestId: ownOpenRequest
      });
      if (!result.isCurrent || activeOpenRequest !== ownOpenRequest) return;
      history.clear();
      operations = [];
      metadata = result.metadata;
      documentId = result.documentId;
      originalUrl = result.originalPreviewDataUrl;
      previewUrl = result.previewDataUrl;
      processingTime = result.processingTimeMs;
      zoom = 100;
      comparison = false;
      previewCurrent = true;
      notify(`${result.metadata.filename} opened locally`);
    } catch (error) {
      if (activeOpenRequest === ownOpenRequest) {
        previewCurrent = true;
        notify(errorMessage(error), 'error');
      }
    } finally {
      if (activeOpenRequest === ownOpenRequest) {
        opening = false;
        processing = renderInFlight;
      }
    }
  }

  function schedulePreview() {
    if (!metadata) return;
    requestId += 1;
    previewCurrent = false;
    previewQueued = true;
    if (renderTimer) clearTimeout(renderTimer);
    if (operations.length === 0) {
      previewQueued = false;
      previewUrl = originalUrl;
      processingTime = 0;
      previewCurrent = true;
      return;
    }
    renderTimer = setTimeout(() => void drainPreviewQueue(), 120);
  }

  async function drainPreviewQueue() {
    if (renderInFlight || !metadata || opening) return;
    renderInFlight = true;
    try {
      while (previewQueued && metadata && !opening) {
        previewQueued = false;
        const ownRequest = requestId;
        const ownDocument = documentId;
        const pipeline = operations.map((operation) => ({ ...operation })) as EditOperation[];
        processing = true;
        try {
          const result = await invoke<PreviewResult>('render_preview', {
            operations: pipeline,
            documentId: ownDocument,
            requestId: ownRequest
          });
          if (
            result.isCurrent &&
            result.requestId === requestId &&
            ownDocument === documentId
          ) {
            previewUrl = result.previewDataUrl;
            processingTime = result.processingTimeMs;
            previewCurrent = true;
          }
        } catch (error) {
          if (ownRequest === requestId && ownDocument === documentId) {
            previewCurrent = true;
            notify(errorMessage(error), 'error');
          }
        }
      }
    } finally {
      renderInFlight = false;
      if (!opening) processing = false;
      if (previewQueued && !opening) void drainPreviewQueue();
    }
  }

  function commit(next: EditOperation[], coalesceKey?: string) {
    operations = history.commit(next, coalesceKey);
    schedulePreview();
  }

  function setNumeric(
    type: OperationType,
    value: number,
    defaultValue: number,
    build: (input: number) => EditOperation
  ) {
    commit(
      replaceOperation(operations, build(value), Math.abs(value - defaultValue) > 0.0001),
      type
    );
  }

  function toggle(operation: EditOperation) {
    const enabled = !operations.some((candidate) => candidate.type === operation.type);
    commit(replaceOperation(operations, operation, enabled));
  }

  function rotate(delta: number) {
    const current = valueFor(operations, 'rotate', 0);
    let degrees = (current + delta) % 360;
    if (degrees < 0) degrees += 360;
    commit(replaceOperation(operations, { type: 'rotate', degrees }, degrees !== 0));
  }

  function undo() {
    if (!history.canUndo) return;
    operations = history.undo();
    schedulePreview();
  }

  function redo() {
    if (!history.canRedo) return;
    operations = history.redo();
    schedulePreview();
  }

  function reset() {
    if (!metadata || operations.length === 0) return;
    operations = history.reset();
    schedulePreview();
  }

  function applyPreset(presetOperations: EditOperation[]) {
    commit(presetOperations);
  }

  async function exportImage() {
    if (!metadata || exporting || opening) return;
    try {
      const stem = metadata.filename.replace(/\.[^.]+$/, '');
      const outputPath = await save({
        title: 'Export edited photo',
        defaultPath: `${stem}-photoforge.png`,
        filters: [
          { name: 'PNG image', extensions: ['png'] },
          { name: 'JPEG image', extensions: ['jpg', 'jpeg'] },
          { name: 'WebP image', extensions: ['webp'] }
        ]
      });
      if (!outputPath) return;
      exporting = true;
      const result = await invoke<ExportResult>('export_image', {
        outputPath,
        operations
      });
      processingTime = result.processingTimeMs;
      notify(`Exported ${result.width} × ${result.height} image`);
    } catch (error) {
      notify(errorMessage(error), 'error');
    } finally {
      exporting = false;
    }
  }

  function active(type: OperationType): boolean {
    return operations.some((operation) => operation.type === type);
  }

  const percent = (value: number) => `${Math.round(value * 100)}%`;

  async function openSettings() {
    settingsOpen = true;
    await tick();
    settingsCloseButton?.focus();
  }

  async function closeSettings() {
    settingsOpen = false;
    await tick();
    document.querySelector<HTMLButtonElement>('button[aria-label="Settings"]')?.focus();
  }

  function trapSettingsFocus(event: KeyboardEvent) {
    if (event.key === 'Tab') event.preventDefault();
  }
</script>

<svelte:head>
  <title>{metadata ? `${metadata.filename} — PhotoForge` : 'PhotoForge'}</title>
</svelte:head>

<div class="app-shell" inert={settingsOpen} aria-hidden={settingsOpen}>
  <header class="topbar">
    <div class="brand" aria-label="PhotoForge">
      <span class="brand-mark" aria-hidden="true"><b></b><i></i></span>
      <span><strong>Photo</strong>Forge</span>
      <em>LOCAL</em>
    </div>

    <nav class="primary-actions" aria-label="File actions">
      <ToolButton label="Open" icon="＋" primary disabled={opening} onclick={chooseImage} />
      <ToolButton
        label={exporting ? 'Exporting' : 'Export'}
        icon="⇩"
        disabled={!metadata || exporting || opening}
        onclick={exportImage}
      />
    </nav>

    <div class="history-actions" aria-label="Edit history">
      <ToolButton label="Undo" icon="↶" disabled={!canUndo} title="Undo (Ctrl+Z)" onclick={undo} />
      <ToolButton label="Redo" icon="↷" disabled={!canRedo} title="Redo (Ctrl+Y)" onclick={redo} />
      <ToolButton label="Reset" icon="⌫" disabled={!metadata || !operations.length} onclick={reset} />
    </div>

    <div class="top-spacer"></div>
    <div class="privacy-chip" title="No cloud uploads"><span></span> Offline</div>
    <ToolButton label="Settings" icon="⚙" onclick={openSettings} />
  </header>

  <main>
    <ImageStage
      {originalUrl}
      {previewUrl}
      filename={metadata?.filename ?? ''}
      {comparison}
      {comparisonPosition}
      splitComparison={comparisonUsesSplitView}
      {zoom}
      {processing}
      stale={!previewCurrent}
      onopen={chooseImage}
      oncomparisonchange={(value) => (comparisonPosition = value)}
    />

    <aside aria-label="Editing controls">
      <div class="inspector-title">
        <div>
          <span>Adjustments</span>
          <small>Non-destructive pipeline</small>
        </div>
        <span class="count">{operations.length}</span>
      </div>

      {#if metadata}
        <div class="metadata-card">
          <div class="file-glyph">{metadata.format.slice(0, 3)}</div>
          <div>
            <strong title={metadata.filename}>{metadata.filename}</strong>
            <span>{metadata.width} × {metadata.height} · {formatBytes(metadata.fileSize)}</span>
          </div>
        </div>
      {/if}

      <div
        class="scroll-panel"
        class:disabled={!metadata || opening}
        inert={!metadata || opening}
        aria-disabled={!metadata || opening}
      >
        <section class="tool-section">
          <h2><span>☀</span> Light</h2>
          <SliderControl
            label="Brightness"
            value={valueFor(operations, 'brightness', 0)}
            min={-0.5}
            max={0.5}
            step={0.01}
            defaultValue={0}
            format={percent}
            onchange={(value) =>
              setNumeric('brightness', value, 0, (amount) => ({ type: 'brightness', amount }))}
          />
          <SliderControl
            label="Contrast"
            value={valueFor(operations, 'contrast', 0)}
            min={-0.75}
            max={0.75}
            step={0.01}
            defaultValue={0}
            format={percent}
            onchange={(value) =>
              setNumeric('contrast', value, 0, (amount) => ({ type: 'contrast', amount }))}
          />
          <SliderControl
            label="Gamma"
            value={valueFor(operations, 'gamma', 1)}
            min={0.3}
            max={2.5}
            step={0.01}
            defaultValue={1}
            format={(value) => value.toFixed(2)}
            onchange={(value) => setNumeric('gamma', value, 1, (input) => ({ type: 'gamma', value: input }))}
          />
        </section>

        <section class="tool-section">
          <h2><span>◒</span> Color</h2>
          <SliderControl
            label="Saturation"
            value={valueFor(operations, 'saturation', 0)}
            min={-1}
            max={1}
            step={0.01}
            defaultValue={0}
            format={percent}
            onchange={(value) =>
              setNumeric('saturation', value, 0, (amount) => ({ type: 'saturation', amount }))}
          />
          <div class="toggle-grid">
            <button class:active={active('grayscale')} type="button" on:click={() => toggle({ type: 'grayscale' })}>
              <span>◐</span> Grayscale
            </button>
            <button class:active={active('sepia')} type="button" on:click={() => toggle({ type: 'sepia' })}>
              <span>◑</span> Sepia
            </button>
          </div>
        </section>

        <section class="tool-section">
          <h2><span>✦</span> Detail</h2>
          <SliderControl
            label="Blur"
            value={valueFor(operations, 'gaussian_blur', 0)}
            min={0}
            max={12}
            step={0.1}
            defaultValue={0}
            format={(value) => value.toFixed(1)}
            onchange={(value) =>
              setNumeric('gaussian_blur', value, 0, (radius) => ({ type: 'gaussian_blur', radius }))}
          />
          <SliderControl
            label="Sharpen"
            value={valueFor(operations, 'sharpen', 0)}
            min={0}
            max={2}
            step={0.02}
            defaultValue={0}
            format={percent}
            onchange={(value) =>
              setNumeric('sharpen', value, 0, (strength) => ({ type: 'sharpen', strength }))}
          />
          <p class="truth-note">Sharpening improves edge contrast; it does not recover missing detail.</p>
        </section>

        <section class="tool-section">
          <h2><span>⌘</span> Transform</h2>
          <div class="transform-grid">
            <button type="button" title="Rotate left" on:click={() => rotate(-90)}>↶<small>Left</small></button>
            <button type="button" title="Rotate right" on:click={() => rotate(90)}>↷<small>Right</small></button>
            <button
              type="button"
              class:active={active('reflect_horizontal')}
              title="Reflect horizontally"
              on:click={() => toggle({ type: 'reflect_horizontal' })}
            >⇋<small>Reflect</small></button>
          </div>
        </section>

        <section class="tool-section presets-section">
          <h2><span>▦</span> Presets</h2>
          <div class="preset-list">
            {#each presets as preset}
              <button type="button" on:click={() => applyPreset(preset.operations)}>
                <span><strong>{preset.name}</strong><small>{preset.description}</small></span>
                <b>›</b>
              </button>
            {/each}
          </div>
        </section>
      </div>
    </aside>
  </main>

  <div class="viewbar" aria-label="Preview controls">
    <button type="button" class:active={comparison} disabled={!metadata || opening} on:click={() => (comparison = !comparison)}>
      ◫ <span>Compare</span>
    </button>
    <i></i>
    <button type="button" disabled={!metadata || opening} aria-label="Zoom out" on:click={() => (zoom = Math.max(25, zoom - 25))}>−</button>
    <span class="zoom-value">{zoom}%</span>
    <button type="button" disabled={!metadata || opening} aria-label="Zoom in" on:click={() => (zoom = Math.min(400, zoom + 25))}>＋</button>
    <button type="button" disabled={!metadata || opening} on:click={() => (zoom = 100)}>Fit</button>
  </div>

  <StatusBar
    dimensions={metadata ? `${metadata.width} × ${metadata.height} · ${metadata.format}` : 'No image loaded'}
    {zoom}
    operationCount={operations.length}
    {processingTime}
    isCurrent={previewCurrent}
  />
</div>

{#if toast}
  <div class="toast" class:error={toastKind === 'error'} role="status">
    <span>{toastKind === 'error' ? '!' : '✓'}</span>{toast}
    <button type="button" aria-label="Dismiss message" on:click={() => (toast = '')}>×</button>
  </div>
{/if}

{#if settingsOpen}
  <div
    class="modal-backdrop"
    role="presentation"
    on:click={(event) => event.target === event.currentTarget && closeSettings()}
  >
    <dialog open class="modal" aria-labelledby="settings-title" on:keydown={trapSettingsFocus}>
      <div class="modal-heading">
        <div><span>Settings</span><h1 id="settings-title">Local by design</h1></div>
        <button bind:this={settingsCloseButton} type="button" aria-label="Close settings" on:click={closeSettings}>×</button>
      </div>
      <div class="setting-row">
        <span class="setting-icon">⌂</span>
        <div><strong>On-device processing</strong><p>Images and edits never leave this computer.</p></div>
        <em>Always on</em>
      </div>
      <div class="setting-row">
        <span class="setting-icon">⌁</span>
        <div><strong>Interactive preview</strong><p>Uses a copy capped at 1600 pixels. Exports use full resolution.</p></div>
        <em>Balanced</em>
      </div>
      <div class="setting-row">
        <span class="setting-icon">◎</span>
        <div><strong>Analytics and telemetry</strong><p>PhotoForge includes no analytics, crash reporting, or remote logs.</p></div>
        <em>Off</em>
      </div>
      <p class="modal-footnote">The original file is never modified by default. Export always asks for a new location.</p>
    </dialog>
  </div>
{/if}
