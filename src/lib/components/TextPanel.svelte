<script lang="ts">
  import { onMount } from 'svelte';
  import { invoke } from '@tauri-apps/api/core';
  import { errorMessage } from '../utils/format';
  import type { TextAlign, TextContent } from '../layers/types';

  /** The text layer being edited, or null when the selection is not one. */
  export let content: TextContent | null = null;
  export let layerId = '';
  export let disabled = false;
  /** Raised with the changed content; the host applies it to the document. */
  export let onchange: (layerId: string, content: TextContent) => void = () => {};
  /** Raised when the user asks to turn this layer into pixels. */
  export let onrasterize: (layerId: string) => void = () => {};

  let families: string[] = [];
  let error = '';
  let loaded = false;

  onMount(() => void loadFonts());

  async function loadFonts() {
    try {
      const result = await invoke<{ families: string[] }>('list_system_fonts');
      families = result.families;
    } catch (reason) {
      error = errorMessage(reason);
    } finally {
      loaded = true;
    }
  }

  /**
   * True when the layer asks for a font this machine does not have.
   *
   * The request is never rewritten to whatever was substituted, so this stays
   * true until the font is installed or the user picks a different one — which
   * is the point: the project keeps recording what it was authored in.
   */
  $: missingFont = Boolean(
    content && content.fontFamily && loaded && families.length > 0 && !families.includes(content.fontFamily)
  );

  function update(patch: Partial<TextContent>) {
    if (!content || disabled) return;
    onchange(layerId, { ...content, ...patch });
  }

  function setNumber(key: keyof TextContent, value: string, fallback: number) {
    const parsed = Number.parseFloat(value);
    update({ [key]: Number.isFinite(parsed) ? parsed : fallback } as Partial<TextContent>);
  }

  const alignments: { id: TextAlign; label: string }[] = [
    { id: 'start', label: 'Start' },
    { id: 'center', label: 'Centre' },
    { id: 'end', label: 'End' },
    { id: 'justified', label: 'Justified' }
  ];

  const weights = [100, 200, 300, 400, 500, 600, 700, 800, 900];
</script>

