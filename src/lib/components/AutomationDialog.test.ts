import { describe, expect, it, vi } from 'vitest';
import { fireEvent, render, screen, waitFor, within } from '@testing-library/svelte';
import AutomationDialog from './AutomationDialog.svelte';
import registry from '../../../src-tauri/tests/fixtures/operation_registry.json';
import type { OperationSpec, StepReport } from '../operations/types';
import type { PluginSummary } from '../plugins/types';
import { insertStep, newMacro, newStep, type Macro } from '../automation/macros';

const specs = registry as unknown as OperationSpec[];
const sky = { type: 'name', name: 'Sky' } as const;

function macro(): Macro {
  let made = newMacro('Dim the sky');
  made = insertStep(made, newStep('core.layer.set_opacity', { selector: sky, opacity: 0.5 }));
  made = insertStep(made, newStep('core.layer.rename', { selector: sky, name: 'Dim sky' }, { kind: 'layer_exists', selector: sky }));
  return made;
}

function setup(props: Partial<Record<string, unknown>> = {}) {
  const handlers = {
    onchange: vi.fn(),
    oncheck: vi.fn(async (made: Macro): Promise<StepReport[]> =>
      made.steps.filter((step) => step.enabled).map((step, index) => ({ index, op: step.op, skipped: index === 1, createdLayers: [], createdPixels: [] }))
    ),
    onrun: vi.fn(async (_macro: Macro) => 'Ran it.'),
    onrecord: vi.fn(),
    onimport: vi.fn(async (): Promise<Macro | null> => null),
    onexport: vi.fn(async (_macro: Macro): Promise<string | null> => null),
    onclose: vi.fn()
  };
  const view = render(AutomationDialog, { props: { specs, macros: [macro()], plugins: [], ...handlers, ...props } });
  const dialog = screen.getByRole('dialog', { name: 'Macros' });
  const saved = () => handlers.onchange.mock.calls.at(-1)?.[0] as Macro[];
  return { ...view, ...handlers, dialog, saved };
}

const button = (dialog: HTMLElement, name: string | RegExp) => within(dialog).getByRole('button', { name }) as HTMLButtonElement;

async function open(dialog: HTMLElement, title: string) {
  await fireEvent.click(within(dialog).getByRole('button', { name: new RegExp(`^\\d+\\. ${title}`) }));
}

