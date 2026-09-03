import { createEvent, fireEvent, render, screen, within } from '@testing-library/svelte';
import { describe, expect, it, vi } from 'vitest';
import LayersPanel from './LayersPanel.svelte';
import {
  createAdjustmentLayer,
  createDocument,
  createGroupLayer,
  createPixelLayer
} from '../layers/tree';
import type { Layer, LayerDocument, LayerMask } from '../layers/types';

function pixel(name: string, pixelId = `px${name}`): Layer {
  return createPixelLayer(name, pixelId, 16, 16);
}

function mask(): LayerMask {
  return {
    snapshot: {
      version: 1,
      width: 1,
      height: 1,
      encoding: 'base64_u8',
      data: '/w',
      checksum: 'fnv1a64:0123456789abcdef'
    },
    enabled: true,
    inverted: false
  };
}

/** jsdom has no DataTransfer, so drag tests use a minimal stand-in. */
function dataTransfer() {
  return { setData: vi.fn(), getData: vi.fn(), effectAllowed: '', dropEffect: '' };
}

/**
 * jsdom does not construct DragEvent, so a `clientY` passed through fireEvent is
 * silently dropped. Defining it explicitly makes the drop-position geometry
 * actually run instead of falling through on a NaN comparison.
 */
async function fireDrag(
  type: 'dragStart' | 'dragOver' | 'drop',
  element: Element,
  clientY: number
) {
  const event = createEvent[type](element, { dataTransfer: dataTransfer() });
  Object.defineProperty(event, 'clientY', { value: clientY, configurable: true });
  return fireEvent(element, event);
}

/** Gives a row a predictable 40px box so the drop bands are deterministic. */
function stubBounds(element: Element) {
  element.getBoundingClientRect = () =>
    ({ top: 0, height: 40, left: 0, right: 100, bottom: 40, width: 100, x: 0, y: 0 }) as DOMRect;
}

