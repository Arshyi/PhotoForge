import { cleanup, fireEvent, render, screen, within } from '@testing-library/svelte';
import { afterEach, describe, expect, it, vi } from 'vitest';
import LayersPanel from './LayersPanel.svelte';
import {
  createDocument,
  createGroupLayer,
  createPixelLayer,
  countLayers,
  displayRows
} from '../layers/tree';
import { MAX_GROUP_DEPTH, MAX_LAYERS, type Layer, type LayerDocument } from '../layers/types';

/**
 * Large-document behaviour for the Layers panel.
 *
 * These are correctness tests at scale, not benchmarks: a wall-clock threshold
 * measured in jsdom says nothing about a real browser and would only flake on a
 * busy machine. What they do assert is that nothing degrades or silently
 * truncates as the tree grows — every layer is counted, every visible row is
 * rendered and reachable, and selection, collapse, reorder, rename, delete, and
 * visibility all still act on the layer they name.
 *
 * The document limits themselves are deliberately not raised to make anything
 * here pass.
 */

function pixel(index: number): Layer {
  return createPixelLayer(`Layer ${index}`, `px${index}`, 16, 16);
}

function flatDocument(count: number): LayerDocument {
  const layers = Array.from({ length: count }, (_, index) => pixel(index));
  return { ...createDocument(16, 16, layers), activeLayerId: layers[0].id };
}

/** A chain of nested groups reaching the model's documented depth limit. */
function deepDocument(depth: number): LayerDocument {
  let node: Layer = pixel(0);
  for (let level = 1; level < depth; level += 1) {
    node = createGroupLayer(`Group ${level}`, [node]);
  }
  return createDocument(16, 16, [node]);
}

function props(document: LayerDocument, overrides: Record<string, unknown> = {}) {
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
    onadjustmenttargetchange: vi.fn(),
    ...overrides
  };
}

afterEach(cleanup);

describe('a hundred layers', () => {
  it('renders every row and counts every layer', () => {
    const document = flatDocument(100);
    render(LayersPanel, { props: props(document) });
    expect(screen.getAllByRole('treeitem')).toHaveLength(100);
    expect(screen.getByText('100 layers')).toBeTruthy();
  });

  it('keeps the last layer reachable, not just the first screenful', async () => {
    const document = flatDocument(100);
    const value = props(document);
    render(LayersPanel, { props: value });
    // Index 0 is the bottom of the stack, so it is the last row on screen.
    await fireEvent.click(screen.getByRole('button', { name: 'Select Layer 0' }));
    expect(value.onselect).toHaveBeenCalledWith(document.layers[0].id, false);
  });

  it('keeps every row inside the one scrollable list', () => {
    const { container } = render(LayersPanel, { props: props(flatDocument(100)) });
    const lists = container.querySelectorAll('.layer-list');
    expect(lists).toHaveLength(1);
    // A long stack must stay inside the list, which is height-capped and scrolls
    // on its own, rather than spilling out and pushing the transform and mask
    // controls off the panel.
    expect(lists[0].querySelectorAll(':scope > li')).toHaveLength(100);
    expect(lists[0].getAttribute('role')).toBe('tree');
  });
});

