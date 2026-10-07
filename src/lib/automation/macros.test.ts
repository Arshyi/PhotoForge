import { describe, expect, it } from 'vitest';
import { blendModes } from '../layers/types';
import registry from '../../../src-tauri/tests/fixtures/operation_registry.json';
import type { LayerSelector, OperationSpec } from '../operations/types';
import type { PluginSummary } from '../plugins/types';
import {
  MACRO_STORAGE_KEY,
  MAX_MACRO_STEPS,
  describeCondition,
  describeStep,
  defaultParams,
  defaultValue,
  dependencyStatus,
  duplicateMacro,
  duplicateStep,
  insertStep,
  loadMacros,
  macroDocument,
  macroPluginDependencies,
  moveStep,
  newMacro,
  newStep,
  normaliseMacro,
  parseMacroDocument,
  removeStep,
  saveMacros,
  setMacroDetails,
  setStepEnabled,
  toCalls,
  updateStep,
  validateCondition,
  validateMacro,
  type Macro
} from './macros';

// The real registry, as the backend publishes it. A macro the editor calls valid
// must be one the backend's own descriptions of its operations accept.
const specs = registry as unknown as OperationSpec[];

const sky: LayerSelector = { type: 'name', name: 'Sky' };

function sample(): Macro {
  let macro = newMacro('Dim the sky');
  macro = insertStep(macro, newStep('core.layer.set_opacity', { selector: sky, opacity: 0.5 }, { kind: 'layer_exists', selector: sky }));
  macro = insertStep(macro, newStep('core.layer.rename', { selector: sky, name: 'Dim sky' }));
  macro = insertStep(macro, newStep('core.layer.add_group', {}));
  return macro;
}

describe('editing a macro', () => {
  it('returns new macros and leaves the one it was given alone', () => {
    const before = sample();
    const snapshot = JSON.stringify(before);
    const [first] = before.steps;
    moveStep(before, first.id, 2);
    removeStep(before, first.id);
    setStepEnabled(before, first.id, false);
    duplicateStep(before, first.id);
    updateStep(before, first.id, { note: 'changed' });
    expect(JSON.stringify(before)).toBe(snapshot);
  });

  it('changes the name or description as a modification, touching nothing else', () => {
    const macro = sample();
    const later = new Date(Date.parse(macro.modifiedAt) + 60_000);
    const renamed = setMacroDetails(macro, { name: 'New name' }, later);
    expect(renamed).toMatchObject({ name: 'New name', description: macro.description, id: macro.id, createdAt: macro.createdAt });
    expect(renamed.modifiedAt).toBe(later.toISOString());
    expect(renamed.steps).toBe(macro.steps);
    expect(setMacroDetails(macro, { description: 'Why' }, later).name).toBe(macro.name);
    expect(macro.name).toBe('Dim the sky');
  });

  it('moves a step, clamping at the ends and ignoring one that is not there', () => {
    const macro = sample();
    const [a, b, c] = macro.steps.map((step) => step.id);
    expect(moveStep(macro, a, 1).steps.map((s) => s.id)).toEqual([b, a, c]);
    expect(moveStep(macro, c, -10).steps.map((s) => s.id)).toEqual([c, a, b]);
    expect(moveStep(macro, a, -1)).toBe(macro);
    expect(moveStep(macro, 'nope', 1)).toBe(macro);
  });

  it('duplicates a step beside itself with a new identifier and its own copy of the parameters', () => {
    const macro = sample();
    const copied = duplicateStep(macro, macro.steps[0].id);
    expect(copied.steps).toHaveLength(4);
    expect(copied.steps[1].id).not.toBe(copied.steps[0].id);
    expect(copied.steps[1].op).toBe(copied.steps[0].op);
    copied.steps[1].params.opacity = 0.9;
    expect(copied.steps[0].params.opacity).toBe(0.5);
  });

  it('will not grow past the step limit', () => {
    let macro = newMacro('Long');
    for (let i = 0; i < MAX_MACRO_STEPS; i += 1) macro = insertStep(macro, newStep('core.layer.add_group'));
    expect(() => insertStep(macro, newStep('core.layer.add_group'))).toThrow(/at most/);
  });

  it('duplicating a macro gives every step a new identifier and the macro a new one', () => {
    const macro = sample();
    const copy = duplicateMacro(macro);
    expect(copy.id).not.toBe(macro.id);
    expect(copy.name).toBe('Dim the sky copy');
    expect(new Set([...macro.steps, ...copy.steps].map((s) => s.id)).size).toBe(6);
  });
});

