<script lang="ts">
  import { onMount } from 'svelte';
  import { open } from '@tauri-apps/plugin-dialog';
  import { errorMessage } from '../utils/format';
  import { describeLocality } from '../plugins/params';
  import * as client from '../plugins/client';
  import type {
    CapabilityId,
    Inspection,
    PluginStatus,
    PluginSummary,
    TestReport
  } from '../plugins/types';

  export let onclose: () => void = () => {};
  export let onmessage: (message: string, kind?: 'error' | 'success') => void = () => {};
  /** Called whenever the set of plugins, or what they may do, changes. */
  export let onchange: () => void = () => {};
  /** Injectable so the manager can be driven without a backend. */
  export let api: Pick<
    typeof client,
    | 'listPlugins'
    | 'inspectPluginPackage'
    | 'installPluginPackage'
    | 'setPluginEnabled'
    | 'setPluginGrants'
    | 'removePlugin'
    | 'removePluginVersion'
    | 'testPlugin'
  > = client;
  export let pickPackage: () => Promise<string | null> = async () => {
    const path = await open({
      multiple: false,
      directory: false,
      filters: [{ name: 'PhotoForge plugin', extensions: ['photoforge-plugin'] }]
    });
    return typeof path === 'string' ? path : null;
  };

  let status: PluginStatus | null = null;
  let loadError = '';
  let busy = '';
  let expanded: string | null = null;
  let tests: Record<string, TestReport> = {};
  let confirmRemove: string | null = null;
  let dialogElement: HTMLDialogElement;

  /** The package being reviewed before it is installed. */
  let review: { path: string; inspection: Inspection; granted: Set<CapabilityId> } | null = null;
  let installProblem = '';

  const CHIPS: Record<string, string> = {
    available: 'On',
    disabled: 'Turned off',
    notGranted: 'Needs permission',
    runtimeUnavailable: 'Cannot run in this build',
    damaged: 'Damaged',
    missing: 'Missing',
    otherVersion: 'Other version'
  };

  async function refresh() {
    try {
      status = await api.listPlugins();
      loadError = '';
    } catch (error) {
      loadError = errorMessage(error);
    }
  }

  onMount(() => {
    void refresh();
    (dialogElement?.querySelector('button.primary, button') as HTMLElement | null)?.focus();
  });

  async function act(key: string, work: () => Promise<unknown>, success?: string) {
    if (busy) return;
    busy = key;
    try {
      await work();
      await refresh();
      onchange();
      if (success) onmessage(success);
    } catch (error) {
      onmessage(errorMessage(error), 'error');
    } finally {
      busy = '';
    }
  }

  async function startInstall() {
    installProblem = '';
    let path: string | null;
    try {
      path = await pickPackage();
    } catch (error) {
      onmessage(errorMessage(error), 'error');
      return;
    }
    if (!path) return;
    busy = 'inspect';
    try {
      const inspection = await api.inspectPluginPackage(path);
      // Everything it asks for is offered, ticked: a person narrows it, not widens it.
      review = { path, inspection, granted: new Set(inspection.capabilities.map((c) => c.id)) };
    } catch (error) {
      onmessage(errorMessage(error), 'error');
    } finally {
      busy = '';
    }
  }

  function toggleGrant(id: CapabilityId) {
    if (!review) return;
    const next = new Set(review.granted);
    if (next.has(id)) next.delete(id);
    else next.add(id);
    review = { ...review, granted: next };
  }

  async function confirmInstall() {
    if (!review || busy) return;
    const { path, inspection, granted } = review;
    busy = 'install';
    installProblem = '';
    try {
      const report = await api.installPluginPackage(path, inspection.contentHash, [...granted]);
      review = null;
      tests = { ...tests, [report.plugin]: report.test };
      expanded = report.plugin;
      await refresh();
      onchange();
      onmessage(
        report.updatedFrom
          ? `${inspection.manifest.name} updated from ${report.updatedFrom} to ${report.version}.`
          : `${inspection.manifest.name} installed.`
      );
    } catch (error) {
      // The package is refused whole, so the review stays and says why.
      installProblem = errorMessage(error);
    } finally {
      busy = '';
    }
  }

  function grantsOf(plugin: PluginSummary): CapabilityId[] {
    return plugin.manifest?.capabilities ?? [];
  }

  function describeCapability(id: CapabilityId): string {
    const found = [...(review?.inspection.capabilities ?? []), ...(review?.inspection.newCapabilities ?? [])].find(
      (entry) => entry.id === id
    );
    return found?.description ?? CAPABILITY_TEXT[id] ?? id;
  }

  const CAPABILITY_TEXT: Record<string, string> = {
    'filter.pixels': 'Run its filters on the pixels of layers you choose',
    'document.read': 'Show facts about the open document in its panels',
    'document.operations': 'Change the document with its commands (undoable, and respecting locked layers)',
    'ui.panel': 'Add panels',
    'ui.tool': 'Add entries to the tools menu'
  };

  function toggleGranted(plugin: PluginSummary, id: CapabilityId) {
    const now = new Set(plugin.granted);
    if (now.has(id)) now.delete(id);
    else now.add(id);
    void act(`grant:${plugin.id}`, () => api.setPluginGrants(plugin.id, [...now]));
  }

  function keys(event: KeyboardEvent) {
    if (event.key !== 'Escape') return;
    event.preventDefault();
    event.stopPropagation();
    if (review) review = null;
    else if (confirmRemove) confirmRemove = null;
    else onclose();
  }

  $: plugins = status?.plugins ?? [];