describe('checking and running', () => {
  it('shows the outcome of a check beside each step, and clears it when the macro changes', async () => {
    const { dialog, oncheck } = setup();
    await fireEvent.click(button(dialog, 'Check'));
    await waitFor(() => expect(within(dialog).getByText('would be skipped')).toBeTruthy());
    expect(within(dialog).getByText('would run')).toBeTruthy();
    expect(oncheck).toHaveBeenCalledTimes(1);
    await fireEvent.input(within(dialog).getByLabelText('Name'), { target: { value: 'Renamed' } });
    await waitFor(() => expect(within(dialog).queryByText('would run')).toBeNull());
  });

  it('answers a check for the enabled steps only, in their own order', async () => {
    const { dialog } = setup();
    await fireEvent.click(within(dialog).getByLabelText('Step 1 Set layer opacity enabled'));
    await fireEvent.click(button(dialog, 'Check'));
    await waitFor(() => expect(within(dialog).getByText('Checked: the step would run. Nothing was changed.')).toBeTruthy());
    // The one enabled step is the second row, and it is the first answer: "would run".
    expect(within(dialog).getByText('would run')).toBeTruthy();
    expect(within(dialog).queryByText('would be skipped')).toBeNull();
  });

  it('runs the macro it shows, and reports what the application says', async () => {
    const { dialog, onrun } = setup();
    await fireEvent.click(button(dialog, 'Run'));
    expect(await within(dialog).findByText('Ran it.')).toBeTruthy();
    expect(onrun).toHaveBeenCalledTimes(1);
    expect(onrun.mock.calls[0][0].name).toBe('Dim the sky');
  });

  it('shows a failure as an alert, and can be run again afterwards', async () => {
    const onrun = vi.fn().mockRejectedValueOnce(new Error('step 2: no layer called Sky')).mockResolvedValue('Ran it.');
    const { dialog } = setup({ onrun });
    await fireEvent.click(button(dialog, 'Run'));
    expect((await within(dialog).findByRole('alert')).textContent).toContain('step 2: no layer called Sky');
    expect(button(dialog, 'Run').disabled).toBe(false);
    await fireEvent.click(button(dialog, 'Run'));
    expect(await within(dialog).findByText('Ran it.')).toBeTruthy();
    expect(within(dialog).queryByText(/no layer called Sky/)).toBeNull();
  });

  it('cannot start a second run while one is going', async () => {
    let finish: (value: string) => void = () => undefined;
    const onrun = vi.fn(() => new Promise<string>((resolve) => (finish = resolve)));
    const { dialog } = setup({ onrun });
    await fireEvent.click(button(dialog, 'Run'));
    await waitFor(() => expect(button(dialog, 'Run').disabled).toBe(true));
    expect(button(dialog, 'Check').disabled).toBe(true);
    finish('Done.');
    expect(await within(dialog).findByText('Done.')).toBeTruthy();
    expect(onrun).toHaveBeenCalledTimes(1);
  });

  it('says why Run is unavailable: no image, no enabled step, a problem, a plugin that cannot run', async () => {
    const none = setup({ hasDocument: false });
    expect(button(none.dialog, 'Run').disabled).toBe(true);
    expect(within(none.dialog).getByText(/Run is unavailable: Open an image first\./)).toBeTruthy();
    none.unmount();

    const off = setup();
    await fireEvent.click(within(off.dialog).getByLabelText('Step 1 Set layer opacity enabled'));
    await fireEvent.click(within(off.dialog).getByLabelText('Step 2 Rename layer enabled'));
    expect(button(off.dialog, 'Run').disabled).toBe(true);
    expect(within(off.dialog).getByText(/Turn on at least one step/)).toBeTruthy();
    off.unmount();

    const broken = newMacro('Broken');
    const bad = insertStep(broken, newStep('core.layer.set_opacity', { selector: sky, opacity: 9 }));
    const problems = setup({ macros: [{ ...bad, name: 'Broken' }] });
    expect(button(problems.dialog, 'Run').disabled).toBe(true);
    expect(within(problems.dialog).getByRole('alert', { name: 'Problems with this macro' }).textContent).toMatch(/Step 1.*opacity must be between/);
    problems.unmount();

    const needs = insertStep(newMacro('Needs'), newStep('core.plugin.add_adjustment', { plugin: 'com.example.p', filter: 'f' }));
    const plugin = { id: 'com.example.p', enabled: false, availability: { kind: 'disabled' }, manifest: { name: 'Pretty' }, granted: [], versions: [] } as unknown as PluginSummary;
    const blocked = setup({ macros: [{ ...needs, name: 'Needs' }], plugins: [plugin] });
    expect(button(blocked.dialog, 'Run').disabled).toBe(true);
    expect(within(blocked.dialog).getByText(/Pretty — It is turned off\./)).toBeTruthy();
  });

  it('lets a plugin macro run once its plugin is ready', () => {
    const needs = insertStep(newMacro('Needs'), newStep('core.plugin.add_adjustment', { plugin: 'com.example.p', filter: 'f' }));
    const plugin = { id: 'com.example.p', enabled: true, availability: { kind: 'available' }, manifest: { name: 'Pretty' }, granted: [], versions: [] } as unknown as PluginSummary;
    const { dialog } = setup({ macros: [{ ...needs, name: 'Needs' }], plugins: [plugin] });
    expect(within(dialog).getByText(/Pretty — ready/)).toBeTruthy();
    expect(button(dialog, 'Run').disabled).toBe(false);
  });

  it('reports a failed check without leaving an answer up', async () => {
    const oncheck = vi.fn().mockRejectedValue(new Error('step 1: unknown layer'));
    const { dialog } = setup({ oncheck });
    await fireEvent.click(button(dialog, 'Check'));
    expect((await within(dialog).findByRole('alert')).textContent).toContain('step 1: unknown layer');
    expect(within(dialog).queryByText('would run')).toBeNull();
  });
});

