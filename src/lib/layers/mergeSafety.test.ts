import { describe, expect, it } from 'vitest';
import { createAdjustmentLayer, createDocument, createGroupLayer, createPixelLayer } from './tree';
import { mergeSafetyProblem } from './mergeSafety';
import type { Layer } from './types';

const pixel = (name: string): Layer => createPixelLayer(name, `px-${name}`, 8, 8);
const clipped = (layer: Layer): Layer => ({ ...layer, clipped: true });
/** A document whose stack, bottom to top, is `layers`; ids are the layers' own. */
const stack = (...layers: Layer[]) => createDocument(8, 8, layers);

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

  describe('with clipping', () => {
    it('merges a clipped layer into its base, with nothing clipped above', () => {
      const [ground, base, ink] = [pixel('ground'), pixel('base'), clipped(pixel('ink'))];
      expect(mergeSafetyProblem(stack(ground, base, ink), [base.id, ink.id])).toBeNull();
    });

    it('refuses to merge a base into a layer clipped to the one below', () => {
      const [a, b, c] = [pixel('a'), clipped(pixel('b')), pixel('c')];
      expect(mergeSafetyProblem(stack(a, b, c), [b.id, c.id])).toMatch(/split a clipping group/);
    });

    it('refuses to merge a base that has layers clipped to it', () => {
      // [a, b, c, x (clipped to c)], Merge down on c.
      const [a, b, c, x] = [pixel('a'), pixel('b'), pixel('c'), clipped(pixel('x'))];
      expect(mergeSafetyProblem(stack(a, b, c, x), [b.id, c.id])).toMatch(/split a clipping group/);
      // [ground, base, ink (clipped)], Merge down on base.
      const [ground, base, ink] = [pixel('ground'), pixel('base'), clipped(pixel('ink'))];
      expect(mergeSafetyProblem(stack(ground, base, ink), [ground.id, base.id])).toMatch(/split a clipping group/);
    });

    it('merges a clipped layer into the base below while others stay clipped above it', () => {
      const [ground, base, ink, tint] = [pixel('ground'), pixel('base'), clipped(pixel('ink')), clipped(pixel('tint'))];
      expect(mergeSafetyProblem(stack(ground, base, ink, tint), [base.id, ink.id])).toBeNull();
    });

    it('merges clipped layers into each other', () => {
      const [ground, base, c1, d, e] = [pixel('ground'), pixel('base'), clipped(pixel('c1')), clipped(pixel('d')), clipped(pixel('e'))];
      expect(mergeSafetyProblem(stack(ground, base, c1, d), [c1.id, d.id])).toBeNull();
      expect(mergeSafetyProblem(stack(ground, base, c1, d, e), [c1.id, d.id])).toBeNull();
    });

    it('merges a whole clipping group, or several', () => {
      const [a, c, x, y] = [pixel('a'), pixel('c'), clipped(pixel('x')), pixel('y')];
      expect(mergeSafetyProblem(stack(a, c, x, y), [c.id, x.id])).toBeNull();
      expect(mergeSafetyProblem(stack(a, c, x, y), [a.id, c.id, x.id, y.id])).toBeNull();
    });

    it('refuses a range that starts inside a group and reaches another base', () => {
      const [a, b, c, d] = [pixel('a'), clipped(pixel('b')), pixel('c'), pixel('d')];
      expect(mergeSafetyProblem(stack(a, b, c, d), [b.id, c.id, d.id])).toMatch(/split a clipping group/);
    });

    it('does not count the flag of the bottom layer, which clips to nothing', () => {
      const [a, b] = [clipped(pixel('a')), clipped(pixel('b'))];
      expect(mergeSafetyProblem(stack(a, b), [a.id, b.id])).toBeNull();
    });

    it('refuses layers clipped to an adjustment layer, which would be clipped to a pixel layer', () => {
      const adjust = createAdjustmentLayer('Grey', { type: 'grayscale' } as never);
      const [x, y] = [clipped(pixel('x')), clipped(pixel('y'))];
      expect(mergeSafetyProblem(stack(adjust, x, y), [adjust.id, x.id])).toMatch(/clipped to an adjustment layer/);
      // With nothing clipped above the pair, there is nothing to re-attach.
      expect(mergeSafetyProblem(stack(adjust, x), [adjust.id, x.id])).toBeNull();
    });

    it('applies inside a group as well', () => {
      const [base, ink, tint] = [pixel('base'), clipped(pixel('ink')), clipped(pixel('tint'))];
      const group = createGroupLayer('Group', [base, ink, tint]);
      const document = stack(pixel('ground'), group);
      expect(mergeSafetyProblem(document, [base.id, ink.id])).toBeNull();
      const [a, b, x] = [pixel('a'), pixel('b'), clipped(pixel('x'))];
      const refused = stack(pixel('ground'), createGroupLayer('Group', [a, b, x]));
      expect(mergeSafetyProblem(refused, [a.id, b.id])).toMatch(/split a clipping group/);
    });
  });
});
