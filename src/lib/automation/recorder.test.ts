import { describe, expect, it } from 'vitest';
import registry from '../../../src-tauri/tests/fixtures/operation_registry.json';
import { createDocument, createGroupLayer, createPixelLayer } from '../layers/tree';
import type { OperationSpec } from '../operations/types';
import { MAX_MACRO_STEPS, newMacro, toCalls, validateMacro } from './macros';
import { Recorder, selectorFor } from './recorder';

const specs = registry as unknown as OperationSpec[];

function document() {
  return createDocument(8, 8, [
    createPixelLayer('Sky', 'px1', 8, 8),
    createPixelLayer('Twin', 'px2', 8, 8),
    createPixelLayer('Twin', 'px3', 8, 8),
    createPixelLayer('   ', 'px4', 8, 8)
  ]);
}

describe('naming a layer in a recording', () => {
  it('uses the name when nothing else shares it, and the identifier when something does', () => {
    const doc = document();
    const [sky, twin, , blank] = doc.layers;
    expect(selectorFor(doc, sky.id)).toEqual({ selector: { type: 'name', name: 'Sky' }, portable: true });
    expect(selectorFor(doc, twin.id)).toEqual({ selector: { type: 'id', id: twin.id }, portable: false });
    expect(selectorFor(doc, blank.id).portable).toBe(false);
    expect(selectorFor(doc, 'not-a-layer').selector).toEqual({ type: 'id', id: 'not-a-layer' });
  });

  it('finds a layer inside a group', () => {
    const group = { ...createGroupLayer('Folder'), content: { type: 'group' as const, children: [createPixelLayer('Deep', 'px9', 8, 8)], isolated: true } };
    const doc = createDocument(8, 8, [group]);
    const deep = (group.content as { children: { id: string }[] }).children[0];
    expect(selectorFor(doc, deep.id)).toEqual({ selector: { type: 'name', name: 'Deep' }, portable: true });
  });
});