describe('editing steps', () => {
  it('turns a step off, moves it, copies it and removes it, saving each time', async () => {
    const { dialog, saved } = setup();
    await fireEvent.click(within(dialog).getByLabelText('Step 1 Set layer opacity enabled'));
    expect(saved()[0].steps[0].enabled).toBe(false);
    await fireEvent.click(button(dialog, 'Move step 1 down'));
    expect(saved()[0].steps.map((step) => step.op)).toEqual(['core.layer.rename', 'core.layer.set_opacity']);
    await fireEvent.click(button(dialog, 'Duplicate step 2'));
    expect(saved()[0].steps).toHaveLength(3);
    expect(new Set(saved()[0].steps.map((step) => step.id)).size).toBe(3);
    await fireEvent.click(button(dialog, 'Remove step 1'));
    expect(saved()[0].steps.map((step) => step.op)).toEqual(['core.layer.set_opacity', 'core.layer.set_opacity']);
  });

  it('cannot move the first step up or the last one down', () => {
    const { dialog } = setup();
    expect(button(dialog, 'Move step 1 up').disabled).toBe(true);
    expect(button(dialog, 'Move step 2 down').disabled).toBe(true);
  });

  it('adds a step of any registered operation with parameters that already pass, and opens it', async () => {
    const { dialog, saved } = setup();
    await fireEvent.change(within(dialog).getByLabelText('Add a step'), { target: { value: 'core.layer.set_blend_mode' } });
    await fireEvent.click(button(dialog, 'Add step'));
    expect(saved()[0].steps[2]).toMatchObject({ op: 'core.layer.set_blend_mode', params: { selector: { type: 'active' }, blendMode: 'normal' } });
    expect(within(dialog).getByRole('combobox', { name: 'blendMode' })).toBeTruthy();
    expect(within(dialog).queryByRole('alert', { name: 'Problems with this macro' })).toBeNull();
  });

  it('offers every operation the registry has, grouped by what it works on', () => {
    const { dialog } = setup();
    const add = within(dialog).getByLabelText('Add a step') as HTMLSelectElement;
    expect([...add.options].map((option) => option.value).sort()).toEqual(specs.map((spec) => spec.id).sort());
    expect([...add.querySelectorAll('optgroup')].map((group) => group.label)).toEqual(['layer', 'adjustment', 'pixels', 'document', 'plugin']);
  });

  it('edits each kind of parameter and checks the result at once', async () => {
    const { dialog, saved } = setup();
    await open(dialog, 'Set layer opacity');
    const opacity = within(dialog).getByLabelText('opacity') as HTMLInputElement;
    await fireEvent.change(opacity, { target: { value: '0.25' } });
    expect(saved()[0].steps[0].params.opacity).toBe(0.25);
    // A number outside the range is accepted into the box and then reported, never silently clamped.
    await fireEvent.change(opacity, { target: { value: '7' } });
    expect(within(dialog).getByRole('alert', { name: 'Problems with this macro' }).textContent).toMatch(/opacity must be between 0 and 1/);
    // Text that is not a number changes nothing.
    await fireEvent.change(opacity, { target: { value: '' } });
    expect(saved()[0].steps[0].params.opacity).toBe(7);

    const selector = within(dialog).getAllByRole('combobox', { name: 'selector' })[0] as HTMLSelectElement;
    await fireEvent.change(selector, { target: { value: 'id' } });
    expect(saved()[0].steps[0].params.selector).toEqual({ type: 'id', id: '' });
    expect(within(dialog).getByText('An identifier names a layer of this document only.')).toBeTruthy();
    await fireEvent.input(within(dialog).getByLabelText('selector: layer identifier'), { target: { value: 'layer-7' } });
    expect(saved()[0].steps[0].params.selector).toEqual({ type: 'id', id: 'layer-7' });
    await fireEvent.change(selector, { target: { value: 'name' } });
    await fireEvent.input(within(dialog).getByLabelText('selector: layer name'), { target: { value: 'Water' } });
    expect(saved()[0].steps[0].params.selector).toEqual({ type: 'name', name: 'Water' });
    await fireEvent.change(selector, { target: { value: 'top' } });
    expect(saved()[0].steps[0].params.selector).toEqual({ type: 'top' });
  });

  it('edits text and booleans, and reports an empty required text', async () => {
    const { dialog, saved } = setup();
    await open(dialog, 'Rename layer');
    const name = within(dialog).getByLabelText('name') as HTMLInputElement;
    await fireEvent.input(name, { target: { value: '' } });
    expect(within(dialog).getByRole('alert', { name: 'Problems with this macro' }).textContent).toMatch(/name needs 1 to \d+ characters/);
    await fireEvent.input(name, { target: { value: 'Dusk' } });
    expect(saved()[0].steps[1].params.name).toBe('Dusk');
    expect(within(dialog).queryByRole('alert', { name: 'Problems with this macro' })).toBeNull();
  });

  it('edits a list of layers, keeping at least one', async () => {
    const group = insertStep(newMacro('Group'), newStep('core.layer.group', { selectors: [{ type: 'name', name: 'A' }] }));
    const { dialog, saved } = setup({ macros: [{ ...group, name: 'Group' }] });
    await open(dialog, 'Group layers');
    expect(button(dialog, 'Remove selectors 1').disabled).toBe(true);
    await fireEvent.click(button(dialog, 'Add layer'));
    expect(saved()[0].steps[0].params.selectors).toEqual([{ type: 'name', name: 'A' }, { type: 'active' }]);
    await fireEvent.click(button(dialog, 'Remove selectors 1'));
    expect(saved()[0].steps[0].params.selectors).toEqual([{ type: 'active' }]);
  });

  it('sets and unsets an optional parameter instead of sending a placeholder', async () => {
    const add = insertStep(newMacro('Group'), newStep('core.layer.add_group', {}));
    const { dialog, saved } = setup({ macros: [{ ...add, name: 'Group' }] });
    await open(dialog, 'New group');
    expect(within(dialog).queryByLabelText(/^name/)).toBeNull();
    await fireEvent.click(button(dialog, 'Set name'));
    expect(saved()[0].steps[0].params).toHaveProperty('name');
    await fireEvent.input(within(dialog).getByLabelText(/^name \(optional\)/), { target: { value: 'Folder' } });
    expect(saved()[0].steps[0].params.name).toBe('Folder');
    await fireEvent.click(button(dialog, 'Do not set name'));
    expect(saved()[0].steps[0].params).not.toHaveProperty('name');
  });

  it('edits JSON, keeping the last good value while the text does not parse', async () => {
    const adjust = insertStep(newMacro('Adjust'), newStep('core.layer.add_adjustment', { operation: { type: 'contrast', amount: 0.25 } }));
    const { dialog, saved, onchange } = setup({ macros: [{ ...adjust, name: 'Adjust' }] });
    await open(dialog, 'New adjustment layer');
    const box = within(dialog).getByLabelText('operation') as HTMLTextAreaElement;
    expect(JSON.parse(box.value)).toEqual({ type: 'contrast', amount: 0.25 });
    await fireEvent.input(box, { target: { value: '{"type": "contrast", "amount": ' } });
    expect(within(dialog).getByText('That is not valid JSON, so the value has not changed.')).toBeTruthy();
    // Nothing was saved: the value in force is still the last good one.
    expect(onchange).not.toHaveBeenCalled();
    await fireEvent.input(box, { target: { value: '{"type": "contrast", "amount": 0.75}' } });
    expect(saved()[0].steps[0].params.operation).toEqual({ type: 'contrast', amount: 0.75 });
    expect(within(dialog).queryByText(/not valid JSON/)).toBeNull();
    // Valid JSON that is not a real edit is reported as a problem with the macro, not accepted.
    await fireEvent.input(box, { target: { value: '{"type": "contrast", "amount": "lots"}' } });
    expect(within(dialog).getByRole('alert', { name: 'Problems with this macro' })).toBeTruthy();
  });

  it('says a step is from a newer version when its operation is unknown, and lets it be removed', async () => {
    const odd = insertStep(newMacro('Odd'), newStep('core.layer.teleport', {}));
    const { dialog, saved } = setup({ macros: [{ ...odd, name: 'Odd' }] });
    await fireEvent.click(within(dialog).getByRole('button', { name: /^1\. core\.layer\.teleport/ }));
    expect(within(dialog).getAllByRole('alert').some((alert) => /not an operation this version of PhotoForge has/.test(alert.textContent ?? ''))).toBe(true);
    expect(button(dialog, 'Run').disabled).toBe(true);
    await fireEvent.click(button(dialog, 'Remove step 1'));
    expect(saved()[0].steps).toEqual([]);
  });
});

