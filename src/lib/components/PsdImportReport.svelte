<script lang="ts">
  import { onDestroy, onMount } from 'svelte';
  import { rememberFocus } from '../utils/focus';
  import {
    summarise,
    type PsdExportReport,
    type PsdImportReport,
    type PsdReportKind
  } from '../layers/psd';
  import type { SvgExportReport, SvgImportReport } from '../layers/svg';
  import type { ExrExportReport, ExrImportReport } from '../layers/exr';

  /**
   * The report of an import, or of an export when `mode` says so. An SVG's and an OpenEXR file's
   * are shown too.
   */
  export let report:
    | PsdImportReport
    | PsdExportReport
    | SvgExportReport
    | SvgImportReport
    | ExrImportReport
    | ExrExportReport;
  export let mode: 'import' | 'export' = 'import';
  export let fileName = '';
  export let onclose: () => void = () => {};

  let dialogElement: HTMLDialogElement;
  $: counts = summarise(report);

  $: isSvg = report.format === 'SVG';
  $: isExr = report.format === 'OpenEXR';
  $: exporting = mode === 'export';
  $: headings = {
    not_imported: {
      title: 'Left out',
      help: 'These are not in the imported document. The layers around them are shown without them.'
    },
    not_exported: {
      title: 'Left out of the file',
      help: isSvg
        ? 'These are not in the SVG, which holds shapes. Your document here is unchanged.'
        : isExr
          ? 'These are not in the OpenEXR file, which holds one picture. Your document here is unchanged.'
          : 'These are not in the Photoshop file, and the layers around them show without them there. Your document here is unchanged.'
    },
    approximated: {
      title: exporting
        ? 'Written, but not exactly as in PhotoForge'
        : isSvg
          ? 'Imported, but not exactly as in the SVG'
          : isExr
            ? 'Imported, but not exactly as in the OpenEXR file'
            : 'Imported, but not exactly as in Photoshop',
      help: exporting
        ? 'These went into the file, changed in the way each line describes.'
        : 'These came across, changed in the way each line describes.'
    },
    note: { title: 'Notes', help: '' }
  } as Record<PsdReportKind, { title: string; help: string }>;
  const order: PsdReportKind[] = ['not_imported', 'not_exported', 'approximated', 'note'];
  $: layerCount = 'layersWritten' in report ? report.layersWritten : report.layersImported;
  $: groupCount = 'groupsWritten' in report ? report.groupsWritten : report.groupsImported;
  /** The same thing said of the same layer name many times (a stack of "Rectangle 1") is one line. */
  function condensed(entries: PsdImportReport['entries']) {
    const lines = new Map<string, { layer: string | null; detail: string; times: number }>();
    for (const entry of entries) {
      const key = `${entry.layer ?? ''}\u0000${entry.detail}`;
      const line = lines.get(key);
      if (line) line.times += 1;
      else lines.set(key, { layer: entry.layer, detail: entry.detail, times: 1 });
    }
    return [...lines.values()];
  }

  $: groups = order
    .map((kind) => ({
      kind,
      count: report.entries.filter((entry) => entry.kind === kind).length,
      lines: condensed(report.entries.filter((entry) => entry.kind === kind))
    }))
    .filter((group) => group.count > 0);

  function keys(event: KeyboardEvent) {
    if (event.key === 'Escape') {
      event.preventDefault();
      event.stopPropagation();
      onclose();
    }
  }

  // Where the person was, so closing the report returns them there.
  const restoreFocus = rememberFocus();
  onDestroy(restoreFocus);
  onMount(() => dialogElement?.querySelector<HTMLElement>('button.primary')?.focus());
</script>

<div class="modal-backdrop" role="presentation">
  <dialog
    open
    bind:this={dialogElement}
    class="modal psd-modal"
    aria-labelledby="psd-report-title"
    on:keydown={keys}
  >
    <div class="modal-heading">
      <div>
        <span>{isSvg ? (exporting ? 'SVG export' : 'SVG import') : isExr ? (exporting ? 'OpenEXR export' : 'OpenEXR import') : exporting ? 'Photoshop export' : 'Photoshop import'}</span>
        <h1 id="psd-report-title">{fileName || (isSvg ? 'SVG' : isExr ? 'OpenEXR image' : 'Photoshop document')}</h1>
      </div>
      <button type="button" aria-label="Close import report" on:click={onclose}>×</button>
    </div>
    <div class="body">
      <p class="summary" data-testid="psd-summary">
        {report.format}, {report.width} × {report.height}{#if 'bitDepth' in report}, {report.bitDepth}-bit {report.colorMode}{/if}.
        {layerCount}
        {isSvg ? (layerCount === 1 ? 'shape' : 'shapes') : layerCount === 1 ? 'layer' : 'layers'}{groupCount
          ? ` and ${groupCount} ${groupCount === 1 ? 'group' : 'groups'}`
          : ''} {mode === 'export' ? 'written' : 'imported'}.
      </p>
      {#if counts.clean}
        <p class="clean" data-testid="psd-clean">
          Nothing was approximated or left out.
        </p>
      {/if}
      {#each groups as group}
        <section class={`group ${group.kind}`} aria-label={headings[group.kind].title}>
          <h2>{headings[group.kind].title} <span class="count">{group.count}</span></h2>
          {#if headings[group.kind].help}<p class="help">{headings[group.kind].help}</p>{/if}
          <ul>
            {#each group.lines as line}
              <li>
                {#if line.layer}<strong>{line.layer}</strong>{/if}
                <span>{line.detail}</span>
                {#if line.times > 1}<span class="times">on {line.times} layers with this name</span>{/if}
              </li>
            {/each}
          </ul>
        </section>
      {/each}
      <p class="fine">
        {#if isSvg && exporting}
          Your document in PhotoForge is not changed. Open the file in a browser or a vector editor
          to see it there; where it differs from PhotoForge is listed above.
        {:else if isExr && exporting}
          Your document in PhotoForge is not changed. The file holds one picture of scene-linear
          light; open it in Blender, Nuke, Photoshop or any editor that reads OpenEXR.
        {:else if mode === 'export'}
          Your document in PhotoForge is not changed. Open the file in Photoshop, or any editor that
          reads PSD, to continue there.
        {:else}
          The original file is not changed. Save this document as a PhotoForge project to keep your work.
        {/if}
      </p>
      <div class="modal-actions">
        <button type="button" class="primary" on:click={onclose}>Close</button>
      </div>
    </div>
  </dialog>
</div>

<style>
  .psd-modal { width: min(620px, 100%); }
  .body { display: grid; gap: 12px; padding: 14px 21px 21px; font-size: 0.66rem; max-height: 70vh; overflow: auto; }
  .summary { margin: 0; color: var(--ink); line-height: 1.5; }
  .clean { margin: 0; padding: 8px 10px; border-radius: 7px; background: rgba(110, 190, 120, 0.14); }
  .group h2 { margin: 0 0 4px; font-size: 0.7rem; }
  .group.not_imported h2,
  .group.not_exported h2 { color: #e0795a; }
  .count { margin-left: 6px; padding: 1px 6px; border-radius: 999px; background: var(--line); font-weight: 400; }
  .help { margin: 0 0 6px; color: var(--ink-soft); }
  ul { margin: 0; padding: 0; list-style: none; display: grid; gap: 6px; }
  li { display: grid; gap: 2px; padding: 7px 10px; border: 1px solid var(--line); border-radius: 7px; line-height: 1.45; }
  li strong { color: var(--ink); }
  li span { color: var(--ink-soft); }
  li .times { color: var(--ink-faint); }
  .fine { margin: 0; color: var(--ink-faint); }
</style>
