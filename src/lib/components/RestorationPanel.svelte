<script lang="ts">
  import type { EditOperation, OperationType } from '../types/editor';
  import SliderControl from './SliderControl.svelte';

  export let operations: EditOperation[];
  export let onset: (operation: EditOperation, enabled: boolean, coalesceKey?: string) => void;

  let expanded: OperationType | null = null;
  const percent = (value: number) => `${Math.round(value * 100)}%`;

  function operation(type: OperationType): EditOperation | undefined {
    return operations.find((candidate) => candidate.type === type);
  }

  function numberParameter(type: OperationType, key: string, fallback: number): number {
    const candidate = operation(type) as unknown as Record<string, unknown> | undefined;
    return typeof candidate?.[key] === 'number' ? (candidate[key] as number) : fallback;
  }

  function booleanParameter(type: OperationType, key: string, fallback: boolean): boolean {
    const candidate = operation(type) as unknown as Record<string, unknown> | undefined;
    return typeof candidate?.[key] === 'boolean' ? (candidate[key] as boolean) : fallback;
  }

  function setStrength(type: OperationType, strength: number) {
    const enabled = strength > 0.0001;
    let next: EditOperation;
    switch (type) {
      case 'auto_white_balance':
        next = { type, strength };
        break;
      case 'local_contrast':
        next = {
          type,
          strength,
          tile_size: numberParameter(type, 'tile_size', 32),
          clip_limit: numberParameter(type, 'clip_limit', 1.5)
        };
        break;
      case 'denoise':
        next = {
          type,
          strength,
          preserve_edges: numberParameter(type, 'preserve_edges', 0.82)
        };
        break;
      case 'deblock':
        next = { type, strength };
        break;
      case 'edge_aware_sharpen':
        next = {
          type,
          strength,
          radius: numberParameter(type, 'radius', 1.2),
          threshold: numberParameter(type, 'threshold', 0.04)
        };
        break;
      case 'mild_deblur':
        next = { type, strength, radius: numberParameter(type, 'radius', 1.2) };
        break;
      case 'uneven_lighting_correction':
        next = { type, strength, radius: numberParameter(type, 'radius', 40) };
        break;
      case 'document_enhance':
        next = {
          type,
          strength,
          grayscale: booleanParameter(type, 'grayscale', false)
        };
        break;
      default:
        return;
    }
    onset(next, enabled, type);
  }

  function updateAdvanced(type: OperationType, key: string, value: number) {
    const strength = Math.max(numberParameter(type, 'strength', 0), 0.01);
    let next: EditOperation;
    if (type === 'local_contrast') {
      next = {
        type,
        strength,
        tile_size: key === 'tile_size' ? Math.round(value) : numberParameter(type, 'tile_size', 32),
        clip_limit: key === 'clip_limit' ? value : numberParameter(type, 'clip_limit', 1.5)
      };
    } else if (type === 'denoise') {
      next = { type, strength, preserve_edges: value };
    } else if (type === 'edge_aware_sharpen') {
      next = {
        type,
        strength,
        radius: key === 'radius' ? value : numberParameter(type, 'radius', 1.2),
        threshold: key === 'threshold' ? value : numberParameter(type, 'threshold', 0.04)
      };
    } else if (type === 'mild_deblur') {
      next = { type, strength, radius: value };
    } else if (type === 'uneven_lighting_correction') {
      next = { type, strength, radius: value };
    } else {
      return;
    }
    onset(next, true, `${type}:${key}`);
  }

  function setDocumentMode(grayscale: boolean) {
    onset(
      {
        type: 'document_enhance',
        strength: Math.max(numberParameter('document_enhance', 'strength', 0), 0.7),
        grayscale
      },
      true
    );
  }

  function toggleAdvanced(type: OperationType) {
    expanded = expanded === type ? null : type;
  }
</script>