<section class="text-panel" aria-label="Text layer">
  <fieldset {disabled} style="border: 0; padding: 0; display: contents;">
  {#if !content}
    <p class="empty">Select a text layer to edit its words and setting.</p>
  {:else}
    {#if error}
      <p class="error" role="alert">{error}</p>
    {/if}

    <label class="field">
      <span>Text</span>
      <textarea
        rows="3"
        aria-label="Text content"
        value={content.text}
        on:input={(event) => update({ text: (event.currentTarget as HTMLTextAreaElement).value })}
      ></textarea>
    </label>

    <label class="field">
      <span>Font</span>
      <select
        aria-label="Font family"
        value={content.fontFamily}
        on:change={(event) => update({ fontFamily: (event.currentTarget as HTMLSelectElement).value })}
      >
        <option value="">System default</option>
        {#if missingFont}
          <!-- Kept in the list so the control shows what the project asks for
               rather than jumping to whatever was substituted. -->
          <option value={content.fontFamily}>{content.fontFamily} (not installed)</option>
        {/if}
        {#each families as family (family)}
          <option value={family}>{family}</option>
        {/each}
      </select>
    </label>

    {#if missingFont}
      <p class="warning" role="status" data-testid="missing-font">
        This machine does not have {content.fontFamily}. The text is drawn in a substitute face,
        so the line breaks and spacing you see are not the ones it was set with. The project still
        asks for {content.fontFamily}, and opening it on a machine that has the font restores it.
      </p>
    {/if}

    <div class="row">
      <label class="field">
        <span>Size</span>
        <input
          type="number"
          min="0.5"
          max="2000"
          step="1"
          aria-label="Font size"
          value={content.fontSize}
          on:change={(event) =>
            setNumber('fontSize', (event.currentTarget as HTMLInputElement).value, content.fontSize)}
        />
      </label>

      <label class="field">
        <span>Weight</span>
        <select
          aria-label="Font weight"
          value={String(content.fontWeight)}
          on:change={(event) =>
            update({ fontWeight: Number((event.currentTarget as HTMLSelectElement).value) })}
        >
          {#each weights as weight (weight)}
            <option value={String(weight)}>{weight}</option>
          {/each}
        </select>
      </label>
    </div>

    <div class="row">
      <label class="field">
        <span>Line height</span>
        <input
          type="number"
          min="0.1"
          max="10"
          step="0.05"
          aria-label="Line height"
          value={content.lineHeight}
          on:change={(event) =>
            setNumber('lineHeight', (event.currentTarget as HTMLInputElement).value, content.lineHeight)}
        />
      </label>

      <label class="field">
        <span>Letter spacing</span>
        <input
          type="number"
          min="-1000"
          max="1000"
          step="0.5"
          aria-label="Letter spacing"
          value={content.letterSpacing}
          on:change={(event) =>
            setNumber(
              'letterSpacing',
              (event.currentTarget as HTMLInputElement).value,
              content.letterSpacing
            )}
        />
      </label>
    </div>

    <fieldset class="align">
      <legend>Alignment</legend>
      {#each alignments as option (option.id)}
        <label>
          <input
            type="radio"
            name="text-align"
            value={option.id}
            checked={content.align === option.id}
            on:change={() => update({ align: option.id })}
          />
          {option.label}
        </label>
      {/each}
    </fieldset>

    <label class="field checkbox">
      <input
        type="checkbox"
        aria-label="Italic"
        checked={content.italic}
        on:change={(event) => update({ italic: (event.currentTarget as HTMLInputElement).checked })}
      />
      <span>Italic</span>
    </label>

    <label class="field">Wrap width (blank for point text)
      <input aria-label="Text wrap width" type="number" min="1" max="1000000" value={content.wrapWidth ?? ''}
        on:change={(event) => update({ wrapWidth: event.currentTarget.value === '' ? null : Number(event.currentTarget.value) })} />
    </label>
    {#each ['originX', 'originY'] as coordinate}
      <label class="field">{coordinate}<input aria-label={`Text ${coordinate}`} type="number" step="any" value={content[coordinate as 'originX' | 'originY']}
        on:change={(event) => setNumber(coordinate as 'originX' | 'originY', event.currentTarget.value, 0)} /></label>
    {/each}
    {#each ['red', 'green', 'blue', 'alpha'] as channel}
      <label class="field">Fill {channel}<input aria-label={`Text fill ${channel}`} type="number" step="0.01" min="0" max="1"
        value={content.fill[channel as keyof typeof content.fill]}
        on:change={(event) => update({ fill: { ...content.fill, [channel]: Number(event.currentTarget.value) } })} /></label>
    {/each}

    <p class="note">
      This layer stays editable text. Nothing in PhotoForge turns it into pixels unless you
      ask here.
    </p>

    <button type="button" class="rasterize" on:click={() => onrasterize(layerId)}>
      Rasterize to pixels
    </button>
  {/if}
  </fieldset>
</section>

<style>
  .text-panel {
    display: flex;
    flex-direction: column;
    gap: 0.6rem;
    padding: 0.75rem;
  }

  .field {
    display: flex;
    flex-direction: column;
    gap: 0.2rem;
    font-size: 0.85rem;
  }

  .field.checkbox {
    flex-direction: row;
    align-items: center;
    gap: 0.4rem;
  }

  .row {
    display: grid;
    grid-template-columns: 1fr 1fr;
    gap: 0.6rem;
  }

  textarea,
  input[type='number'],
  select {
    font: inherit;
    padding: 0.3rem;
  }

  .align {
    display: flex;
    flex-wrap: wrap;
    gap: 0.5rem;
    border: 1px solid rgba(128, 128, 128, 0.4);
    padding: 0.4rem 0.6rem;
    font-size: 0.85rem;
  }

  .align legend {
    font-size: 0.8rem;
  }

  .warning {
    background: rgba(200, 140, 0, 0.15);
    border-left: 3px solid rgb(200, 140, 0);
    padding: 0.5rem;
    margin: 0;
    font-size: 0.82rem;
  }

  .error {
    color: rgb(200, 60, 60);
    margin: 0;
    font-size: 0.85rem;
  }

  .note,
  .empty {
    margin: 0;
    font-size: 0.8rem;
    opacity: 0.75;
  }

  .rasterize {
    font: inherit;
    padding: 0.4rem;
  }
</style>