describe('conditions', () => {
  it('adds, changes and removes a condition, and reverses it with one box', async () => {
    const { dialog, saved } = setup();
    await open(dialog, 'Set layer opacity');
    const kind = within(dialog).getByLabelText('Condition') as HTMLSelectElement;
    expect(kind.value).toBe('');
    await fireEvent.change(kind, { target: { value: 'layer_count_at_least' } });
    expect(saved()[0].steps[0].when).toEqual({ kind: 'layer_count_at_least', count: 1 });
    await fireEvent.change(within(dialog).getByLabelText('Layers'), { target: { value: '3' } });
    expect(saved()[0].steps[0].when).toEqual({ kind: 'layer_count_at_least', count: 3 });
    // An impossible count is not taken.
    await fireEvent.change(within(dialog).getByLabelText('Layers'), { target: { value: '9999' } });
    expect(saved()[0].steps[0].when).toEqual({ kind: 'layer_count_at_least', count: 3 });
    await fireEvent.click(within(dialog).getByLabelText(/Reverse it/));
    expect(saved()[0].steps[0].when).toEqual({ kind: 'not', condition: { kind: 'layer_count_at_least', count: 3 } });
    await fireEvent.change(kind, { target: { value: 'precision' } });
    expect(saved()[0].steps[0].when).toEqual({ kind: 'not', condition: { kind: 'precision', precision: 'linear_srgb_f32' } });
    await fireEvent.change(within(dialog).getByLabelText('Precision'), { target: { value: 'legacy_srgb8' } });
    expect(saved()[0].steps[0].when).toEqual({ kind: 'not', condition: { kind: 'precision', precision: 'legacy_srgb8' } });
    await fireEvent.change(kind, { target: { value: 'active_layer_kind' } });
    await fireEvent.change(within(dialog).getByLabelText('Kind'), { target: { value: 'group' } });
    expect(saved()[0].steps[0].when).toEqual({ kind: 'not', condition: { kind: 'active_layer_kind', layerKind: 'group' } });
    await fireEvent.change(kind, { target: { value: '' } });
    expect(saved()[0].steps[0].when).toBeNull();
  });

  it('names the layer a condition asks about, the same way a step does', async () => {
    const { dialog, saved } = setup();
    await open(dialog, 'Set layer opacity');
    await fireEvent.change(within(dialog).getByLabelText('Condition'), { target: { value: 'layer_exists' } });
    await fireEvent.change(within(dialog).getByRole('combobox', { name: 'Layer in the condition' }), { target: { value: 'name' } });
    await fireEvent.input(within(dialog).getByLabelText('Layer in the condition: layer name'), { target: { value: 'Sky' } });
    expect(saved()[0].steps[0].when).toEqual({ kind: 'layer_exists', selector: { type: 'name', name: 'Sky' } });
  });

  it('shows a condition it cannot edit in words, and removes it rather than half-editing it', async () => {
    const deep = newStep('core.layer.add_group', {}, {
      kind: 'not', condition: { kind: 'not', condition: { kind: 'layer_count_at_least', count: 2 } }
    });
    const { dialog, saved } = setup({ macros: [{ ...insertStep(newMacro('Deep'), deep), name: 'Deep' }] });
    await open(dialog, 'New group');
    expect(within(dialog).getByText('Only if not (not (the document has at least 2 layers)).')).toBeTruthy();
    expect(within(dialog).queryByLabelText('Condition')).toBeNull();
    await fireEvent.click(button(dialog, 'Remove condition'));
    expect(saved()[0].steps[0].when).toBeNull();
  });

  it('marks a conditional step in the list, and says what the condition is', () => {
    const { dialog } = setup();
    expect(within(dialog).getByText('if')).toBeTruthy();
    expect(within(dialog).getByText('Only if the layer named “Sky” exists, when this step is reached.')).toBeTruthy();
  });
});