function props(document: LayerDocument) {
  return {
    document,
    thumbnails: {} as Record<string, string>,
    editTarget: 'layer' as const,
    adjustmentTarget: 'document' as const,
    disabled: false,
    busy: false,
    hasSelection: false,
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

function selectedDocument(): { document: LayerDocument; background: Layer; top: Layer } {
  const background = pixel('Background');
  const top = pixel('Sky');
  const document = { ...createDocument(16, 16, [background, top]), activeLayerId: top.id };
  return { document, background, top };
}

describe('LayersPanel', () => {
  it('renders the stack top layer first with a layer count', () => {
    const { document } = selectedDocument();
    render(LayersPanel, { props: props(document) });
    const items = screen.getAllByRole('treeitem');
    expect(within(items[0]).getByText('Sky')).toBeTruthy();
    expect(within(items[1]).getByText('Background')).toBeTruthy();
    expect(screen.getByText('2 layers')).toBeTruthy();
  });

  it('shows an empty state when there are no layers', () => {
    render(LayersPanel, { props: props(createDocument(16, 16, [])) });
    expect(screen.getByText(/No layers yet/)).toBeTruthy();
  });

  it('selects a layer when its row is clicked', async () => {
    const { document, background } = selectedDocument();
    const value = props(document);
    render(LayersPanel, { props: value });
    await fireEvent.click(screen.getByRole('button', { name: 'Select Background' }));
    expect(value.onselect).toHaveBeenCalledWith(background.id);
  });

  it('marks the selected layer for assistive technology', () => {
    const { document, top } = selectedDocument();
    render(LayersPanel, { props: props(document) });
    const items = screen.getAllByRole('treeitem');
    expect(items[0].getAttribute('aria-selected')).toBe('true');
    expect(items[1].getAttribute('aria-selected')).toBe('false');
    expect(items[0].getAttribute('data-layer-id')).toBe(top.id);
  });

  it('toggles visibility and lock state', async () => {
    const { document, top } = selectedDocument();
    const value = props(document);
    render(LayersPanel, { props: value });
    await fireEvent.click(screen.getByRole('button', { name: 'Hide Sky' }));
    expect(value.ontoggle).toHaveBeenCalledWith(top.id, 'visible');
    await fireEvent.click(screen.getByRole('button', { name: 'Lock Sky' }));
    expect(value.ontoggle).toHaveBeenCalledWith(top.id, 'locked');
  });

  it('dims hidden layers', () => {
    const hidden = { ...pixel('Hidden'), visible: false };
    const document = createDocument(16, 16, [hidden]);
    render(LayersPanel, { props: props(document) });
    expect(screen.getAllByRole('treeitem')[0].className).toContain('dimmed');
  });

  it('expands and collapses a group and indents its children', async () => {
    const child = pixel('Child');
    const group = createGroupLayer('Group', [child]);
    const document = createDocument(16, 16, [group]);
    const value = props(document);
    render(LayersPanel, { props: value });

    const items = screen.getAllByRole('treeitem');
    expect(items).toHaveLength(2);
    expect(items[0].getAttribute('aria-expanded')).toBe('true');
    expect(items[1].getAttribute('aria-level')).toBe('2');

    await fireEvent.click(screen.getByRole('button', { name: 'Collapse Group' }));
    expect(value.ontoggle).toHaveBeenCalledWith(group.id, 'collapsed');
  });

  it('hides the children of a collapsed group', () => {
    const group = { ...createGroupLayer('Group', [pixel('Child')]), collapsed: true };
    render(LayersPanel, { props: props(createDocument(16, 16, [group])) });
    expect(screen.getAllByRole('treeitem')).toHaveLength(1);
    expect(screen.getAllByRole('treeitem')[0].getAttribute('aria-expanded')).toBe('false');
  });

  it('renders a thumbnail when one is available and an icon otherwise', () => {
    const { document, top } = selectedDocument();
    const value = props(document);
    value.thumbnails = { [top.id]: 'data:image/png;base64,AAAA' };
    const { container } = render(LayersPanel, { props: value });
    expect(container.querySelectorAll('.thumbnail img')).toHaveLength(1);
    expect(container.querySelectorAll('.thumbnail em')).toHaveLength(1);
  });

  it('shows a mask thumbnail only for layers that have a mask', () => {
    const masked = { ...pixel('Masked'), mask: mask() };
    const document = createDocument(16, 16, [pixel('Plain'), masked]);
    const { container } = render(LayersPanel, { props: props(document) });
    expect(container.querySelectorAll('.mask-cell')).toHaveLength(1);
  });

  it('changes opacity and blend mode for the selected layer', async () => {
    const { document, top } = selectedDocument();
    const value = props(document);
    render(LayersPanel, { props: value });

    const opacity = screen.getByLabelText('Opacity') as HTMLInputElement;
    await fireEvent.input(opacity, { target: { value: '40' } });
    expect(value.onopacity).toHaveBeenCalledWith(top.id, 0.4);

    await fireEvent.change(screen.getByLabelText('Blend'), { target: { value: 'multiply' } });
    expect(value.onblend).toHaveBeenCalledWith(top.id, 'multiply');
  });

  it('offers every documented blend mode', () => {
    const { document } = selectedDocument();
    render(LayersPanel, { props: props(document) });
    const options = within(screen.getByLabelText('Blend') as HTMLSelectElement).getAllByRole(
      'option'
    );
    expect(options).toHaveLength(16);
    const labels = options.map((option) => option.textContent?.trim());
    for (const expected of ['Normal', 'Multiply', 'Screen', 'Overlay', 'Luminosity']) {
      expect(labels).toContain(expected);
    }
  });

  it('disables the property controls for a locked layer', () => {
    const locked = { ...pixel('Locked'), locked: true };
    const document = { ...createDocument(16, 16, [locked]), activeLayerId: locked.id };
    render(LayersPanel, { props: props(document) });
    expect((screen.getByLabelText('Opacity') as HTMLInputElement).disabled).toBe(true);
    expect((screen.getByLabelText('Blend') as HTMLSelectElement).disabled).toBe(true);
  });

  it('renames a layer when the rename field is confirmed', async () => {
    const { document, top } = selectedDocument();
    const value = props(document);
    render(LayersPanel, { props: value });
    await fireEvent.click(screen.getByRole('button', { name: 'Rename layer' }));
    const field = screen.getByLabelText('Rename Sky') as HTMLInputElement;
    await fireEvent.input(field, { target: { value: 'Clouds' } });
    await fireEvent.keyDown(field, { key: 'Enter' });
    expect(value.onrename).toHaveBeenCalledWith(top.id, 'Clouds');
  });

  it('abandons a rename on Escape', async () => {
    const { document } = selectedDocument();
    const value = props(document);
    render(LayersPanel, { props: value });
    await fireEvent.click(screen.getByRole('button', { name: 'Rename layer' }));
    const field = screen.getByLabelText('Rename Sky') as HTMLInputElement;
    await fireEvent.input(field, { target: { value: 'Discarded' } });
    await fireEvent.keyDown(field, { key: 'Escape' });
    expect(value.onrename).not.toHaveBeenCalled();
  });

  it('creates layers, groups, adjustment layers, and imports', async () => {
    const { document } = selectedDocument();
    const value = props(document);
    render(LayersPanel, { props: value });
    for (const [label, kind] of [
      ['New pixel layer', 'pixel'],
      ['New group', 'group'],
      ['New adjustment layer', 'adjustment'],
      ['Place image as layer', 'import']
    ] as const) {
      await fireEvent.click(screen.getByRole('button', { name: label }));
      expect(value.oncreate).toHaveBeenCalledWith(kind);
    }
  });

  it('raises duplicate and delete actions for the selected layer', async () => {
    const { document, top } = selectedDocument();
    const value = props(document);
    render(LayersPanel, { props: value });
    await fireEvent.click(screen.getByRole('button', { name: 'Duplicate layer' }));
    expect(value.onaction).toHaveBeenCalledWith('duplicate', top.id);
    await fireEvent.click(screen.getByRole('button', { name: 'Delete layer' }));
    expect(value.onaction).toHaveBeenCalledWith('delete', top.id);
  });

  it('reorders with the move buttons using stack indices', async () => {
    const { document, top } = selectedDocument();
    const value = props(document);
    render(LayersPanel, { props: value });
    // The selected layer is on top at stack index 1, so it can only move down.
    expect((screen.getByRole('button', { name: 'Move layer up' }) as HTMLButtonElement).disabled).toBe(
      true
    );
    await fireEvent.click(screen.getByRole('button', { name: 'Move layer down' }));
    expect(value.onreorder).toHaveBeenCalledWith(top.id, null, 0);
  });

  it('drags a row below another and shows the drop hint', async () => {
    const { document, background, top } = selectedDocument();
    const value = props(document);
    render(LayersPanel, { props: value });

    const source = screen.getByTestId(`layer-row-${top.id}`);
    const target = screen.getByTestId(`layer-row-${background.id}`);
    stubBounds(target);

    await fireDrag('dragStart', source, 0);
    // The lower half of the bottom row means "below it" in display order,
    // which is stack index 0.
    await fireDrag('dragOver', target, 34);
    expect(target.className).toContain('drop-below');
    await fireDrag('drop', target, 34);
    expect(value.onreorder).toHaveBeenCalledWith(top.id, null, 0);
  });

  it('drags a row above another', async () => {
    const { document, background, top } = selectedDocument();
    const value = props(document);
    render(LayersPanel, { props: value });

    const source = screen.getByTestId(`layer-row-${top.id}`);
    const target = screen.getByTestId(`layer-row-${background.id}`);
    stubBounds(target);

    await fireDrag('dragStart', source, 0);
    await fireDrag('dragOver', target, 4);
    expect(target.className).toContain('drop-above');
    await fireDrag('drop', target, 4);
    expect(value.onreorder).toHaveBeenCalledWith(top.id, null, 1);
  });

  it('drops a layer into a group when released over its middle', async () => {
    const loose = pixel('Loose');
    const group = createGroupLayer('Group', []);
    const document = { ...createDocument(16, 16, [loose, group]), activeLayerId: loose.id };
    const value = props(document);
    render(LayersPanel, { props: value });

    const source = screen.getByTestId(`layer-row-${loose.id}`);
    const target = screen.getByTestId(`layer-row-${group.id}`);
    stubBounds(target);

    await fireDrag('dragStart', source, 0);
    await fireDrag('dragOver', target, 20);
    expect(target.className).toContain('drop-inside');
    await fireDrag('drop', target, 20);
    expect(value.onreorder).toHaveBeenCalledWith(loose.id, group.id, 0);
  });

  it('does not treat the middle of a non-group row as a drop target', async () => {
    const { document, background, top } = selectedDocument();
    const value = props(document);
    render(LayersPanel, { props: value });
    const target = screen.getByTestId(`layer-row-${background.id}`);
    stubBounds(target);

    await fireDrag('dragStart', screen.getByTestId(`layer-row-${top.id}`), 0);
    await fireDrag('drop', target, 20);
    expect(value.onreorder).toHaveBeenCalledWith(top.id, null, 0);
  });

  it('ignores a drop of a row onto itself', async () => {
    const { document, top } = selectedDocument();
    const value = props(document);
    render(LayersPanel, { props: value });
    const row = screen.getByTestId(`layer-row-${top.id}`);
    stubBounds(row);

    await fireDrag('dragStart', row, 0);
    await fireDrag('drop', row, 34);
    expect(value.onreorder).not.toHaveBeenCalled();
  });

  it('does not start a drag from a locked layer', async () => {
    const locked = { ...pixel('Locked'), locked: true };
    const other = pixel('Other');
    const document = { ...createDocument(16, 16, [locked, other]), activeLayerId: other.id };
    const value = props(document);
    render(LayersPanel, { props: value });

    const source = screen.getByTestId(`layer-row-${locked.id}`);
    expect(source.getAttribute('draggable')).toBe('false');
    await fireDrag('dragStart', source, 0);
    const target = screen.getByTestId(`layer-row-${other.id}`);
    stubBounds(target);
    await fireDrag('drop', target, 5);
    expect(value.onreorder).not.toHaveBeenCalled();
  });

  it('switches the active editing target and explains what it changes', async () => {
    const { document } = selectedDocument();
    const value = props(document);
    const { rerender } = render(LayersPanel, { props: value });
    expect(screen.getByText(/Painting changes the pixels of/)).toBeTruthy();

    await fireEvent.click(screen.getByRole('button', { name: 'Selection' }));
    expect(value.ontargetchange).toHaveBeenCalledWith('selection');

    await rerender({ ...value, editTarget: 'selection' });
    expect(screen.getByText(/Painting changes the document selection/)).toBeTruthy();
  });

  it('only offers the mask target when the selected layer has a mask', async () => {
    const { document, top } = selectedDocument();
    const value = props(document);
    const { rerender } = render(LayersPanel, { props: value });
    expect((screen.getByRole('button', { name: 'Mask' }) as HTMLButtonElement).disabled).toBe(true);

    const masked = { ...top, mask: mask() };
    const withMask = { ...document, layers: [document.layers[0], masked] };
    await rerender({ ...value, document: withMask });
    expect((screen.getByRole('button', { name: 'Mask' }) as HTMLButtonElement).disabled).toBe(false);
  });

  it('warns clearly while the mask is the editing target', async () => {
    const { document, top } = selectedDocument();
    const masked = { ...top, mask: mask() };
    const withMask = { ...document, layers: [document.layers[0], masked] };
    const value = { ...props(withMask), editTarget: 'mask' as const };
    render(LayersPanel, { props: value });
    expect(screen.getByText(/Painting changes the mask on/)).toBeTruthy();
  });

  it('chooses where global adjustments are applied', async () => {
    const { document } = selectedDocument();
    const value = props(document);
    render(LayersPanel, { props: value });
    const select = screen.getByLabelText('Adjustments go to') as HTMLSelectElement;
    expect(select.value).toBe('document');
    await fireEvent.change(select, { target: { value: 'adjustmentLayer' } });
    expect(value.onadjustmenttargetchange).toHaveBeenCalledWith('adjustmentLayer');
  });

  it('offers mask creation for a layer without one', async () => {
    const { document, top } = selectedDocument();
    const value = props(document);
    render(LayersPanel, { props: value });
    await fireEvent.click(screen.getByRole('button', { name: 'Reveal all' }));
    expect(value.onaction).toHaveBeenCalledWith('mask_white', top.id);
    await fireEvent.click(screen.getByRole('button', { name: 'Hide all' }));
    expect(value.onaction).toHaveBeenCalledWith('mask_black', top.id);
    expect(
      (screen.getByRole('button', { name: 'From selection' }) as HTMLButtonElement).disabled
    ).toBe(true);
  });

  it('enables creating a mask from a selection once one exists', async () => {
    const { document, top } = selectedDocument();
    const value = { ...props(document), hasSelection: true };
    render(LayersPanel, { props: value });
    await fireEvent.click(screen.getByRole('button', { name: 'From selection' }));
    expect(value.onaction).toHaveBeenCalledWith('mask_from_selection', top.id);
  });

  it('offers mask management once a mask exists', async () => {
    const { document, top } = selectedDocument();
    const masked = { ...top, mask: mask() };
    const withMask = { ...document, layers: [document.layers[0], masked] };
    const value = props(withMask);
    render(LayersPanel, { props: value });

    await fireEvent.click(screen.getByRole('button', { name: 'Disable' }));
    expect(value.onaction).toHaveBeenCalledWith('mask_toggle', top.id);
    await fireEvent.click(screen.getByRole('button', { name: 'Invert' }));
    expect(value.onaction).toHaveBeenCalledWith('mask_invert', top.id);
    await fireEvent.click(screen.getByRole('button', { name: 'To selection' }));
    expect(value.onaction).toHaveBeenCalledWith('mask_load_selection', top.id);
    await fireEvent.click(screen.getByRole('button', { name: 'Delete' }));
    expect(value.onaction).toHaveBeenCalledWith('mask_delete', top.id);
  });

  it('raises group, merge, and flatten actions', async () => {
    const { document, top } = selectedDocument();
    const value = props(document);
    render(LayersPanel, { props: value });
    await fireEvent.click(screen.getByRole('button', { name: 'Group' }));
    expect(value.onaction).toHaveBeenCalledWith('group', top.id);
    await fireEvent.click(screen.getByRole('button', { name: 'Merge down' }));
    expect(value.onaction).toHaveBeenCalledWith('merge_down', top.id);
    await fireEvent.click(screen.getByRole('button', { name: 'Flatten' }));
    expect(value.onaction).toHaveBeenCalledWith('flatten');
    expect((screen.getByRole('button', { name: 'Ungroup' }) as HTMLButtonElement).disabled).toBe(
      true
    );
  });

  it('enables ungroup only for a selected group', () => {
    const group = createGroupLayer('Group', [pixel('Child')]);
    const document = { ...createDocument(16, 16, [group]), activeLayerId: group.id };
    render(LayersPanel, { props: props(document) });
    expect((screen.getByRole('button', { name: 'Ungroup' }) as HTMLButtonElement).disabled).toBe(
      false
    );
    expect((screen.getByRole('button', { name: 'Group' }) as HTMLButtonElement).disabled).toBe(true);
  });

  it('opens an adjustment layer for editing on double click', async () => {
    const adjustment = createAdjustmentLayer('Curves', { type: 'grayscale' });
    const document = { ...createDocument(16, 16, [adjustment]), activeLayerId: adjustment.id };
    const value = props(document);
    render(LayersPanel, { props: value });

    await fireEvent.dblClick(screen.getByRole('button', { name: 'Select Curves' }));
    expect(value.onaction).toHaveBeenCalledWith('edit_adjustment', adjustment.id);
    await fireEvent.click(screen.getByRole('button', { name: 'Edit adjustment' }));
    expect(value.onaction).toHaveBeenCalledWith('edit_adjustment', adjustment.id);
  });

  it('labels adjustment layers by type and hides pixel-only actions', () => {
    const adjustment = createAdjustmentLayer('Curves', { type: 'grayscale' });
    const document = { ...createDocument(16, 16, [adjustment]), activeLayerId: adjustment.id };
    render(LayersPanel, { props: props(document) });
    expect(screen.getByText(/Adjustment layer/)).toBeTruthy();
    expect(
      (screen.getByRole('button', { name: 'Rasterize' }) as HTMLButtonElement).disabled
    ).toBe(true);
    expect((screen.getByRole('button', { name: 'Layer' }) as HTMLButtonElement).disabled).toBe(true);
  });

  it('raises transform actions for pixel layers', async () => {
    const { document, top } = selectedDocument();
    const value = props(document);
    render(LayersPanel, { props: value });
    await fireEvent.click(screen.getByRole('button', { name: 'Reset transform' }));
    expect(value.onaction).toHaveBeenCalledWith('reset_transform', top.id);
    await fireEvent.click(screen.getByRole('button', { name: 'Rasterize' }));
    expect(value.onaction).toHaveBeenCalledWith('rasterize_transform', top.id);
  });

  it('shows non-default blend mode and opacity on the row itself', () => {
    const styled = { ...pixel('Styled'), blendMode: 'multiply' as const, opacity: 0.5 };
    const document = createDocument(16, 16, [styled]);
    render(LayersPanel, { props: props(document) });
    const row = screen.getAllByRole('treeitem')[0];
    expect(within(row).getByText(/Multiply/)).toBeTruthy();
    expect(within(row).getByText(/50%/)).toBeTruthy();
  });

  it('disables every control while the panel is disabled', () => {
    const { document } = selectedDocument();
    const value = { ...props(document), disabled: true };
    const { container } = render(LayersPanel, { props: value });
    const enabled = [...container.querySelectorAll('button')].filter(
      (button) => !(button as HTMLButtonElement).disabled
    );
    expect(enabled).toHaveLength(0);
  });
});