describe('what a macro sends', () => {
  it('is its enabled steps, in order, with their conditions and no editor-only fields', () => {
    let macro = sample();
    macro = setStepEnabled(macro, macro.steps[1].id, false);
    const calls = toCalls(macro);
    expect(calls.map((call) => call.op)).toEqual(['core.layer.set_opacity', 'core.layer.add_group']);
    expect(calls[0].when).toEqual({ kind: 'layer_exists', selector: sky });
    expect('when' in calls[1]).toBe(false);
    expect(Object.keys(calls[0]).sort()).toEqual(['op', 'params', 'when']);
  });

  it('sends copies, so running a macro cannot change it', () => {
    const macro = sample();
    const [call] = toCalls(macro);
    (call.params as Record<string, unknown>).opacity = 1;
    expect(macro.steps[0].params.opacity).toBe(0.5);
  });
});

describe('checking a macro against the registry', () => {
  it('accepts a well-formed one', () => {
    expect(validateMacro(sample(), specs)).toEqual([]);
  });

  it('names the step and the problem for each kind of mistake', () => {
    let macro = newMacro('Bad');
    macro = insertStep(macro, newStep('core.layer.teleport', {}));
    macro = insertStep(macro, newStep('core.layer.set_opacity', { selector: sky, opacity: 4 }));
    macro = insertStep(macro, newStep('core.layer.set_opacity', { selector: sky }));
    macro = insertStep(macro, newStep('core.layer.rename', { selector: sky, name: 'x', extra: 1 }));
    macro = insertStep(macro, newStep('core.layer.select', { selector: { type: 'eval', code: '1' } }));
    macro = insertStep(macro, newStep('core.layer.add_group', {}, { kind: 'eval' } as never));
    const problems = validateMacro(macro, specs);
    expect(problems).toHaveLength(6);
    expect(problems[0]).toMatch(/^Step 1: core\.layer\.teleport is not an operation/);
    expect(problems[1]).toMatch(/^Step 2 \(.*\): opacity must be between/);
    expect(problems[2]).toMatch(/^Step 3 \(.*\): opacity is required/);
    expect(problems[3]).toMatch(/^Step 4 \(.*\): extra is not a parameter/);
    expect(problems[4]).toMatch(/^Step 5 \(.*\): selector is not a layer selector/);
    expect(problems[5]).toMatch(/^Step 6 \(.*\): eval is not a condition/);
  });

  it('checks the edits inside an adjustment, not only that something is there', () => {
    let macro = newMacro('Edits');
    macro = insertStep(macro, newStep('core.layer.add_adjustment', { operation: { type: 'contrast', amount: 0.25 } }));
    expect(validateMacro(macro, specs)).toEqual([]);
    macro = insertStep(macro, newStep('core.layer.add_adjustment', { operation: { type: 'contrast', amount: 'lots' } }));
    macro = insertStep(macro, newStep('core.layer.apply_edit', { selector: sky, operations: [] }));
    const problems = validateMacro(macro, specs);
    expect(problems).toHaveLength(2);
    expect(problems[0]).toMatch(/Step 2/);
    expect(problems[1]).toMatch(/Step 3.*1 to 100 edits/);
  });

  it('refuses a duplicated step identifier, an empty name and an oversize note', () => {
    const macro = sample();
    const clash = { ...macro, name: ' ', steps: macro.steps.map((step) => ({ ...step, id: 'same', note: 'n'.repeat(201) })) };
    const problems = validateMacro(clash, specs);
    expect(problems.some((p) => /name must contain/.test(p))).toBe(true);
    expect(problems.some((p) => /shares its identifier/.test(p))).toBe(true);
    expect(problems.some((p) => /note is longer/.test(p))).toBe(true);
  });

  it('allows a null optional selector and refuses a null required one', () => {
    let macro = newMacro('Move');
    macro = insertStep(macro, newStep('core.layer.move', { selector: sky, parent: null, index: 0 }));
    expect(validateMacro(macro, specs)).toEqual([]);
    macro = insertStep(macro, newStep('core.layer.move', { selector: null as never, index: 0 }));
    expect(validateMacro(macro, specs)).toHaveLength(1);
  });

  it('accepts every blend mode the application has, and none it does not', () => {
    for (const mode of blendModes) {
      const macro = insertStep(newMacro('Blend'), newStep('core.layer.set_blend_mode', { selector: sky, blendMode: mode.id }));
      expect(validateMacro(macro, specs), mode.id).toEqual([]);
    }
    expect(blendModes.map((mode) => mode.id)).toContain('luminosity');
    const bad = insertStep(newMacro('Blend'), newStep('core.layer.set_blend_mode', { selector: sky, blendMode: 'glow' }));
    expect(validateMacro(bad, specs)).toHaveLength(1);
  });

  it('says every core operation in the registry has a parameter kind it knows how to check', () => {
    const known = new Set([
      'selector', 'selectors', 'optionalSelector', 'text', 'bool', 'number', 'integer', 'blendMode', 'editOperation', 'editOperations', 'json'
    ]);
    for (const spec of specs) for (const param of spec.params) expect(known.has(param.kind.kind), `${spec.id}.${param.name}`).toBe(true);
  });
});