describe('macros', () => {
  it('makes, copies and deletes macros, asking twice before deleting', async () => {
    const { dialog, saved } = setup();
    await fireEvent.click(button(dialog, 'New macro'));
    expect(saved()).toHaveLength(2);
    expect((within(dialog).getByLabelText('Name') as HTMLInputElement).value).toBe('New macro 2');
    await fireEvent.click(button(dialog, 'Duplicate'));
    expect(saved().map((made) => made.name)).toEqual(['Dim the sky', 'New macro 2', 'New macro 2 copy']);
    await fireEvent.click(button(dialog, 'Delete macro'));
    // Once is a question, not a deletion.
    expect(saved()).toHaveLength(3);
    await fireEvent.click(button(dialog, 'Confirm: delete this macro'));
    expect(saved().map((made) => made.name)).toEqual(['Dim the sky', 'New macro 2']);
  });

  it('forgets a half-made deletion when another macro is chosen', async () => {
    const { dialog, saved } = setup({ macros: [macro(), { ...macro(), id: 'other', name: 'Other' }] });
    await fireEvent.click(button(dialog, 'Delete macro'));
    await fireEvent.click(within(dialog).getByRole('button', { name: /^Other/ }));
    expect(within(dialog).getByRole('button', { name: 'Delete macro' })).toBeTruthy();
    expect(saved()).toBeUndefined();
  });

  it('tells the person what a recording left out before they trust it', () => {
    const { dialog } = setup({ notices: ['Editing text cannot be recorded, so it is not in this macro.'] });
    expect(within(dialog).getByRole('note').textContent).toContain('Editing text cannot be recorded');
  });

  it('opens on the macro it is told to', () => {
    const { dialog } = setup({ macros: [macro(), { ...macro(), id: 'wanted', name: 'Wanted' }], selectedId: 'wanted' });
    expect((within(dialog).getByLabelText('Name') as HTMLInputElement).value).toBe('Wanted');
    expect(within(dialog).getByRole('button', { name: /^Wanted/ }).getAttribute('aria-current')).toBe('true');
  });

  it('explains itself when there are no macros, and still shows an import failure', async () => {
    const onimport = vi.fn().mockRejectedValue(new Error('The macro in the file is malformed.'));
    const { dialog } = setup({ macros: [], onimport });
    expect(within(dialog).getByText('No macros yet.')).toBeTruthy();
    expect(within(dialog).getByText(/can do nothing the editor itself could not/)).toBeTruthy();
    await fireEvent.click(button(dialog, 'Import…'));
    expect((await within(dialog).findByRole('alert')).textContent).toContain('malformed');
  });

  it('adds an imported macro as a new one and says nothing in it has run', async () => {
    const imported = { ...macro(), id: 'imported', name: 'Brought in' };
    const onimport = vi.fn().mockResolvedValue(imported);
    const { dialog, saved } = setup({ onimport });
    await fireEvent.click(button(dialog, 'Import…'));
    expect(await within(dialog).findByText(/Imported “Brought in”\. It is a new macro; nothing it contains has run\./)).toBeTruthy();
    expect(saved().map((made) => made.id)).toContain('imported');
    expect((within(dialog).getByLabelText('Name') as HTMLInputElement).value).toBe('Brought in');
  });

  it('exports the macro shown and reports where it went', async () => {
    const onexport = vi.fn().mockResolvedValue(String.raw`C:\out\m.json`);
    const { dialog } = setup({ onexport });
    await fireEvent.click(button(dialog, 'Export…'));
    expect(await within(dialog).findByText(/Saved “Dim the sky” to C:\\out\\m\.json\./)).toBeTruthy();
    expect(onexport.mock.calls[0][0].name).toBe('Dim the sky');
  });

  it('says nothing when a file dialog is cancelled', async () => {
    const { dialog } = setup();
    await fireEvent.click(button(dialog, 'Export…'));
    await fireEvent.click(button(dialog, 'Import…'));
    await waitFor(() => expect(button(dialog, 'Import…').disabled).toBe(false));
    expect(within(dialog).queryByRole('status')).toBeNull();
    expect(within(dialog).queryByRole('alert')).toBeNull();
  });

  it('starts a recording only with an image open', async () => {
    const withImage = setup();
    await fireEvent.click(button(withImage.dialog, 'Record…'));
    expect(withImage.onrecord).toHaveBeenCalledTimes(1);
    withImage.unmount();
    const without = setup({ hasDocument: false });
    expect(button(without.dialog, 'Record…').disabled).toBe(true);
  });
});

