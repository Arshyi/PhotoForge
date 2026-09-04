import { describe, expect, it } from 'vitest';
import { createDocument, createGroupLayer, createPixelLayer } from './tree';
import { mergeSafetyProblem } from './mergeSafety';

describe('merge safety', () => {
  it('accepts a contiguous range when its blend backdrop is included', () => {
    const bottom = createPixelLayer('Bottom', 'px1', 8, 8);
    const top = createPixelLayer('Top', 'px2', 8, 8);
    top.blendMode = 'multiply';
    expect(mergeSafetyProblem(createDocument(8, 8, [bottom, top]), [top.id, bottom.id])).toBeNull();
  });

  it('rejects a blend-dependent range that omits a lower sibling', () => {
    const bottom = createPixelLayer('Bottom', 'px1', 8, 8);
    const middle = createPixelLayer('Middle', 'px2', 8, 8);
    middle.blendMode = 'screen';
    const top = createPixelLayer('Top', 'px3', 8, 8);
    expect(mergeSafetyProblem(createDocument(8, 8, [bottom, middle, top]), [middle.id, top.id]))
      .toMatch(/depends on layers beneath/);
  });

  it('requires selected identifiers to be contiguous siblings', () => {
    const bottom = createPixelLayer('Bottom', 'px1', 8, 8);
    const top = createPixelLayer('Top', 'px2', 8, 8);
    const group = createGroupLayer('Group', [bottom, top]);
    expect(mergeSafetyProblem(createDocument(8, 8, [group]), [bottom.id, top.id])).toBeNull();
    const middle = createPixelLayer('Middle', 'px3', 8, 8);
    expect(mergeSafetyProblem(createDocument(8, 8, [bottom, middle, top]), [bottom.id, top.id]))
      .toMatch(/contiguous sibling/);
  });
});
