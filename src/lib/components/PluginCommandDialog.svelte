<script lang="ts">
  import { onDestroy, onMount } from 'svelte';
  import { rememberFocus } from '../utils/focus';
  import ParamForm from './ParamForm.svelte';
  import { firstProblem, initialValues } from '../plugins/params';
  import type { CommandDecl, PluginSummary } from '../plugins/types';
  import type { OperationSpec } from '../operations/types';

  export let plugin: PluginSummary;
  export let command: CommandDecl;
  /** The registry, so the steps can be shown by name. */
  export let operations: OperationSpec[] = [];
  export let remembered: Record<string, number> = {};
  export let onrun: (values: Record<string, number>) => void = () => {};
  export let oncancel: () => void = () => {};

  let values = initialValues(command.parameters, remembered);
  let dialogElement: HTMLDialogElement;

  $: problem = firstProblem(command.parameters, values);
  $: titles = command.steps.map((step) => operations.find((spec) => spec.id === step.op)?.title ?? step.op);

  function keys(event: KeyboardEvent) {
    if (event.key === 'Escape') {
      event.preventDefault();
      event.stopPropagation();
      oncancel();
    }
  }

  // Where the person was, so closing the dialog returns them there.
  const restoreFocus = rememberFocus();
  onDestroy(restoreFocus);

  onMount(() => dialogElement?.querySelector<HTMLElement>('input, select, button.primary')?.focus());
</script>

<div class="modal-backdrop" role="presentation">
  <dialog open bind:this={dialogElement} class="modal command-modal" aria-labelledby="command-title" on:keydown={keys}>
    <div class="modal-heading">
      <div>
        <span>{plugin.manifest?.name ?? plugin.id}</span>
        <h1 id="command-title">{command.title}</h1>
      </div>
      <button type="button" aria-label="Cancel command" on:click={oncancel}>×</button>
    </div>
    <div class="body">
      {#if command.description}<p class="fine">{command.description}</p>{/if}
      <ParamForm params={command.parameters ?? []} {values} idPrefix="command-param" onchange={(next) => (values = next)} />
      <div class="steps" data-testid="steps">
        <strong>It will, as one undoable step:</strong>
        <ol>{#each titles as title}<li>{title}</li>{/each}</ol>
      </div>
      {#if problem}<p class="problem" role="alert">{problem}</p>{/if}
      <div class="modal-actions">
        <button type="button" on:click={oncancel}>Cancel</button>
        <button type="button" class="primary" disabled={Boolean(problem)} on:click={() => onrun(values)}>Run</button>
      </div>
    </div>
  </dialog>
</div>

<style>
  .command-modal { width: min(440px, 100%); }
  .body { display: grid; gap: 12px; padding: 14px 21px 21px; font-size: 0.66rem; }
  .fine { margin: 0; color: var(--ink-soft); line-height: 1.5; }
  .steps { padding: 9px 12px; border: 1px solid var(--line-strong); border-radius: 8px; color: var(--ink-soft); }
  .steps ol { margin: 4px 0 0; padding-left: 18px; }
  .problem { margin: 0; padding: 8px 10px; border-radius: 7px; background: rgba(224, 121, 90, 0.16); }
  button:disabled { opacity: 0.5; }
</style>