describe('the dialog as something to use without a mouse', () => {
  it('closes on Escape and from the close button, and puts focus inside when it opens', async () => {
    const { dialog, onclose } = setup();
    expect(dialog.contains(document.activeElement)).toBe(true);
    await fireEvent.keyDown(dialog, { key: 'Escape' });
    await fireEvent.click(button(dialog, 'Close automation'));
    expect(onclose).toHaveBeenCalledTimes(2);
  });

  it('names its controls: the steps list, each step, the macro list, the current macro', async () => {
    const { dialog } = setup();
    expect(within(dialog).getByRole('navigation', { name: 'Macros' })).toBeTruthy();
    expect(within(dialog).getByRole('list', { name: 'Steps' }).querySelectorAll(':scope > li')).toHaveLength(2);
    const toggle = within(dialog).getByRole('button', { name: /^1\. Set layer opacity/ });
    expect(toggle.getAttribute('aria-expanded')).toBe('false');
    await fireEvent.click(toggle);
    expect(toggle.getAttribute('aria-expanded')).toBe('true');
    expect(document.getElementById(toggle.getAttribute('aria-controls')!)).toBeTruthy();
    expect(within(dialog).getByRole('button', { name: /^Dim the sky/ }).getAttribute('aria-current')).toBe('true');
  });

  it('gives every input a label a screen reader will read', async () => {
    const { dialog } = setup();
    await open(dialog, 'Set layer opacity');
    await open(dialog, 'Rename layer');
    for (const control of dialog.querySelectorAll('input, select, textarea')) {
      const labelled = Boolean(
        control.getAttribute('aria-label') ||
          control.getAttribute('aria-labelledby') ||
          (control.id && dialog.querySelector(`label[for="${control.id}"]`))
      );
      expect(labelled, control.outerHTML).toBe(true);
    }
  });
});
