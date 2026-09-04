import { cleanup, render, screen, within } from '@testing-library/svelte';
import { afterEach, describe, expect, it, vi } from 'vitest';
import LayersPanel from './LayersPanel.svelte';
import TransformPanel from './TransformPanel.svelte';
import TransformOverlay from './TransformOverlay.svelte';
import AdjustmentLayerDialog from './AdjustmentLayerDialog.svelte';
import CurveEditor from './CurveEditor.svelte';
import { createDocument, createGroupLayer, createPixelLayer } from '../layers/tree';
import { identityCurves } from '../layers/adjustments';
import { identityTransform, type Layer, type LayerDocument } from '../layers/types';

/**
 * Keyboard and assistive-technology coverage for the layer surfaces.
 *
 * This is not a full audit and does not replace one with a screen reader. It
 * pins the things a Phase 8 change could quietly break: that every control has
 * a name, that the tree exposes its hierarchy, that a toggle reports its state,
 * and that nothing interactive is reachable only with a pointer.
 */

function pixelLayer(name: string): Layer {
  return createPixelLayer(name, `px${name}`, 16, 16);
}

function panelDocument(): LayerDocument {
  const background = pixelLayer('Background');
  const inner = createGroupLayer('Inner', [pixelLayer('Leaf')]);
  const group = createGroupLayer('Outer', [inner, pixelLayer('Cloud')]);
  const document = createDocument(16, 16, [background, group]);
  return { ...document, activeLayerId: background.id };
}

function panelProps(document: LayerDocument) {
  return {
    document,
    thumbnails: {} as Record<string, string>,
    editTarget: 'layer' as const,
    adjustmentTarget: 'document' as const,
    disabled: false,
    busy: false,
    hasSelection: false,
    selectedIds: [] as string[],
    onselect: vi.fn(),
    ontoggle: vi.fn(),
    onrename: vi.fn(),
    onopacity: vi.fn(),
    onblend: vi.fn(),
    onreorder: vi.fn(),
    oncreate: vi.fn(),
    onaction: vi.fn(),
    ontargetchange: vi.fn(),
    onadjustmenttargetchange: vi.fn()
  };
}

/** Every element a keyboard user can land on. */
function focusable(container: HTMLElement): HTMLElement[] {
  return [
    ...container.querySelectorAll<HTMLElement>(
      'button, input, select, textarea, a[href], [tabindex]:not([tabindex="-1"])'
    )
  ];
}

/** The name assistive technology would announce for a control. */
function accessibleName(element: HTMLElement): string {
  const label = element.getAttribute('aria-label');
  if (label) return label;
  const labelledBy = element.getAttribute('aria-labelledby');
  if (labelledBy) {
    return labelledBy
      .split(/\s+/)
      .map((id) => window.document.getElementById(id)?.textContent ?? '')
      .join(' ')
      .trim();
  }
  const owned = element.closest('label');
  if (owned) return owned.textContent?.trim() ?? '';
  const id = element.getAttribute('id');
  if (id) {
    const external = window.document.querySelector(`label[for="${id}"]`);
    if (external) return external.textContent?.trim() ?? '';
  }
  return element.textContent?.trim() ?? '';
}

afterEach(cleanup);

describe('Layers panel semantics', () => {
  it('exposes the stack as a tree with a level for every row', () => {
    const { container } = render(LayersPanel, { props: panelProps(panelDocument()) });
    const tree = container.querySelector('[role="tree"]');
    expect(tree?.getAttribute('aria-label')).toBeTruthy();
    const rows = screen.getAllByRole('treeitem');
    expect(rows.length).toBeGreaterThan(0);
    for (const row of rows) {
      expect(Number(row.getAttribute('aria-level'))).toBeGreaterThanOrEqual(1);
      expect(row.getAttribute('aria-selected')).toMatch(/true|false/);
    }
  });

  it('says whether a group is open or closed', () => {
    const { container } = render(LayersPanel, { props: panelProps(panelDocument()) });
    const groups = [...container.querySelectorAll('[role="treeitem"][aria-expanded]')];
    expect(groups.length).toBe(2);
    expect(groups.every((row) => row.getAttribute('aria-expanded') === 'true')).toBe(true);
  });

  it('names every control a keyboard user can reach', () => {
    const { container } = render(LayersPanel, { props: panelProps(panelDocument()) });
    const unnamed = focusable(container).filter((element) => !accessibleName(element));
    expect(unnamed.map((element) => element.outerHTML.slice(0, 80))).toEqual([]);
  });

  it('reports the state of every toggle rather than only its colour', () => {
    const { container } = render(LayersPanel, { props: panelProps(panelDocument()) });
    for (const name of ['Hide Background', 'Lock Background']) {
      const toggle = within(container).getByRole('button', { name });
      expect(toggle.getAttribute('aria-pressed')).toMatch(/true|false/);
    }
  });

  it('gives the opacity and blend controls real labels', () => {
    render(LayersPanel, { props: panelProps(panelDocument()) });
    expect(screen.getByLabelText('Opacity')).toBeTruthy();
    expect(screen.getByLabelText('Blend')).toBeTruthy();
  });
});

