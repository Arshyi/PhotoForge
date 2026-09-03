<script lang="ts">
  import SliderControl from './SliderControl.svelte';
  import CurveEditor from './CurveEditor.svelte';
  import {
    adjustmentDefinitions,
    definitionFor,
    formatField,
    hslBands,
    readField,
    writeField,
    type ScalarField
  } from '../layers/adjustments';
  import type { BaseEditOperation, CurveSet, HslAdjustment, HslSettings } from '../types/editor';

  /** `null` means the dialog is closed. */
  export let operation: BaseEditOperation | null = null;
  /** Creating a new layer versus editing an existing one. */
  export let mode: 'create' | 'edit' = 'create';
  export let layerName = '';
  export let onchange: (operation: BaseEditOperation, coalesceKey?: string) => void;
  export let onconfirm: (operation: BaseEditOperation) => void;
  export let oncancel: () => void;

  let dialog: HTMLDivElement;

  $: definition = operation ? definitionFor(operation.type) : null;
  $: levels = operation?.type === 'levels' ? operation : null;
  $: hslSettings = operation?.type === 'hsl' ? operation.settings : null;
  $: curveSet = operation?.type === 'curves' ? operation.curves : null;

  function updateCurves(curves: CurveSet, coalesceKey?: string) {
    if (!operation || operation.type !== 'curves') return;
    onchange({ ...operation, curves }, coalesceKey);
  }

  function choose(type: string) {
    const chosen = adjustmentDefinitions.find((entry) => entry.type === type);
    if (chosen) onchange(chosen.build());
  }

  function updateScalar(field: ScalarField, value: number) {
    if (!operation) return;
    onchange(writeField(operation, field, value), `adjustment:${operation.type}:${field.key}`);
  }

  function updateLevels(key: string, value: number) {
    if (!operation || operation.type !== 'levels') return;
    const next = { ...operation, [key]: Math.round(value) } as typeof operation;
    // Levels validation requires a strictly increasing input range, so keep the
    // two input points from crossing rather than sending a rejected edit.
    if (key === 'input_black') next.input_black = Math.min(next.input_black, next.input_white - 1);
    if (key === 'input_white') next.input_white = Math.max(next.input_white, next.input_black + 1);
    if (key === 'gamma') next.gamma = Math.min(10, Math.max(0.1, value));
    if (key === 'output_black') next.output_black = Math.min(next.output_black, next.output_white);
    if (key === 'output_white') next.output_white = Math.max(next.output_white, next.output_black);
    onchange(next, `adjustment:levels:${key}`);
  }

  function updateHsl(band: keyof HslSettings, key: keyof HslAdjustment, value: number) {
    if (!operation || operation.type !== 'hsl') return;
    const settings: HslSettings = {
      ...operation.settings,
      [band]: { ...operation.settings[band], [key]: value }
    };
    onchange({ ...operation, settings }, `adjustment:hsl:${band}:${key}`);
  }

  function trapFocus(event: KeyboardEvent) {
    if (event.key === 'Escape') {
      event.preventDefault();
      oncancel();
      return;
    }
    if (event.key !== 'Tab' || !dialog) return;
    const focusable = [...dialog.querySelectorAll<HTMLElement>('button, input, select')].filter(
      (element) => !element.hasAttribute('disabled')
    );
    if (focusable.length === 0) return;
    const first = focusable[0];
    const last = focusable[focusable.length - 1];
    if (event.shiftKey && window.document.activeElement === first) {
      event.preventDefault();
      last.focus();
    } else if (!event.shiftKey && window.document.activeElement === last) {
      event.preventDefault();
      first.focus();
    }
  }
</script>