describe('conditions', () => {
  it('mirror the backend: closed list, strict fields, shallow nesting', () => {
    const exists = { kind: 'layer_exists', selector: sky };
    for (const ok of [
      exists,
      { kind: 'layer_count_at_least', count: 3 },
      { kind: 'active_layer_kind', layerKind: 'group' },
      { kind: 'precision', precision: 'legacy_srgb8' },
      { kind: 'not', condition: { kind: 'not', condition: exists } }
    ]) expect(validateCondition(ok), JSON.stringify(ok)).toBeNull();
    for (const bad of [
      { kind: 'layer_count_at_least', count: 0 },
      { kind: 'layer_count_at_least', count: 513 },
      { kind: 'layer_count_at_least', count: 1.5 },
      { kind: 'layer_count_at_least', count: 3, extra: true },
      { kind: 'active_layer_kind', layerKind: 'photo' },
      { kind: 'precision', precision: 'float64' },
      { kind: 'layer_exists', selector: { type: 'name', name: '' } },
      { kind: 'layer_exists', selector: { type: 'name', name: 'x'.repeat(121) } },
      { kind: 'layer_exists', selector: { type: 'active', id: 'x' } },
      { kind: 'not', condition: { kind: 'not', condition: { kind: 'not', condition: exists } } },
      { kind: 'eval', code: '1+1' },
      'layer_exists',
      null
    ]) expect(validateCondition(bad), JSON.stringify(bad)).not.toBeNull();
  });

  it('are described in words a person can check', () => {
    expect(describeCondition({ kind: 'layer_exists', selector: sky })).toBe('the layer named “Sky” exists');
    expect(describeCondition({ kind: 'not', condition: { kind: 'layer_count_at_least', count: 1 } })).toBe(
      'not (the document has at least 1 layer)'
    );
    expect(describeCondition({ kind: 'active_layer_kind', layerKind: 'smart_object' })).toBe('the selected layer is a smart object layer');
    expect(describeCondition({ kind: 'precision', precision: 'linear_srgb_f32' })).toBe('the document is linear float');
  });

  it('describe a step by its operation title and its parameters', () => {
    const step = sample().steps[0];
    const { title, summary } = describeStep(step, specs);
    expect(title).toBe(specs.find((s) => s.id === step.op)?.title);
    expect(title).not.toBe(step.op);
    expect(summary).toContain('the layer named “Sky”');
    expect(summary).toContain('opacity: 0.5');
    expect(describeStep(newStep('core.nope'), specs).title).toBe('core.nope');
  });
});