<section class="tool-section restoration-section" aria-labelledby="restoration-heading">
  <h2 id="restoration-heading"><span>◇</span> Restoration</h2>
  <p class="restoration-intro">Local, deterministic tools. No content is generated.</p>

  <div class="restore-tool" title="Balances broad color casts using transparent-aware gray-world statistics.">
    <SliderControl
      label="Auto White Balance"
      value={numberParameter('auto_white_balance', 'strength', 0)}
      min={0} max={1} step={0.02} defaultValue={0} format={percent}
      onchange={(value) => setStrength('auto_white_balance', value)}
    />
  </div>

  <div class="restore-tool">
    <SliderControl
      label="Local Contrast"
      value={numberParameter('local_contrast', 'strength', 0)}
      min={0} max={1} step={0.02} defaultValue={0} format={percent}
      onchange={(value) => setStrength('local_contrast', value)}
    />
    <button class="advanced-toggle" type="button" aria-expanded={expanded === 'local_contrast'} on:click={() => toggleAdvanced('local_contrast')}>Advanced contrast controls</button>
    {#if expanded === 'local_contrast'}
      <div class="advanced-controls">
        <SliderControl label="Contrast tile size" value={numberParameter('local_contrast', 'tile_size', 32)} min={8} max={128} step={8} defaultValue={32} format={(value) => `${value}px`} onchange={(value) => updateAdvanced('local_contrast', 'tile_size', value)} />
        <SliderControl label="Contrast clip limit" value={numberParameter('local_contrast', 'clip_limit', 1.5)} min={0.5} max={4} step={0.1} defaultValue={1.5} format={(value) => value.toFixed(1)} onchange={(value) => updateAdvanced('local_contrast', 'clip_limit', value)} />
      </div>
    {/if}
  </div>

  <div class="restore-tool">
    <SliderControl label="Denoise" value={numberParameter('denoise', 'strength', 0)} min={0} max={1} step={0.02} defaultValue={0} format={percent} onchange={(value) => setStrength('denoise', value)} />
    <button class="advanced-toggle" type="button" aria-expanded={expanded === 'denoise'} on:click={() => toggleAdvanced('denoise')}>Advanced denoise controls</button>
    {#if expanded === 'denoise'}
      <div class="advanced-controls">
        <SliderControl label="Edge preservation" value={numberParameter('denoise', 'preserve_edges', 0.82)} min={0} max={1} step={0.02} defaultValue={0.82} format={percent} onchange={(value) => updateAdvanced('denoise', 'preserve_edges', value)} />
      </div>
    {/if}
  </div>

  <div class="restore-tool" title="Conservatively softens visible 8×8 compression boundaries.">
    <SliderControl label="JPEG Cleanup" value={numberParameter('deblock', 'strength', 0)} min={0} max={1} step={0.02} defaultValue={0} format={percent} onchange={(value) => setStrength('deblock', value)} />
  </div>

  <div class="restore-tool">
    <SliderControl label="Edge-Aware Sharpen" value={numberParameter('edge_aware_sharpen', 'strength', 0)} min={0} max={2} step={0.02} defaultValue={0} format={percent} onchange={(value) => setStrength('edge_aware_sharpen', value)} />
    <button class="advanced-toggle" type="button" aria-expanded={expanded === 'edge_aware_sharpen'} on:click={() => toggleAdvanced('edge_aware_sharpen')}>Advanced sharpening controls</button>
    {#if expanded === 'edge_aware_sharpen'}
      <div class="advanced-controls">
        <SliderControl label="Sharpen radius" value={numberParameter('edge_aware_sharpen', 'radius', 1.2)} min={0.5} max={4} step={0.1} defaultValue={1.2} format={(value) => value.toFixed(1)} onchange={(value) => updateAdvanced('edge_aware_sharpen', 'radius', value)} />
        <SliderControl label="Sharpen threshold" value={numberParameter('edge_aware_sharpen', 'threshold', 0.04)} min={0} max={0.25} step={0.005} defaultValue={0.04} format={percent} onchange={(value) => updateAdvanced('edge_aware_sharpen', 'threshold', value)} />
      </div>
    {/if}
  </div>

  <div class="restore-tool">
    <SliderControl label="Mild Deblur" value={numberParameter('mild_deblur', 'strength', 0)} min={0} max={1} step={0.02} defaultValue={0} format={percent} onchange={(value) => setStrength('mild_deblur', value)} />
    <button class="advanced-toggle" type="button" aria-expanded={expanded === 'mild_deblur'} on:click={() => toggleAdvanced('mild_deblur')}>Advanced deblur controls</button>
    {#if expanded === 'mild_deblur'}
      <div class="advanced-controls">
        <SliderControl label="Deblur radius" value={numberParameter('mild_deblur', 'radius', 1.2)} min={0.5} max={3} step={0.1} defaultValue={1.2} format={(value) => value.toFixed(1)} onchange={(value) => updateAdvanced('mild_deblur', 'radius', value)} />
      </div>
    {/if}
    {#if numberParameter('mild_deblur', 'strength', 0) > 0.7}
      <p class="warning" role="note">Strong deblur may amplify noise or create halos.</p>
    {/if}
  </div>

  <div class="restore-tool">
    <SliderControl label="Uneven Lighting" value={numberParameter('uneven_lighting_correction', 'strength', 0)} min={0} max={1} step={0.02} defaultValue={0} format={percent} onchange={(value) => setStrength('uneven_lighting_correction', value)} />
    <button class="advanced-toggle" type="button" aria-expanded={expanded === 'uneven_lighting_correction'} on:click={() => toggleAdvanced('uneven_lighting_correction')}>Advanced lighting controls</button>
    {#if expanded === 'uneven_lighting_correction'}
      <div class="advanced-controls">
        <SliderControl label="Lighting radius" value={numberParameter('uneven_lighting_correction', 'radius', 40)} min={4} max={96} step={4} defaultValue={40} format={(value) => `${value}px`} onchange={(value) => updateAdvanced('uneven_lighting_correction', 'radius', value)} />
      </div>
    {/if}
  </div>

  <div class="restore-tool document-tool">
    <SliderControl label="Document Enhance" value={numberParameter('document_enhance', 'strength', 0)} min={0} max={1} step={0.02} defaultValue={0} format={percent} onchange={(value) => setStrength('document_enhance', value)} />
    <div class="mode-buttons" aria-label="Document enhancement mode">
      <button type="button" class:active={operation('document_enhance') && !booleanParameter('document_enhance', 'grayscale', false)} on:click={() => setDocumentMode(false)}>Color</button>
      <button type="button" class:active={booleanParameter('document_enhance', 'grayscale', false)} on:click={() => setDocumentMode(true)}>Grayscale</button>
    </div>
  </div>

  <p class="truth-note">Restoration improves captured pixels; it cannot recreate factual detail that was never recorded.</p>
</section>

<style>
  .restoration-section { gap: 14px; }
  .restoration-intro { margin: -6px 0 1px; color: var(--ink-faint); font-size: 0.61rem; line-height: 1.45; }
  .restore-tool { display: grid; gap: 8px; padding: 11px; border: 1px solid var(--line); border-radius: 9px; background: var(--surface-soft); }
  .advanced-toggle { justify-self: start; padding: 0; border: 0; color: var(--ink-faint); background: transparent; font-size: 0.58rem; cursor: pointer; }
  .advanced-toggle::after { content: ' +'; color: var(--accent); }
  .advanced-toggle[aria-expanded='true']::after { content: ' −'; }
  .advanced-toggle:hover { color: var(--accent-bright); }
  .advanced-controls { display: grid; gap: 13px; padding-top: 9px; border-top: 1px solid var(--line); }
  .warning { margin: 0; padding: 7px; border-left: 2px solid #d7a85d; color: #e2c58f; background: rgba(215,168,93,0.08); font-size: 0.58rem; line-height: 1.4; }
  .mode-buttons { display: grid; grid-template-columns: 1fr 1fr; gap: 6px; }
  .mode-buttons button { padding: 7px; border: 1px solid var(--line); border-radius: 6px; color: var(--ink-faint); background: var(--surface); font-size: 0.6rem; cursor: pointer; }
  .mode-buttons button.active, .mode-buttons button:hover { border-color: #59684a; color: var(--accent-bright); background: var(--accent-dim); }
</style>