</script>

<div class="modal-backdrop" role="presentation">
  <dialog open bind:this={dialogElement} class="modal plugin-modal" aria-labelledby="plugins-title" on:keydown={keys}>
    <div class="modal-heading">
      <div>
        <span>Extensions</span>
        <h1 id="plugins-title">{review ? 'Install a plugin' : 'Plugins'}</h1>
      </div>
      <button type="button" aria-label="Close plugins" on:click={onclose}>×</button>
    </div>

    <div class="body">
      {#if review}
        {@const inspection = review.inspection}
        <section aria-labelledby="review-name">
          <h2 id="review-name">{inspection.manifest.name} <small>{inspection.manifest.version}</small></h2>
          <p class="who">by {inspection.manifest.publisher}{inspection.manifest.license ? ` · ${inspection.manifest.license}` : ''}</p>
          {#if inspection.manifest.description}<p>{inspection.manifest.description}</p>{/if}
          <p class="unsigned" data-testid="signature">{inspection.signature}</p>

          {#if inspection.installedVersion}
            <p class="note" data-testid="update-note">
              {#if inspection.alreadyInstalled}
                This exact version is already installed.
              {:else if inspection.olderThanInstalled}
                This is <strong>older</strong> than the installed version {inspection.installedVersion}.
              {:else}
                This replaces version {inspection.installedVersion}. Documents made with that version keep using it
                until you remove it.
              {/if}
            </p>
          {/if}
          {#if !inspection.runtimeAvailable && inspection.hasModule}
            <p class="problem" role="alert">
              This build of PhotoForge does not include the plugin runtime, so a plugin with filters cannot be installed.
            </p>
          {/if}

          <fieldset class="permissions">
            <legend>It asks to be allowed to</legend>
            {#each inspection.capabilities as capability}
              {@const isNew = inspection.newCapabilities.some((entry) => entry.id === capability.id)}
              <label>
                <input
                  type="checkbox"
                  checked={review.granted.has(capability.id)}
                  on:change={() => toggleGrant(capability.id)}
                />
                <span>
                  {capability.description}
                  {#if isNew}<em class="new">new in this version</em>{/if}
                </span>
              </label>
            {:else}
              <p class="none">Nothing. It only describes things.</p>
            {/each}
          </fieldset>
          <p class="fine">
            It can never read your files, use the network, start programs or see anything outside what is listed. You can
            change these later.
          </p>

          {#if inspection.readme}
            <details><summary>About this plugin</summary><pre class="readme">{inspection.readme}</pre></details>
          {/if}
          {#if installProblem}<p class="problem" role="alert" data-testid="install-problem">{installProblem}</p>{/if}
        </section>
        <div class="modal-actions">
          <button type="button" on:click={() => (review = null)}>Cancel</button>
          <button
            type="button"
            class="primary"
            disabled={busy === 'install' || (!inspection.runtimeAvailable && inspection.hasModule)}
            on:click={confirmInstall}
          >
            {busy === 'install' ? 'Testing and installing…' : inspection.installedVersion && !inspection.alreadyInstalled ? 'Update' : 'Install'}
          </button>
        </div>
      {:else}
        {#if status && !status.runtimeAvailable}
          <p class="problem" role="status" data-testid="no-runtime">
            This build of PhotoForge does not include the plugin runtime. Plugins can be listed, and documents that
            need one will say so, but no plugin can run.
          </p>
        {/if}
        {#if status?.warning}<p class="problem" role="alert" data-testid="state-warning">{status.warning}</p>{/if}
        {#if loadError}<p class="problem" role="alert">{loadError}</p>{/if}
        <p class="fine">
          A plugin can do only what you allow here. It has no access to your files, the network or other programs, and
          PhotoForge does not check who made it.
        </p>
        <div class="toolbar">
          <button type="button" class="primary" disabled={Boolean(busy)} on:click={startInstall}>Install plugin…</button>
          <small>{status ? `Stored in ${status.pluginFolder}` : ''}</small>
        </div>

        <ul class="plugins" aria-label="Installed plugins">
          {#each plugins as plugin (plugin.id)}
            {@const manifest = plugin.manifest}
            {@const report = tests[plugin.id]}
            <li class="plugin" class:open={expanded === plugin.id}>
              <div class="summary">
                <button
                  type="button"
                  class="name"
                  aria-expanded={expanded === plugin.id}
                  on:click={() => (expanded = expanded === plugin.id ? null : plugin.id)}
                >
                  <strong>{manifest?.name ?? plugin.id}</strong>
                  <small>{manifest ? `${manifest.version} · ${manifest.publisher}` : plugin.id}</small>
                </button>
                <span class={`chip ${plugin.availability.kind}`} data-testid={`chip-${plugin.id}`}>
                  {CHIPS[plugin.availability.kind] ?? plugin.availability.kind}
                </span>
              </div>

              {#if expanded === plugin.id}
                <div class="details">
                  {#if manifest?.description}<p>{manifest.description}</p>{/if}
                  {#if plugin.availability.kind === 'damaged'}
                    <p class="problem" role="alert">{plugin.availability.reason}</p>
                  {:else if plugin.availability.kind === 'notGranted'}
                    <p class="note">It needs to be allowed to {plugin.availability.capability} before its filters can run.</p>
                  {/if}

                  {#if manifest}
                    <dl class="offers">
                      {#if manifest.filters?.length}
                        <dt>Filters</dt>
                        <dd>
                          <ul>{#each manifest.filters as filter}<li><strong>{filter.title}</strong> — {describeLocality(filter.locality)}</li>{/each}</ul>
                        </dd>
                      {/if}
                      {#if manifest.commands?.length}
                        <dt>Commands</dt>
                        <dd><ul>{#each manifest.commands as command}<li>{command.title}</li>{/each}</ul></dd>
                      {/if}
                      {#if manifest.panels?.length}
                        <dt>Panels</dt>
                        <dd><ul>{#each manifest.panels as panel}<li>{panel.title}</li>{/each}</ul></dd>
                      {/if}
                      {#if manifest.tools?.length}
                        <dt>Tools</dt>
                        <dd><ul>{#each manifest.tools as tool}<li>{tool.title}</li>{/each}</ul></dd>
                      {/if}
                    </dl>
                  {/if}

                  <fieldset class="permissions" disabled={Boolean(busy)}>
                    <legend>Allowed to</legend>
                    {#each grantsOf(plugin) as capability}
                      <label>
                        <input
                          type="checkbox"
                          checked={plugin.granted.includes(capability)}
                          on:change={() => toggleGranted(plugin, capability)}
                        />
                        <span>{describeCapability(capability)}</span>
                      </label>
                    {:else}
                      <p class="none">It asked for nothing.</p>
                    {/each}
                  </fieldset>

                  {#if report}
                    <div class="test" role="status" data-testid={`test-${plugin.id}`}>
                      <strong>{report.passed ? 'Self-test passed' : 'Self-test failed'}</strong>
                      <ul>
                        {#each report.filters as result}
                          <li class:fail={!result.passed}>
                            {result.filter}: {result.passed ? `ok (${result.tiles} tile${result.tiles === 1 ? '' : 's'})` : result.message}
                          </li>
                        {/each}
                      </ul>
                    </div>
                  {/if}

                  {#if plugin.versions.length > 1}
                    <details class="versions">
                      <summary>{plugin.versions.length} versions installed</summary>
                      <ul>
                        {#each plugin.versions as version}
                          <li>
                            {version.version}
                            {#if version.contentHash === plugin.activeHash}<em>in use</em>{:else}
                              <button
                                type="button"
                                disabled={Boolean(busy)}
                                on:click={() => act(`version:${version.contentHash}`, () => api.removePluginVersion(plugin.id, version.contentHash), `Version ${version.version} removed.`)}
                              >Remove {version.version}</button>
                            {/if}
                          </li>
                        {/each}
                      </ul>
                    </details>
                  {/if}

                  <div class="actions">
                    <button
                      type="button"
                      disabled={Boolean(busy)}
                      on:click={() => act(`enable:${plugin.id}`, () => api.setPluginEnabled(plugin.id, !plugin.enabled))}
                    >{plugin.enabled ? 'Turn off' : 'Turn on'}</button>
                    <button
                      type="button"
                      disabled={Boolean(busy)}
                      on:click={() =>
                        act(`test:${plugin.id}`, async () => {
                          tests = { ...tests, [plugin.id]: await api.testPlugin(plugin.id) };
                        })}
                    >Test</button>
                    {#if confirmRemove === plugin.id}
                      <span class="confirm">Documents that use it will say it is missing.</span>
                      <button type="button" class="danger" disabled={Boolean(busy)} on:click={() => { const id = plugin.id; confirmRemove = null; void act(`remove:${id}`, () => api.removePlugin(id), 'Plugin removed.'); }}>Remove it</button>
                      <button type="button" on:click={() => (confirmRemove = null)}>Keep it</button>
                    {:else}
                      <button type="button" disabled={Boolean(busy)} on:click={() => (confirmRemove = plugin.id)}>Remove…</button>
                    {/if}
                  </div>
                </div>
              {/if}
            </li>
          {:else}
            <li class="empty">{status ? 'No plugins are installed.' : 'Loading…'}</li>
          {/each}
        </ul>
      {/if}
    </div>
  </dialog>
</div>

<style>
  .plugin-modal { width: min(640px, 100%); max-height: calc(100vh - 40px); overflow: auto; }
  .body { display: grid; gap: 12px; padding: 14px 21px 21px; }
  .fine { margin: 0; color: var(--ink-faint); font-size: 0.62rem; line-height: 1.5; }
  .toolbar { display: flex; gap: 10px; align-items: center; }
  .toolbar small { color: var(--ink-faint); font-size: 0.58rem; overflow-wrap: anywhere; }
  .toolbar button, .actions button, .versions button { padding: 7px 11px; border: 1px solid var(--line-strong); border-radius: 7px; color: var(--ink-soft); background: var(--surface-raised); font-size: 0.62rem; font-weight: 700; cursor: pointer; }
  .toolbar button.primary { color: #152012; border-color: var(--accent); background: var(--accent); }
  button:disabled { opacity: 0.5; cursor: default; }
  button:focus-visible { outline: 2px solid var(--accent); outline-offset: 2px; }
  .plugins { display: grid; gap: 8px; margin: 0; padding: 0; list-style: none; }
  .plugin { border: 1px solid var(--line-strong); border-radius: 9px; background: var(--surface-raised); }
  .summary { display: flex; align-items: center; justify-content: space-between; gap: 10px; padding: 9px 12px; }
  .name { display: grid; gap: 2px; padding: 0; border: 0; color: var(--ink); background: transparent; text-align: left; cursor: pointer; }
  .name strong { font-size: 0.74rem; }
  .name small { color: var(--ink-faint); font-size: 0.6rem; }
  .chip { padding: 3px 8px; border-radius: 999px; font: 700 0.56rem/1 var(--font-mono); letter-spacing: 0.04em; background: rgba(120, 130, 120, 0.2); white-space: nowrap; }
  .chip.available { background: rgba(140, 200, 120, 0.22); }
  .chip.damaged, .chip.missing, .chip.runtimeUnavailable, .chip.notGranted, .chip.otherVersion { background: rgba(224, 121, 90, 0.22); }
  .details { display: grid; gap: 10px; padding: 4px 12px 12px; font-size: 0.66rem; line-height: 1.5; }
  .details p { margin: 0; }
  .offers { display: grid; grid-template-columns: auto 1fr; gap: 4px 12px; margin: 0; }
  .offers dt { color: var(--ink-faint); }
  .offers dd { margin: 0; }
  .offers ul, .test ul, .versions ul { margin: 0; padding-left: 16px; }
  .permissions { display: grid; gap: 6px; margin: 0; padding: 9px 12px; border: 1px solid var(--line-strong); border-radius: 8px; }
  .permissions legend { padding: 0 6px; color: var(--ink-faint); font-size: 0.6rem; }
  .permissions label { display: flex; gap: 8px; align-items: flex-start; cursor: pointer; }
  .permissions .none { color: var(--ink-faint); }
  .new { margin-left: 6px; color: var(--accent); font-style: normal; font-weight: 700; }
  .unsigned, .note { margin: 0; padding: 8px 10px; border-radius: 7px; background: rgba(120, 130, 120, 0.16); font-size: 0.64rem; }
  .problem { margin: 0; padding: 9px 11px; border-radius: 7px; background: rgba(224, 121, 90, 0.16); font-size: 0.66rem; line-height: 1.5; }
  .test { padding: 8px 10px; border-radius: 7px; background: rgba(120, 130, 120, 0.12); }
  .test .fail { color: #e0795a; }
  .actions { display: flex; flex-wrap: wrap; gap: 6px; align-items: center; }
  .actions .danger { border-color: #e0795a; color: #e0795a; }
  .confirm { color: var(--ink-soft); font-size: 0.62rem; }
  .empty { padding: 14px; color: var(--ink-faint); font-size: 0.68rem; text-align: center; }
  .who { margin: 0; color: var(--ink-faint); font-size: 0.64rem; }
  .readme { max-height: 200px; overflow: auto; margin: 6px 0 0; padding: 8px; border-radius: 6px; background: rgba(0, 0, 0, 0.2); font-size: 0.6rem; white-space: pre-wrap; }
  h2 { margin: 0; font-size: 0.86rem; }
  h2 small { color: var(--ink-faint); font-weight: 400; }
</style>