describe('plugin dependencies', () => {
  it('lists each plugin a macro would need, once, however it is named', () => {
    let macro = newMacro('Uses plugins');
    macro = insertStep(macro, newStep('core.plugin.apply_filter', { selector: sky, plugin: 'com.example.b', filter: 'f' }));
    macro = insertStep(macro, newStep('core.plugin.add_adjustment', { plugin: 'com.example.a', filter: 'f' }));
    macro = insertStep(macro, newStep('core.plugin.add_adjustment', { plugin: 'com.example.a', filter: 'g' }));
    macro = insertStep(
      macro,
      newStep('core.layer.add_adjustment', { operation: { type: 'plugin_filter', plugin: 'com.example.c', filter: 'f' } })
    );
    macro = insertStep(
      macro,
      newStep('core.layer.apply_edit', {
        selector: sky,
        operations: [{ type: 'masked', operation: { type: 'plugin_filter', plugin: 'com.example.d' } }, { type: 'contrast', amount: 1 }]
      })
    );
    expect(macroPluginDependencies(macro)).toEqual(['com.example.a', 'com.example.b', 'com.example.c', 'com.example.d']);
    expect(macroPluginDependencies(sample())).toEqual([]);
  });
});

function memoryStorage(initial: Record<string, string> = {}) {
  const data = new Map(Object.entries(initial));
  return {
    getItem: (key: string) => data.get(key) ?? null,
    setItem: (key: string, value: string) => void data.set(key, value),
    data
  };
}

describe('storage', () => {
  it('round-trips macros', () => {
    const storage = memoryStorage();
    const macros = [sample(), newMacro('Empty')];
    expect(saveMacros(macros, storage)).toBe(true);
    expect(loadMacros(storage)).toEqual(macros);
  });

  it('says so when the browser will not keep them', () => {
    const storage = {
      setItem: () => {
        throw new Error('quota');
      }
    };
    expect(saveMacros([sample()], storage)).toBe(false);
  });

  it('reads nothing from damaged storage rather than failing, and drops only the entries that are bad', () => {
    expect(loadMacros(memoryStorage({ [MACRO_STORAGE_KEY]: '{not json' }))).toEqual([]);
    expect(loadMacros(memoryStorage({ [MACRO_STORAGE_KEY]: '{"a":1}' }))).toEqual([]);
    expect(loadMacros(memoryStorage())).toEqual([]);
    const good = sample();
    const stored = JSON.stringify([good, { schemaVersion: 2, name: 'future', steps: [] }, { schemaVersion: 1, name: 5, steps: [] }, 7]);
    expect(loadMacros(memoryStorage({ [MACRO_STORAGE_KEY]: stored }))).toEqual([good]);
  });

  it('refuses a stored macro with a malformed condition rather than dropping the condition', () => {
    const bad = JSON.parse(JSON.stringify(sample()));
    bad.steps[0].when = { kind: 'eval', code: '1' };
    expect(normaliseMacro(bad)).toBeNull();
  });

  it('never lets the storage read throw', () => {
    const storage = {
      getItem: () => {
        throw new Error('blocked');
      }
    };
    expect(loadMacros(storage)).toEqual([]);
  });
});