describe('transform controls', () => {
  function transformProps(overrides: Record<string, unknown> = {}) {
    return {
      transform: { ...identityTransform },
      layerWidth: 100,
      layerHeight: 60,
      canvasWidth: 200,
      canvasHeight: 200,
      layerName: 'Sky',
      onchange: vi.fn(),
      onaspectchange: vi.fn(),
      ontoggle: vi.fn(),
      onreset: vi.fn(),
      onrasterize: vi.fn(),
      onflip: vi.fn(),
      ...overrides
    };
  }

  it('names every numeric field and action in the panel', () => {
    const { container } = render(TransformPanel, { props: transformProps() });
    const unnamed = focusable(container).filter((element) => !accessibleName(element));
    expect(unnamed.map((element) => element.outerHTML.slice(0, 80))).toEqual([]);
  });

  it('marks the sampling choice and the transform mode as pressed states', () => {
    render(TransformPanel, { props: transformProps({ active: true }) });
    expect(screen.getByRole('button', { name: 'Smooth' }).getAttribute('aria-pressed')).toBe(
      'true'
    );
    expect(screen.getByRole('button', { name: 'Transforming' }).getAttribute('aria-pressed')).toBe(
      'true'
    );
  });

  it('gives the panel and its sampling group a heading and a legend', () => {
    const { container } = render(TransformPanel, { props: transformProps() });
    expect(screen.getByRole('heading', { name: 'Transform' })).toBeTruthy();
    expect(container.querySelector('fieldset > legend')?.textContent).toBe('Sampling');
  });

  it('makes the on-canvas box focusable and explains its gestures', () => {
    const { container } = render(TransformOverlay, {
      props: {
        transform: { ...identityTransform },
        layerWidth: 100,
        layerHeight: 100,
        canvasWidth: 200,
        canvasHeight: 200,
        layerName: 'Sky',
        onpreview: vi.fn(),
        oncommit: vi.fn(),
        oncancel: vi.fn()
      }
    });
    const surface = container.querySelector('.transform-surface') as HTMLButtonElement;
    expect(surface.tagName).toBe('BUTTON');
    const label = surface.getAttribute('aria-label') ?? '';
    // A pointer-only description would leave a keyboard user with no way in.
    expect(label).toMatch(/Arrow keys/);
    expect(label).toMatch(/Escape/);
    // Decorative geometry must not be announced one shape at a time.
    expect(container.querySelector('svg')?.getAttribute('aria-hidden')).toBe('true');
  });
});

describe('adjustment surfaces', () => {
  it('names every control in the adjustment dialog', () => {
    const { container } = render(AdjustmentLayerDialog, {
      props: {
        operation: { type: 'brightness', amount: 0.2 },
        mode: 'create' as const,
        layerName: 'Warmth',
        onchange: vi.fn(),
        onconfirm: vi.fn(),
        oncancel: vi.fn()
      }
    });
    const unnamed = focusable(container).filter((element) => !accessibleName(element));
    expect(unnamed.map((element) => element.outerHTML.slice(0, 80))).toEqual([]);
  });

  it('exposes each curve point as a slider with a readable value', () => {
    const { container } = render(CurveEditor, {
      props: { curves: identityCurves(), onchange: vi.fn(), disabled: false }
    });
    const points = container.querySelectorAll('.curve-point[role="slider"]');
    expect(points.length).toBeGreaterThan(0);
    for (const point of points) {
      expect(point.getAttribute('aria-valuetext')).toBeTruthy();
      expect(point.getAttribute('tabindex')).toBe('0');
    }
  });
});
