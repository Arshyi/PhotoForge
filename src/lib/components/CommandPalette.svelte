<script lang="ts">
  import { onMount, tick } from 'svelte';
  import { searchCommands, type PaletteCommand } from '../commands/registry';

  export let commands: PaletteCommand[] = [];
  export let onclose: () => void = () => {};

  let query = '';
  let active = 0;
  let input: HTMLInputElement;
  let list: HTMLUListElement;
  /** What was focused before the palette opened, so closing puts focus back. */
  let returnTo: HTMLElement | null = null;

  $: results = searchCommands(commands, query);
  $: if (active >= results.length) active = Math.max(0, results.length - 1);
  $: current = results[active]?.command ?? null;
  // Evaluated when the palette is shown or the query changes, not on every keystroke
  // of a long-running command: a reason is a fact about this moment.
  $: reasons = new Map(results.map(({ command }) => [command.id, command.unavailable()]));
  $: announcement =
    results.length === 0
      ? 'No commands match.'
      : `${results.length} command${results.length === 1 ? '' : 's'}. ${current?.title ?? ''}${
          current && reasons.get(current.id) ? ` — unavailable: ${reasons.get(current.id)}` : ''
        }`;

  onMount(() => {
    returnTo = document.activeElement instanceof HTMLElement ? document.activeElement : null;
    input?.focus();
    return () => returnTo?.focus?.();
  });

  async function move(delta: number) {
    if (!results.length) return;
    active = (active + delta + results.length) % results.length;
    await tick();
    list?.querySelector<HTMLElement>('[aria-selected="true"]')?.scrollIntoView?.({ block: 'nearest' });
  }

  function choose(command: PaletteCommand | null) {
    if (!command) return;
    // A command that cannot run says why and stays open: closing would hide the reason.
    if (reasons.get(command.id)) return;
    onclose();
    void Promise.resolve(command.run()).catch(() => undefined);
  }

  function keys(event: KeyboardEvent) {
    // The key that opened it closes it.
    if ((event.ctrlKey || event.metaKey) && event.shiftKey && event.key.toLowerCase() === 'p') {
      event.preventDefault();
      onclose();
      return;
    }
    switch (event.key) {
      case 'ArrowDown':
        event.preventDefault();
        void move(1);
        break;
      case 'ArrowUp':
        event.preventDefault();
        void move(-1);
        break;
      case 'Home':
        if (event.ctrlKey) {
          event.preventDefault();
          active = 0;
        }
        break;
      case 'End':
        if (event.ctrlKey) {
          event.preventDefault();
          active = Math.max(0, results.length - 1);
        }
        break;
      case 'Enter':
        event.preventDefault();
        choose(current);
        break;
      case 'Escape':
        event.preventDefault();
        event.stopPropagation();
        onclose();
        break;
    }
  }
</script>

<div class="modal-backdrop palette-backdrop" role="presentation" on:mousedown|self={onclose}>
  <dialog open class="palette" aria-label="Command palette" on:keydown={keys}>
    <input
      bind:this={input}
      bind:value={query}
      type="text"
      role="combobox"
      aria-expanded="true"
      aria-controls="palette-results"
      aria-activedescendant={current ? `palette-${current.id}` : undefined}
      aria-autocomplete="list"
      aria-label="Search commands"
      placeholder="Type a command"
      autocomplete="off"
      spellcheck="false"
    />
    <ul id="palette-results" role="listbox" aria-label="Commands" bind:this={list}>
      {#each results as { command }, index (command.id)}
        {@const reason = reasons.get(command.id)}
        <!-- Keyboard use is on the combobox input above, which keeps focus and moves the
             active option; the option itself is only clickable for a pointer. -->
        <!-- svelte-ignore a11y_click_events_have_key_events -->
        <li
          id={`palette-${command.id}`}
          role="option"
          aria-selected={index === active}
          aria-disabled={reason ? 'true' : undefined}
          class:active={index === active}
          class:unavailable={Boolean(reason)}
          on:mousemove={() => (active = index)}
          on:click={() => choose(command)}
        >
          <span class="title">{command.title}</span>
          <span class="meta">
            {#if reason}<em>{reason}</em>{:else}{command.group}{/if}
            {#if command.shortcut}<kbd>{command.shortcut}</kbd>{/if}
          </span>
          {#if command.description && index === active && !reason}<small>{command.description}</small>{/if}
        </li>
      {:else}
        <li class="empty" role="presentation">No commands match “{query}”.</li>
      {/each}
    </ul>
    <p class="status" role="status" aria-live="polite">{announcement}</p>
  </dialog>
</div>

<style>
  .palette-backdrop { align-items: start; padding-top: 12vh; }
  .palette { width: min(560px, 100%); max-height: 70vh; display: grid; grid-template-rows: auto 1fr auto; padding: 0; border: 1px solid var(--line-strong); border-radius: 11px; color: var(--ink); background: var(--surface); overflow: hidden; }
  input { padding: 13px 16px; border: 0; border-bottom: 1px solid var(--line); color: var(--ink); background: transparent; font-size: 0.85rem; }
  input:focus-visible { outline: 2px solid var(--accent); outline-offset: -2px; }
  ul { margin: 0; padding: 4px; list-style: none; overflow: auto; }
  li { display: grid; grid-template-columns: 1fr auto; gap: 2px 12px; padding: 8px 12px; border-radius: 7px; cursor: pointer; font-size: 0.72rem; }
  li.active { background: rgba(140, 200, 120, 0.16); }
  li.unavailable { opacity: 0.6; cursor: default; }
  .title { font-weight: 600; }
  .meta { display: flex; gap: 8px; align-items: center; color: var(--ink-faint); font-size: 0.6rem; }
  .meta em { font-style: normal; }
  kbd { padding: 1px 6px; border: 1px solid var(--line-strong); border-radius: 4px; font: 0.58rem var(--font-mono); }
  small { grid-column: 1 / -1; color: var(--ink-soft); font-size: 0.6rem; }
  .empty { display: block; color: var(--ink-faint); text-align: center; cursor: default; }
  .status { margin: 0; padding: 6px 14px; border-top: 1px solid var(--line); color: var(--ink-faint); font-size: 0.58rem; }
</style>