describe('macro files', () => {
  it('round-trip, and import as a new macro', () => {
    const macro = sample();
    const imported = parseMacroDocument(JSON.stringify(macroDocument(macro)));
    expect(imported.id).not.toBe(macro.id);
    expect({ ...imported, id: macro.id }).toEqual(macro);
  });

  it('refuse what is not exactly one macro', () => {
    const text = (value: unknown) => JSON.stringify(value);
    expect(() => parseMacroDocument('nope')).toThrow(/not valid JSON/);
    expect(() => parseMacroDocument(text({ kind: 'something', schemaVersion: 1 }))).toThrow(/not a PhotoForge macro/);
    expect(() => parseMacroDocument(text({ kind: 'photoforge-macro', schemaVersion: 2, macro: sample() }))).toThrow(/version/);
    expect(() => parseMacroDocument(text({ kind: 'photoforge-macro', schemaVersion: 1, macro: { steps: 'x' } }))).toThrow(/malformed/);
    expect(() => parseMacroDocument('x'.repeat(1_000_001))).toThrow(/larger than/);
    const tooMany = { ...sample(), steps: Array.from({ length: MAX_MACRO_STEPS + 1 }, () => newStep('core.layer.add_group')) };
    expect(() => parseMacroDocument(text(macroDocument(tooMany)))).toThrow(/malformed/);
  });

  it('carries no code: a file cannot add anything to a macro except registered operations and conditions', () => {
    const hostile = JSON.parse(JSON.stringify(macroDocument(sample())));
    hostile.macro.steps[0].script = 'fetch("https://example.invalid")';
    hostile.macro.steps[0].when = { kind: 'eval', code: 'process.exit()' };
    expect(() => parseMacroDocument(JSON.stringify(hostile))).toThrow(/malformed/);
    delete hostile.macro.steps[0].when;
    const parsed = parseMacroDocument(JSON.stringify(hostile));
    // The unknown field is dropped on the way in, not carried.
    expect('script' in parsed.steps[0]).toBe(false);
  });
});

describe('starting values', () => {
  it('give every operation in the registry a new step the checks accept, except for text only the person can supply', () => {
    for (const spec of specs) {
      const macro = insertStep(newMacro('Defaults'), newStep(spec.id, defaultParams(spec)));
      const problems = validateMacro(macro, specs);
      const textParams = spec.params.filter((param) => param.required && param.kind.kind === 'text');
      expect(problems.length, `${spec.id}: ${problems.join(' | ')}`).toBe(textParams.length);
      for (const problem of problems) expect(problem).toMatch(/needs 1 to \d+ characters/);
    }
  });

  it('start a number inside its range, even when the range excludes zero', () => {
    expect(defaultValue({ kind: 'number', min: 0.5, max: 2 })).toBe(0.5);
    expect(defaultValue({ kind: 'integer', min: -3, max: 3 })).toBe(0);
    expect(defaultValue({ kind: 'integer', min: -9, max: -4 })).toBe(-4);
  });

  it('give each step its own copy of a default', () => {
    const kind = { kind: 'selectors' } as const;
    const first = defaultValue(kind) as unknown[];
    first.push('x');
    expect(defaultValue(kind)).toHaveLength(1);
  });
});

describe('plugin status for a macro', () => {
  const plugin = (id: string, availability: PluginSummary['availability'], enabled = true): PluginSummary =>
    ({ id, enabled, granted: [], activeHash: 'h', versions: [], manifest: { name: `Plugin ${id}` }, availability, readme: null, license: null }) as unknown as PluginSummary;

  it('says which plugins can run now and why the others cannot', () => {
    const status = dependencyStatus(
      ['a', 'b', 'c', 'd', 'e', 'f', 'gone'],
      [
        plugin('a', { kind: 'available' }),
        plugin('b', { kind: 'disabled' }, false),
        plugin('c', { kind: 'notGranted', capability: 'document.operations' }),
        plugin('d', { kind: 'runtimeUnavailable' }),
        plugin('e', { kind: 'damaged', reason: 'bad zip' }),
        plugin('f', { kind: 'otherVersion', installedVersion: '2.0.0', installedHash: 'x' })
      ]
    );
    expect(status.map((entry) => entry.state)).toEqual(['ready', 'turned_off', 'unavailable', 'unavailable', 'unavailable', 'unavailable', 'missing']);
    expect(status[0]).toMatchObject({ name: 'Plugin a', reason: '' });
    expect(status[2].reason).toContain('document.operations');
    expect(status[4].reason).toContain('bad zip');
    expect(status[5].reason).toContain('2.0.0');
    expect(status[6]).toMatchObject({ id: 'gone', name: 'gone', reason: 'It is not installed.' });
  });
});