describe('two hundred and fifty layers', () => {
  const COUNT = 250;

  it('stays inside the documented layer ceiling', () => {
    expect(COUNT).toBeLessThanOrEqual(MAX_LAYERS);
  });

  it('renders and counts them all', () => {
    render(LayersPanel, { props: props(flatDocument(COUNT)) });
    expect(screen.getAllByRole('treeitem')).toHaveLength(COUNT);
    expect(screen.getByText(`${COUNT} layers`)).toBeTruthy();
  });

  it('acts on the named layer for every row action', async () => {
    const document = flatDocument(COUNT);
    const value = props(document);
    render(LayersPanel, { props: value });
    const middle = document.layers[COUNT - 120];

    await fireEvent.click(screen.getByRole('button', { name: `Select ${middle.name}` }));
    expect(value.onselect).toHaveBeenCalledWith(middle.id, false);

    await fireEvent.click(screen.getByRole('button', { name: `Hide ${middle.name}` }));
    expect(value.ontoggle).toHaveBeenCalledWith(middle.id, 'visible');

    await fireEvent.click(screen.getByRole('button', { name: `Lock ${middle.name}` }));
    expect(value.ontoggle).toHaveBeenCalledWith(middle.id, 'locked');
  });

  it('keeps the active layer marked among all the others', () => {
    const document = flatDocument(COUNT);
    const active = document.layers[200];
    render(LayersPanel, { props: props({ ...document, activeLayerId: active.id }) });
    const selected = screen
      .getAllByRole('treeitem')
      .filter((row) => row.getAttribute('aria-selected') === 'true');
    expect(selected).toHaveLength(1);
    expect(within(selected[0]).getByText(active.name)).toBeTruthy();
  });

  it('renames and deletes a layer buried deep in the stack', async () => {
    const document = flatDocument(COUNT);
    const target = document.layers[7];
    const value = props({ ...document, activeLayerId: target.id });
    render(LayersPanel, { props: value });
    await fireEvent.click(screen.getByRole('button', { name: 'Rename layer' }));
    const field = screen.getByLabelText(`Rename ${target.name}`) as HTMLInputElement;
    await fireEvent.input(field, { target: { value: 'Renamed' } });
    await fireEvent.keyDown(field, { key: 'Enter' });
    expect(value.onrename).toHaveBeenCalledWith(target.id, 'Renamed');

    await fireEvent.click(screen.getByRole('button', { name: 'Delete layer' }));
    expect(value.onaction).toHaveBeenCalledWith('delete', target.id);
  });

  it('moves a layer without renumbering the rest of the stack', async () => {
    const document = flatDocument(COUNT);
    const target = document.layers[100];
    const value = props({ ...document, activeLayerId: target.id });
    render(LayersPanel, { props: value });
    await fireEvent.click(screen.getByRole('button', { name: 'Move layer up' }));
    expect(value.onreorder).toHaveBeenCalledWith(target.id, null, 101);
  });
});

describe('deep nesting', () => {
  it('renders a chain at the documented depth limit with correct levels', () => {
    const document = deepDocument(MAX_GROUP_DEPTH);
    expect(countLayers(document)).toBe(MAX_GROUP_DEPTH);
    render(LayersPanel, { props: props(document) });
    const rows = screen.getAllByRole('treeitem');
    expect(rows).toHaveLength(MAX_GROUP_DEPTH);
    // Each level is one deeper than the last, so assistive technology can
    // follow the hierarchy all the way down.
    expect(rows.map((row) => Number(row.getAttribute('aria-level')))).toEqual(
      Array.from({ length: MAX_GROUP_DEPTH }, (_, index) => index + 1)
    );
  });

  it('collapsing the outermost group hides every level beneath it', async () => {
    const document = deepDocument(MAX_GROUP_DEPTH);
    const collapsed = {
      ...document,
      layers: [{ ...document.layers[0], collapsed: true }]
    };
    render(LayersPanel, { props: props(collapsed) });
    expect(screen.getAllByRole('treeitem')).toHaveLength(1);
    // The count still reports the whole document, not what is on screen.
    expect(screen.getByText(`${MAX_GROUP_DEPTH} layers`)).toBeTruthy();
    expect(displayRows(collapsed)).toHaveLength(1);
  });

  it('collapsing a group in a large document leaves the rest addressable', async () => {
    const groups = Array.from({ length: 20 }, (_, index) =>
      createGroupLayer(`Group ${index}`, [pixel(index * 2), pixel(index * 2 + 1)])
    );
    const document = createDocument(16, 16, groups);
    expect(countLayers(document)).toBe(60);
    const half = {
      ...document,
      layers: document.layers.map((layer, index) =>
        index % 2 === 0 ? { ...layer, collapsed: true } : layer
      )
    };
    const value = props(half);
    render(LayersPanel, { props: value });
    // Twenty group rows, plus the children of the ten expanded groups.
    expect(screen.getAllByRole('treeitem')).toHaveLength(40);
    expect(screen.getByText('60 layers')).toBeTruthy();
    await fireEvent.click(screen.getByRole('button', { name: 'Select Layer 3' }));
    expect(value.onselect).toHaveBeenCalled();
  });
});

describe('many masks and thumbnails', () => {
  it('renders a thumbnail for every layer that has one', () => {
    const document = flatDocument(100);
    const thumbnails = Object.fromEntries(
      document.layers.map((layer) => [
        layer.id,
        'data:image/png;base64,iVBORw0KGgoAAAANSUhEUg=='
      ])
    );
    const { container } = render(LayersPanel, { props: props(document, { thumbnails }) });
    expect(container.querySelectorAll('.thumbnail img')).toHaveLength(100);
  });

  it('renders rows for layers whose thumbnail has not arrived yet', () => {
    // Thumbnails load lazily, so a row must never wait on one to appear.
    render(LayersPanel, { props: props(flatDocument(100), { thumbnails: {} }) });
    expect(screen.getAllByRole('treeitem')).toHaveLength(100);
  });
});
