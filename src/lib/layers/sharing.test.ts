import { describe, expect, it } from 'vitest';
import { createDocument, createGroupLayer, createPixelLayer } from './tree';
import type { Layer, LayerDocument } from './types';
import { equal, shareUnchanged } from './sharing';

/** What coming back from the engine does to a document: every object is new. */
const roundTrip = <T>(value: T): T => JSON.parse(JSON.stringify(value));

function sample(): LayerDocument {
  const masked: Layer = {
    ...createPixelLayer('Masked', 'px1', 8, 8),
    id: 'masked',
    mask: { snapshot: { width: 8, height: 8, data: 'AAAA'.repeat(1000) } as never, enabled: true, inverted: false }
  };
  const leaf = { ...createPixelLayer('Leaf', 'px2', 4, 4), id: 'leaf' };
  const group = { ...createGroupLayer('Group', [leaf]), id: 'group' };
  const plain = { ...createPixelLayer('Plain', 'px3', 8, 8), id: 'plain' };
  return createDocument(8, 8, [masked, group, plain]);
}

describe('shareUnchanged', () => {
  it('gives every layer that did not change back its old identity', () => {
    const before = sample();
    const after = shareUnchanged(before, roundTrip(before));
    expect(after.layers[0]).toBe(before.layers[0]);
    expect(after.layers[1]).toBe(before.layers[1]);
    expect(after.layers[2]).toBe(before.layers[2]);
    expect(after.layers).toBe(before.layers);
  });

  it('shares what did not change and takes what did', () => {
    const before = sample();
    const returned = roundTrip(before);
    returned.layers[2] = { ...returned.layers[2], opacity: 0.5 };
    const after = shareUnchanged(before, returned);
    expect(after.layers[0]).toBe(before.layers[0]);
    expect(after.layers[1]).toBe(before.layers[1]);
    expect(after.layers[2]).not.toBe(before.layers[2]);
    expect(after.layers[2].opacity).toBe(0.5);
  });

  it('keeps the unchanged children of a group that did change', () => {
    const before = sample();
    const returned = roundTrip(before);
    const group = returned.layers[1];
    if (group.content.type !== 'group') throw new Error('expected a group');
    const extra = { ...createPixelLayer('Extra', 'px4', 4, 4), id: 'extra' };
    returned.layers[1] = { ...group, content: { ...group.content, children: [...group.content.children, extra] } };
    const after = shareUnchanged(before, returned);
    const oldGroup = before.layers[1].content;
    const newGroup = after.layers[1].content;
    if (oldGroup.type !== 'group' || newGroup.type !== 'group') throw new Error('expected groups');
    expect(after.layers[1]).not.toBe(before.layers[1]);
    expect(newGroup.children[0]).toBe(oldGroup.children[0]);
    expect(newGroup.children[1].id).toBe('extra');
  });

  it('never merges layers that differ', () => {
    const before = sample();
    const returned = roundTrip(before);
    returned.layers[0] = { ...returned.layers[0], name: 'Renamed' };
    const after = shareUnchanged(before, returned);
    expect(after.layers[0].name).toBe('Renamed');
    expect(before.layers[0].name).toBe('Masked');
  });

  it('keeps the document-level fields of the new document', () => {
    const before = sample();
    const returned = { ...roundTrip(before), activeLayerId: 'plain' };
    expect(shareUnchanged(before, returned).activeLayerId).toBe('plain');
  });
});

describe('equal', () => {
  it('treats absent and null as the same, and nothing else as the same', () => {
    expect(equal({ a: 1, b: null }, { a: 1 })).toBe(true);
    expect(equal({ a: 1 }, { a: 2 })).toBe(false);
    expect(equal([1, 2], [1, 2, 3])).toBe(false);
    expect(equal({ a: [1, { b: 2 }] }, { a: [1, { b: 2 }] })).toBe(true);
    expect(equal(0, null)).toBe(false);
    expect(equal('x', 'y')).toBe(false);
  });
});