{#if operation}
  <div
    class="adjustment-backdrop"
    role="dialog"
    aria-modal="true"
    tabindex="-1"
    aria-label={mode === 'create' ? 'New adjustment layer' : `Edit ${layerName}`}
    on:keydown={trapFocus}
  >
    <div class="adjustment-dialog" bind:this={dialog}>
      <header>
        <div>
          <strong>{mode === 'create' ? 'New adjustment layer' : `Edit ${layerName}`}</strong>
          <small>{definition?.description ?? 'Parameters are stored, never baked into pixels.'}</small>
        </div>
        <button type="button" aria-label="Close" on:click={oncancel}>✕</button>
      </header>

      <label class="type-row">
        <span>Adjustment</span>
        <select
          value={operation.type}
          disabled={mode === 'edit'}
          on:change={(event) => choose((event.currentTarget as HTMLSelectElement).value)}
        >
          {#each adjustmentDefinitions as entry}
            <option value={entry.type}>{entry.label}</option>
          {/each}
        </select>
      </label>

      <div class="parameters">
        {#if definition && definition.fields.length}
          {#each definition.fields as field (field.key)}
            <SliderControl
              label={field.label}
              value={readField(operation, field)}
              min={field.min}
              max={field.max}
              step={field.step}
              defaultValue={field.defaultValue}
              format={(value) => formatField(field, value)}
              onchange={(value) => updateScalar(field, value)}
            />
          {/each}
        {:else if levels}
          <div class="levels-grid">
            {#each [['input_black', 'Input black', 0, 255], ['input_white', 'Input white', 0, 255], ['output_black', 'Output black', 0, 255], ['output_white', 'Output white', 0, 255]] as entry (entry[0])}
              <label>
                <span>{entry[1]}</span>
                <input
                  type="number"
                  min={entry[2]}
                  max={entry[3]}
                  step="1"
                  value={(levels as unknown as Record<string, number>)[entry[0] as string]}
                  on:input={(event) =>
                    updateLevels(
                      entry[0] as string,
                      Number((event.currentTarget as HTMLInputElement).value)
                    )}
                />
              </label>
            {/each}
          </div>
          <SliderControl
            label="Gamma"
            value={levels.gamma}
            min={0.1}
            max={10}
            step={0.01}
            defaultValue={1}
            format={(value) => value.toFixed(2)}
            onchange={(value) => updateLevels('gamma', value)}
          />
        {:else if curveSet}
          <CurveEditor curves={curveSet} onchange={updateCurves} />
        {:else if hslSettings}
          <div class="hsl-bands">
            {#each hslBands as band (band)}
              <details open={band === 'master'}>
                <summary>{band[0].toUpperCase() + band.slice(1)}</summary>
                <SliderControl
                  label="Hue"
                  value={hslSettings[band].hue}
                  min={-180}
                  max={180}
                  step={1}
                  defaultValue={0}
                  format={(value) => `${Math.round(value)}°`}
                  onchange={(value) => updateHsl(band, 'hue', value)}
                />
                <SliderControl
                  label="Saturation"
                  value={hslSettings[band].saturation}
                  min={-1}
                  max={1}
                  step={0.01}
                  defaultValue={0}
                  format={(value) => `${Math.round(value * 100)}%`}
                  onchange={(value) => updateHsl(band, 'saturation', value)}
                />
                <SliderControl
                  label="Lightness"
                  value={hslSettings[band].lightness}
                  min={-1}
                  max={1}
                  step={0.01}
                  defaultValue={0}
                  format={(value) => `${Math.round(value * 100)}%`}
                  onchange={(value) => updateHsl(band, 'lightness', value)}
                />
              </details>
            {/each}
          </div>
        {:else}
          <p class="no-parameters">
            This adjustment has no parameters. Its strength is controlled by the layer's opacity
            and mask.
          </p>
        {/if}
      </div>

      <footer>
        <p>Changes preview live. The layer stores these settings, not baked pixels.</p>
        <div>
          <button type="button" on:click={oncancel}>Cancel</button>
          <button
            type="button"
            class="primary"
            on:click={() => operation && onconfirm(operation)}
            >{mode === 'create' ? 'Create layer' : 'Done'}</button
          >
        </div>
      </footer>
    </div>
  </div>
{/if}

<style>
  .adjustment-backdrop { position: fixed; inset: 0; z-index: 40; display: grid; place-items: center; padding: 20px; background: rgba(6,8,10,.72); }
  .adjustment-dialog { display: grid; gap: 12px; width: min(430px, 100%); max-height: min(86vh, 720px); padding: 16px; overflow-y: auto; border: 1px solid var(--line); border-radius: 12px; background: var(--surface); box-shadow: 0 24px 60px rgba(0,0,0,.5); }
  header { display: flex; align-items: flex-start; justify-content: space-between; gap: 10px; }
  header > div { display: grid; gap: 3px; }
  header strong { color: var(--ink); font-size: .82rem; }
  header small { color: var(--ink-faint); font-size: .63rem; line-height: 1.4; }
  header button { min-width: 26px; padding: 4px 7px; }
  .type-row { display: grid; gap: 4px; }
  .type-row span { color: var(--ink-soft); font-size: .64rem; }
  select, input[type='number'] { min-width: 0; padding: 7px; border: 1px solid var(--line); border-radius: 6px; color: var(--ink); background: var(--surface-raised); font: inherit; }
  .parameters { display: grid; gap: 9px; }
  .levels-grid { display: grid; grid-template-columns: 1fr 1fr; gap: 7px; }
  .levels-grid label { display: grid; gap: 3px; color: var(--ink-soft); font-size: .62rem; }
  .hsl-bands { display: grid; gap: 5px; }
  .hsl-bands details { padding: 7px 8px; border: 1px solid var(--line); border-radius: 7px; }
  .hsl-bands summary { color: var(--ink-soft); cursor: pointer; font-size: .66rem; }
  .hsl-bands details[open] summary { margin-bottom: 6px; }
  .no-parameters { margin: 0; padding: 9px; border: 1px dashed var(--line); border-radius: 7px; color: var(--ink-faint); font-size: .64rem; line-height: 1.45; }
  footer { display: grid; gap: 8px; padding-top: 4px; border-top: 1px solid var(--line); }
  footer p { margin: 0; color: var(--ink-faint); font-size: .6rem; line-height: 1.4; }
  footer > div { display: flex; justify-content: flex-end; gap: 6px; }
  footer button { padding: 7px 13px; }
  .primary { border-color: var(--accent); color: var(--accent); background: rgba(192,231,126,.12); }
</style>
