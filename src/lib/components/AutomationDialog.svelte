<script lang="ts">
  import { onDestroy, onMount } from 'svelte';
  import { rememberFocus } from '../utils/focus';
  import {
    defaultParams,
    defaultValue,
    dependencyStatus,
    describeCondition,
    describeStep,
    duplicateMacro,
    duplicateStep,
    insertStep,
    macroPluginDependencies,
    moveStep,
    newMacro,
    newStep,
    removeStep,
    setMacroDetails,
    setStepEnabled,
    toCalls,
    updateStep,
    validateMacro,
    type Macro,
    type MacroStep
  } from '../automation/macros';
  import type { Condition, OperationSpec, StepReport } from '../operations/types';
  import type { PluginSummary } from '../plugins/types';
  import ConditionField from './ConditionField.svelte';
  import MacroParamField from './MacroParamField.svelte';

  /** Every operation a step may be, as the backend's registry publishes them. */
  export let specs: OperationSpec[] = [];
  export let macros: Macro[] = [];
  export let plugins: PluginSummary[] = [];
  /** Which macro to open on, e.g. the one a recording just made. */
  export let selectedId: string | null = null;
  /** Things the person should know before they trust what is here, e.g. what a recording left out. */
  export let notices: string[] = [];
  /** Whether a document is open to run against. */
  export let hasDocument = true;
  export let onchange: (macros: Macro[]) => void = () => {};
  /** What the macro would do, without doing it. */
  export let oncheck: (macro: Macro) => Promise<StepReport[]> = async () => [];
  /** Runs the macro as one transaction; resolves to what to tell the person. */
  export let onrun: (macro: Macro) => Promise<string> = async () => '';
  export let onrecord: () => void = () => {};
  export let onimport: () => Promise<Macro | null> = async () => null;
  export let onexport: (macro: Macro) => Promise<string | null> = async () => null;
  export let onclose: () => void = () => {};

  let dialogElement: HTMLDialogElement;
  let items: Macro[] = macros;
  let chosen: string | null = selectedId ?? macros[0]?.id ?? null;
  let open: Record<string, boolean> = {};
  let addOperation = '';
  let results: StepReport[] | null = null;
  let checkedFor = '';
  let message = '';
  let error = '';
  let busy: '' | 'check' | 'run' | 'file' = '';
  let confirmFor: string | null = null;

  $: selected = items.find((macro) => macro.id === chosen) ?? null;
  $: problems = selected ? validateMacro(selected, specs) : [];
  $: enabledSteps = selected ? selected.steps.filter((step) => step.enabled) : [];
  $: dependencies = selected ? dependencyStatus(macroPluginDependencies(selected), plugins) : [];
  $: blocked = dependencies.filter((entry) => entry.state !== 'ready');
  $: fingerprint = selected ? `${selected.id}:${selected.modifiedAt}` : '';
  // A check answers for the macro as it was when it was made. Edit the macro and the
  // answer is no longer shown, rather than left up beside steps it no longer describes.
  $: shownResults = results && checkedFor === fingerprint ? results : null;
  $: confirmDelete = Boolean(selected) && confirmFor === selected?.id;
  // Only enabled steps are sent, so a check's answers are in the order of the enabled steps.
  $: outcomes = new Map<string, StepReport>(
    shownResults ? enabledSteps.flatMap((step, index) => (shownResults[index] ? [[step.id, shownResults[index]] as const] : [])) : []
  );
  $: operationGroups = ['layer', 'adjustment', 'pixels', 'document', 'plugin']
    .map((category) => ({ category, specs: specs.filter((spec) => spec.category === category) }))
    .filter((group) => group.specs.length);
  $: if (!addOperation && specs[0]) addOperation = specs[0].id;
  $: runBlock = !selected
    ? 'Choose a macro.'
    : !hasDocument
      ? 'Open an image first.'
      : problems.length
        ? 'Fix the problems listed first.'
        : !enabledSteps.length
          ? 'Turn on at least one step.'
          : blocked.length
            ? `${blocked[0].name}: ${blocked[0].reason}`
            : busy
              ? 'Wait for the current action to finish.'
              : '';

  function commit(next: Macro[]) {
    items = next;
    onchange(items);
  }

  function edit(change: (macro: Macro) => Macro) {
    if (!selected) return;
    const next = change(selected);
    commit(items.map((macro) => (macro.id === next.id ? next : macro)));
    message = '';
    error = '';
  }

  function createMacro() {
    const macro = newMacro(`New macro ${items.length + 1}`);
    commit([...items, macro]);
    chosen = macro.id;
  }

  function copyMacro() {
    if (!selected) return;
    const copy = duplicateMacro(selected);
    commit([...items, copy]);
    chosen = copy.id;
  }

  function deleteMacro() {
    if (!selected) return;
    if (!confirmDelete) {
      confirmFor = selected.id;
      return;
    }
    const remaining = items.filter((macro) => macro.id !== selected?.id);
    commit(remaining);
    chosen = remaining[0]?.id ?? null;
    confirmFor = null;
  }

  function addStep() {
    const spec = specs.find((candidate) => candidate.id === addOperation);
    if (!spec) return;
    const step = newStep(spec.id, defaultParams(spec));
    edit((macro) => insertStep(macro, step));
    open = { ...open, [step.id]: true };
  }

  function setParam(step: MacroStep, name: string, value: unknown) {
    const params = { ...step.params };
    if (value === undefined) delete params[name];
    else params[name] = value;
    edit((macro) => updateStep(macro, step.id, { params }));
  }

  function setWhen(step: MacroStep, when: Condition | null) {
    edit((macro) => updateStep(macro, step.id, { when }));
  }

  function specOf(step: MacroStep): OperationSpec | undefined {
    return specs.find((candidate) => candidate.id === step.op);
  }

  async function check() {
    if (!selected || busy) return;
    busy = 'check';
    message = '';
    error = '';
    try {
      const report = await oncheck(selected);
      results = report;
      checkedFor = fingerprint;
      const skipped = report.filter((step) => step.skipped).length;
      message = skipped
        ? `Checked: ${report.length - skipped} of ${report.length} steps would run; ${skipped} would be skipped. Nothing was changed.`
        : report.length === 1
          ? 'Checked: the step would run. Nothing was changed.'
          : `Checked: all ${report.length} steps would run. Nothing was changed.`;
    } catch (cause) {
      results = null;
      error = cause instanceof Error ? cause.message : String(cause);
    } finally {
      busy = '';
    }
  }

  async function run() {
    if (!selected || runBlock) return;
    busy = 'run';
    message = '';
    error = '';
    try {
      message = await onrun(selected);
    } catch (cause) {
      error = cause instanceof Error ? cause.message : String(cause);
    } finally {
      busy = '';
    }
  }

  async function importFile() {
    if (busy) return;
    busy = 'file';
    error = '';
    try {
      const imported = await onimport();
      if (imported) {
        commit([...items, imported]);
        chosen = imported.id;
        message = `Imported “${imported.name}”. It is a new macro; nothing it contains has run.`;
      }
    } catch (cause) {
      error = cause instanceof Error ? cause.message : String(cause);
    } finally {
      busy = '';
    }
  }

  async function exportFile() {
    if (!selected || busy) return;
    busy = 'file';
    error = '';
    try {
      const path = await onexport(selected);
      if (path) message = `Saved “${selected.name}” to ${path}.`;
    } catch (cause) {
      error = cause instanceof Error ? cause.message : String(cause);
    } finally {
      busy = '';
    }
  }

  function keys(event: KeyboardEvent) {
    if (event.key === 'Escape') {
      event.preventDefault();
      event.stopPropagation();
      onclose();
    }
  }

  // Where the person was, so closing the dialog returns them there.
  const restoreFocus = rememberFocus();
  onDestroy(restoreFocus);

  onMount(() => {
    dialogElement?.querySelector<HTMLElement>('button, input, select')?.focus();
  });