describe('the recorder', () => {
  it('records nothing until it is started, and nothing after it stops', () => {
    const recorder = new Recorder();
    const doc = document();
    recorder.recordOnLayer(doc, doc.layers[0].id, 'core.layer.set_visible', { visible: false });
    recorder.skip('Editing text');
    expect(recorder.snapshot).toEqual({ recording: false, steps: [], notes: [] });
    recorder.start();
    recorder.recordOnLayer(doc, doc.layers[0].id, 'core.layer.set_visible', { visible: false });
    const { steps } = recorder.stop();
    expect(steps).toHaveLength(1);
    recorder.recordOnLayer(doc, doc.layers[0].id, 'core.layer.set_visible', { visible: true });
    expect(recorder.snapshot.steps).toHaveLength(1);
    expect(recorder.snapshot.recording).toBe(false);
  });

  it('turns a drag into one step holding the last value', () => {
    const recorder = new Recorder();
    const doc = document();
    const sky = doc.layers[0].id;
    recorder.start();
    for (const opacity of [0.9, 0.7, 0.5, 0.3]) {
      recorder.recordOnLayer(doc, sky, 'core.layer.set_opacity', { opacity }, { gestureKey: `layer-opacity:${sky}` });
    }
    expect(recorder.snapshot.steps).toHaveLength(1);
    expect(recorder.snapshot.steps[0].params).toEqual({ selector: { type: 'name', name: 'Sky' }, opacity: 0.3 });
  });

  it('does not merge a drag with another gesture, another layer, or a later separate drag', () => {
    const recorder = new Recorder();
    const doc = document();
    const [sky, twin] = [doc.layers[0].id, doc.layers[1].id];
    recorder.start();
    recorder.recordOnLayer(doc, sky, 'core.layer.set_opacity', { opacity: 0.5 }, { gestureKey: `layer-opacity:${sky}` });
    recorder.recordOnLayer(doc, twin, 'core.layer.set_opacity', { opacity: 0.5 }, { gestureKey: `layer-opacity:${twin}` });
    recorder.recordOnLayer(doc, sky, 'core.layer.set_opacity', { opacity: 0.2 }, { gestureKey: `layer-opacity:${sky}` });
    recorder.recordOnLayer(doc, sky, 'core.layer.set_visible', { visible: false });
    recorder.recordOnLayer(doc, sky, 'core.layer.set_opacity', { opacity: 0.1 }, { gestureKey: `layer-opacity:${sky}` });
    expect(recorder.snapshot.steps.map((step) => step.op)).toEqual([
      'core.layer.set_opacity',
      'core.layer.set_opacity',
      'core.layer.set_opacity',
      'core.layer.set_visible',
      'core.layer.set_opacity'
    ]);
  });

  it('never merges operations that are not continuous gestures, even with a key', () => {
    const recorder = new Recorder();
    const doc = document();
    const sky = doc.layers[0].id;
    recorder.start();
    recorder.recordOnLayer(doc, sky, 'core.layer.rename', { name: 'A' }, { gestureKey: 'k' });
    recorder.recordOnLayer(doc, sky, 'core.layer.rename', { name: 'B' }, { gestureKey: 'k' });
    expect(recorder.snapshot.steps).toHaveLength(2);
  });

  it('says so when it recorded a layer by identifier, once', () => {
    const recorder = new Recorder();
    const doc = document();
    recorder.start();
    recorder.recordOnLayer(doc, doc.layers[1].id, 'core.layer.set_visible', { visible: false });
    recorder.recordOnLayer(doc, doc.layers[2].id, 'core.layer.set_visible', { visible: false });
    expect(recorder.snapshot.notes.filter((note) => note.kind === 'identifier')).toHaveLength(1);
    expect(recorder.snapshot.steps[0].params.selector).toEqual({ type: 'id', id: doc.layers[1].id });
  });

  it('says what it could not record rather than leaving it out silently', () => {
    const recorder = new Recorder();
    recorder.start();
    recorder.skip('Editing text');
    recorder.skip('Editing text');
    recorder.skip('Placing an image');
    const notes = recorder.snapshot.notes;
    expect(notes.map((note) => note.message)).toEqual([
      'Editing text cannot be recorded, so it is not in this macro.',
      'Placing an image cannot be recorded, so it is not in this macro.'
    ]);
  });

  it('stops at the step limit and says so', () => {
    const recorder = new Recorder();
    recorder.start();
    for (let index = 0; index < MAX_MACRO_STEPS + 5; index += 1) recorder.record('core.layer.add_group', {});
    expect(recorder.snapshot.steps).toHaveLength(MAX_MACRO_STEPS);
    expect(recorder.snapshot.notes.some((note) => note.kind === 'full')).toBe(true);
  });

  it('keeps recording across a skipped action without merging around it', () => {
    const recorder = new Recorder();
    const doc = document();
    const sky = doc.layers[0].id;
    recorder.start();
    recorder.recordOnLayer(doc, sky, 'core.layer.set_opacity', { opacity: 0.5 }, { gestureKey: 'g' });
    recorder.skip('Editing text');
    recorder.recordOnLayer(doc, sky, 'core.layer.set_opacity', { opacity: 0.2 }, { gestureKey: 'g' });
    expect(recorder.snapshot.steps).toHaveLength(2);
  });

  it('notifies subscribers and stops notifying after unsubscribe', () => {
    const recorder = new Recorder();
    const seen: number[] = [];
    const off = recorder.subscribe((state) => seen.push(state.steps.length));
    recorder.start();
    recorder.record('core.layer.add_group', {});
    off();
    recorder.record('core.layer.add_group', {});
    expect(seen).toEqual([0, 0, 1]);
  });

  it('discard forgets everything', () => {
    const recorder = new Recorder();
    recorder.start();
    recorder.record('core.layer.add_group', {});
    recorder.skip('Painting');
    recorder.discard();
    expect(recorder.snapshot).toEqual({ recording: false, steps: [], notes: [] });
  });

  it('records several layers as one list of selectors, with the same honesty about identifiers', () => {
    const recorder = new Recorder();
    const doc = document();
    recorder.start();
    recorder.recordOnLayers(doc, [doc.layers[0].id, doc.layers[1].id], 'core.layer.group', { name: 'Group' });
    const [step] = recorder.snapshot.steps;
    expect(step.params).toEqual({
      selectors: [{ type: 'name', name: 'Sky' }, { type: 'id', id: doc.layers[1].id }],
      name: 'Group'
    });
    expect(recorder.snapshot.notes.map((note) => note.kind)).toEqual(['identifier']);
    const quiet = new Recorder();
    quiet.recordOnLayers(doc, [doc.layers[0].id], 'core.layer.group', {});
    expect(quiet.snapshot.steps).toEqual([]);
  });

  it('produces steps the registry accepts, ready to send as one transaction', () => {
    const recorder = new Recorder();
    const doc = document();
    const sky = doc.layers[0].id;
    recorder.start();
    recorder.recordOnLayer(doc, sky, 'core.layer.set_opacity', { opacity: 0.4 }, { gestureKey: 'g' });
    recorder.recordOnLayer(doc, sky, 'core.layer.set_blend_mode', { blendMode: 'multiply' });
    recorder.recordOnLayer(doc, sky, 'core.layer.rename', { name: 'Dim sky' });
    recorder.recordOnLayer(doc, sky, 'core.layer.move', { parent: null, index: 0 });
    recorder.record('core.layer.add_group', { name: 'Folder' });
    recorder.recordOnLayers(doc, [sky, doc.layers[1].id], 'core.layer.group', { name: 'Group' });
    recorder.record('core.document.flatten', {});
    const macro = { ...newMacro('Recorded'), steps: recorder.stop().steps };
    expect(validateMacro(macro, specs)).toEqual([]);
    expect(toCalls(macro).map((call) => call.op)).toEqual([
      'core.layer.set_opacity',
      'core.layer.set_blend_mode',
      'core.layer.rename',
      'core.layer.move',
      'core.layer.add_group',
      'core.layer.group',
      'core.document.flatten'
    ]);
  });
});