</script>

<div class="modal-backdrop" role="presentation">
  <dialog open bind:this={dialogElement} class="modal automation-modal" aria-labelledby="automation-title" on:keydown={keys}>
    <div class="modal-heading">
      <div>
        <span>Automation</span>
        <h1 id="automation-title">Macros</h1>
      </div>
      <button type="button" aria-label="Close automation" on:click={onclose}>×</button>
    </div>

    <div class="layout">
      <nav class="list" aria-label="Macros">
        <ul>
          {#each items as macro (macro.id)}
            <li>
              <button type="button" class:current={macro.id === chosen} aria-current={macro.id === chosen ? 'true' : undefined} on:click={() => (chosen = macro.id)}>
                {macro.name}
                <small>{macro.steps.length} step{macro.steps.length === 1 ? '' : 's'}</small>
              </button>
            </li>
          {:else}
            <li class="fine">No macros yet.</li>
          {/each}
        </ul>
        <div class="list-actions">
          <button type="button" on:click={createMacro}>New macro</button>
          <button type="button" disabled={!hasDocument || Boolean(busy)} title={hasDocument ? 'Do something in the Layers panel and PhotoForge writes it down' : 'Open an image first'} on:click={onrecord}>Record…</button>
          <button type="button" disabled={Boolean(busy)} on:click={importFile}>Import…</button>
        </div>
      </nav>

      <section class="editor" aria-label="Macro editor">
        {#each notices as notice}
          <p class="notice" role="note">{notice}</p>
        {/each}
        <div class="status" aria-live="polite">
          {#if message}<p role="status">{message}</p>{/if}
          {#if error}<p class="problem" role="alert">{error}</p>{/if}
        </div>
        {#if !selected}
          <p class="fine">Make a macro, record one, or import one. A macro is a list of steps. Each step is one of PhotoForge’s own operations, so a macro can do nothing the editor itself could not, and running it is one entry in Undo.</p>
        {:else}
          <div class="head">
            <div>
              <label for="macro-name">Name</label>
              <input id="macro-name" type="text" maxlength="120" value={selected.name} on:input={(event) => edit((macro) => setMacroDetails(macro, { name: event.currentTarget.value }))} />
            </div>
            <div>
              <label for="macro-description">Description</label>
              <input id="macro-description" type="text" maxlength="500" value={selected.description} on:input={(event) => edit((macro) => setMacroDetails(macro, { description: event.currentTarget.value }))} />
            </div>
          </div>

          {#if dependencies.length}
            <div class="deps" aria-label="Plugins this macro uses">
              <strong>Uses plugins</strong>
              <ul>
                {#each dependencies as entry}
                  <li class:bad={entry.state !== 'ready'}>{entry.name} — {entry.state === 'ready' ? 'ready' : entry.reason}</li>
                {/each}
              </ul>
            </div>
          {/if}

          <ol class="steps" aria-label="Steps">
            {#each selected.steps as step, index (step.id)}
              {@const spec = specOf(step)}
              {@const outcome = outcomes.get(step.id)}
              {@const text = describeStep(step, specs)}
              <li class:off={!step.enabled}>
                <div class="row">
                  <input type="checkbox" aria-label={`Step ${index + 1} ${text.title} enabled`} checked={step.enabled} on:change={(event) => edit((macro) => setStepEnabled(macro, step.id, event.currentTarget.checked))} />
                  <button type="button" class="title" aria-expanded={open[step.id] ? 'true' : 'false'} aria-controls={`step-${step.id}`} on:click={() => (open = { ...open, [step.id]: !open[step.id] })}>
                    <b>{index + 1}.</b> {text.title}
                    {#if step.when}<em>if</em>{/if}
                  </button>
                  {#if outcome}
                    <span class="outcome" class:skip={outcome.skipped}>{outcome.skipped ? 'would be skipped' : 'would run'}</span>
                  {/if}
                  <span class="tools">
                    <button type="button" aria-label={`Move step ${index + 1} up`} disabled={index === 0} on:click={() => edit((macro) => moveStep(macro, step.id, -1))}>↑</button>
                    <button type="button" aria-label={`Move step ${index + 1} down`} disabled={index === selected.steps.length - 1} on:click={() => edit((macro) => moveStep(macro, step.id, 1))}>↓</button>
                    <button type="button" aria-label={`Duplicate step ${index + 1}`} on:click={() => edit((macro) => duplicateStep(macro, step.id))}>Copy</button>
                    <button type="button" aria-label={`Remove step ${index + 1}`} on:click={() => edit((macro) => removeStep(macro, step.id))}>Remove</button>
                  </span>
                </div>
                {#if text.summary}<p class="summary">{text.summary}</p>{/if}
                {#if step.when}<p class="summary">Only if {describeCondition(step.when)}, when this step is reached.</p>{/if}
                {#if open[step.id]}
                  <div class="body" id={`step-${step.id}`}>
                    {#if !spec}
                      <p class="problem" role="alert">{step.op} is not an operation this version of PhotoForge has. Remove the step, or run this macro on a version that does.</p>
                    {:else}
                      <p class="fine">{spec.summary}</p>
                      {#each spec.params as param (param.name)}
                        {#if param.required || step.params[param.name] !== undefined}
                          <MacroParamField
                            spec={param}
                            value={step.params[param.name]}
                            idPrefix={`p-${step.id}`}
                            onchange={(value) => setParam(step, param.name, value)}
                          />
                          {#if !param.required}
                            <button type="button" class="link" on:click={() => setParam(step, param.name, undefined)}>Do not set {param.name}</button>
                          {/if}
                        {:else}
                          <button type="button" class="link" on:click={() => setParam(step, param.name, defaultValue(param.kind))}>Set {param.name}</button>
                        {/if}
                      {/each}
                      <ConditionField value={step.when} id={`c-${step.id}`} onchange={(when) => setWhen(step, when)} />
                      <label for={`note-${step.id}`}>Note</label>
                      <input id={`note-${step.id}`} type="text" maxlength="200" value={step.note} on:input={(event) => edit((macro) => updateStep(macro, step.id, { note: event.currentTarget.value }))} />
                    {/if}
                  </div>
                {/if}
                {#if step.note && !open[step.id]}<p class="summary">{step.note}</p>{/if}
              </li>
            {:else}
              <li class="fine">This macro has no steps. Add one below, or record some.</li>
            {/each}
          </ol>

          <div class="add">
            <label for="add-step">Add a step</label>
            <select id="add-step" bind:value={addOperation}>
              {#each operationGroups as group}
                <optgroup label={group.category}>
                  {#each group.specs as spec}
                    <option value={spec.id}>{spec.title}</option>
                  {/each}
                </optgroup>
              {/each}
            </select>
            <button type="button" on:click={addStep} disabled={selected.steps.length >= 100}>Add step</button>
          </div>

          {#if problems.length}
            <ul class="problems" role="alert" aria-label="Problems with this macro">
              {#each problems as problem}<li>{problem}</li>{/each}
            </ul>
          {/if}

          <div class="actions">
            <button type="button" class:armed={confirmDelete} on:click={deleteMacro}>{confirmDelete ? 'Confirm: delete this macro' : 'Delete macro'}</button>
            <button type="button" on:click={copyMacro}>Duplicate</button>
            <button type="button" disabled={Boolean(busy)} on:click={exportFile}>Export…</button>
            <span class="spacer"></span>
            <button type="button" disabled={Boolean(busy) || Boolean(problems.length) || !enabledSteps.length || !hasDocument} on:click={check}>Check</button>
            <button type="button" class="primary" disabled={Boolean(runBlock)} title={runBlock || 'Run every enabled step as one change you can undo in one go'} on:click={run}>Run</button>
          </div>
          {#if runBlock && selected}<p class="fine">Run is unavailable: {runBlock}</p>{/if}
          <p class="fine">Running a macro is one entry in Undo, and if any step cannot be done, nothing is changed. {toCalls(selected).length} step{toCalls(selected).length === 1 ? '' : 's'} will be sent.</p>
        {/if}
      </section>
    </div>
  </dialog>
</div>

<style>
  .automation-modal { width: min(1040px, 100%); }
  .layout { display: grid; grid-template-columns: 220px 1fr; min-height: 420px; }
  .list { display: grid; align-content: start; gap: 10px; padding: 14px; border-right: 1px solid var(--line); }
  .list ul { display: grid; gap: 4px; margin: 0; padding: 0; list-style: none; }
  .list li > button { display: grid; width: 100%; padding: 7px 9px; border: 1px solid transparent; border-radius: 7px; color: var(--ink-soft); background: transparent; font-size: 0.64rem; text-align: left; cursor: pointer; }
  .list li > button.current { border-color: var(--accent); color: var(--ink); background: var(--surface-raised); }
  .list small { color: var(--ink-faint); font-size: 0.56rem; }
  .list-actions { display: grid; gap: 5px; }
  .list-actions button, .add button, .actions button, .tools button, .link { padding: 5px 9px; border: 1px solid var(--line-strong); border-radius: 6px; color: var(--ink-soft); background: var(--surface-raised); font-size: 0.6rem; font-weight: 700; cursor: pointer; }
  .editor { display: grid; align-content: start; gap: 10px; padding: 14px 18px 18px; font-size: 0.64rem; }
  .head { display: grid; grid-template-columns: 1fr 2fr; gap: 10px; }
  .head > div { display: grid; gap: 4px; }
  label { color: var(--ink-soft); }
  input[type='text'], select { padding: 5px 7px; border: 1px solid var(--line-strong); border-radius: 6px; color: var(--ink); background: var(--surface-raised); font: inherit; }
  .steps { display: grid; gap: 6px; margin: 0; padding: 0; list-style: none; }
  .steps > li { padding: 7px 9px; border: 1px solid var(--line); border-radius: 8px; }
  .steps > li.off { opacity: 0.6; }
  .row { display: flex; gap: 8px; align-items: center; }
  .title { flex: 1; padding: 3px 4px; border: 0; color: var(--ink); background: transparent; font-size: 0.66rem; text-align: left; cursor: pointer; }
  .title em { margin-left: 6px; color: var(--accent); font-size: 0.56rem; font-style: normal; font-weight: 700; text-transform: uppercase; }
  .tools { display: flex; gap: 4px; }
  .tools button:disabled, .actions button:disabled, .add button:disabled, .list-actions button:disabled { opacity: 0.5; }
  .outcome { padding: 2px 6px; border-radius: 5px; background: rgba(120, 170, 120, 0.2); font-size: 0.56rem; }
  .outcome.skip { background: rgba(200, 170, 80, 0.2); }
  .summary { margin: 4px 0 0 26px; color: var(--ink-faint); font-size: 0.58rem; line-height: 1.4; }
  .body { display: grid; gap: 8px; margin: 8px 0 2px 26px; }
  .link { justify-self: start; font-weight: 400; }
  .add { display: flex; gap: 8px; align-items: center; }
  .actions { display: flex; gap: 6px; align-items: center; }
  .actions .spacer { flex: 1; }
  .actions .primary { color: #152012; border-color: var(--accent); background: var(--accent); }
  .actions .armed { border-color: #e0795a; color: #e0795a; }
  .deps { padding: 8px 10px; border-radius: 7px; background: rgba(120, 130, 120, 0.16); }
  .deps ul { margin: 4px 0 0; padding-left: 16px; }
  .deps li.bad { color: #e0795a; }
  .notice { margin: 0; padding: 8px 10px; border-radius: 7px; background: rgba(200, 170, 80, 0.16); line-height: 1.5; }
  .problems { margin: 0; padding: 8px 10px 8px 24px; border-radius: 7px; background: rgba(224, 121, 90, 0.16); line-height: 1.5; }
  .problem { margin: 0; padding: 8px 10px; border-radius: 7px; background: rgba(224, 121, 90, 0.16); }
  .status p { margin: 0; line-height: 1.5; }
  .fine { margin: 0; color: var(--ink-faint); font-size: 0.6rem; line-height: 1.5; }
  @media (max-width: 760px) {
    .layout { grid-template-columns: 1fr; }
    .list { border-right: 0; border-bottom: 1px solid var(--line); }
    .head { grid-template-columns: 1fr; }
  }
</style>
