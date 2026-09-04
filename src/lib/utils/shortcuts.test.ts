import { describe, expect, it } from 'vitest';
import {
  bindingAction,
  isTextEntryTarget,
  ownsArrowKeys,
  resolveShortcut,
  suppressedWhileTyping,
  type ShortcutContext,
  type ShortcutEvent
} from './shortcuts';
import { defaultShortcuts } from './workspace';

function context(overrides: Partial<ShortcutContext> = {}): ShortcutContext {
  return {
    hasImage: true,
    hasLayers: true,
    hasActiveLayer: true,
    transformActive: false,
    selectionBusy: false,
    selectionTool: 'none',
    settingsOpen: false,
    modalOpen: false,
    bindings: defaultShortcuts,
    ...overrides
  };
}

function press(key: string, overrides: Partial<ShortcutEvent> = {}): ShortcutEvent {
  return { key, ctrlKey: false, shiftKey: false, altKey: false, target: null, ...overrides };
}

/** A stand-in for the element a keystroke landed on. */
function element(tag: string, attributes: Record<string, string> = {}): EventTarget {
  const node = document.createElement(tag);
  for (const [name, value] of Object.entries(attributes)) node.setAttribute(name, value);
  return node;
}

describe('typing targets', () => {
  it('recognises the places a user types', () => {
    expect(isTextEntryTarget(element('input'))).toBe(true);
    expect(isTextEntryTarget(element('input', { type: 'text' }))).toBe(true);
    expect(isTextEntryTarget(element('input', { type: 'number' }))).toBe(true);
    expect(isTextEntryTarget(element('textarea'))).toBe(true);
    expect(isTextEntryTarget(element('select'))).toBe(true);
    expect(isTextEntryTarget(element('div', { contenteditable: 'true' }))).toBe(true);
  });

  it('does not treat a button or a slider as somewhere to type', () => {
    expect(isTextEntryTarget(element('button'))).toBe(false);
    expect(isTextEntryTarget(element('input', { type: 'range' }))).toBe(false);
    expect(isTextEntryTarget(element('input', { type: 'checkbox' }))).toBe(false);
    expect(isTextEntryTarget(element('div'))).toBe(false);
    expect(isTextEntryTarget(null)).toBe(false);
    expect(isTextEntryTarget(undefined)).toBe(false);
  });

  it('knows which controls handle the arrow keys themselves', () => {
    expect(ownsArrowKeys(element('input', { type: 'range' }))).toBe(true);
    expect(ownsArrowKeys(element('select'))).toBe(true);
    expect(ownsArrowKeys(element('div', { role: 'slider' }))).toBe(true);
    expect(ownsArrowKeys(element('button'))).toBe(false);
  });

  it('gives every plain key to the field, and only the field commands', () => {
    expect(suppressedWhileTyping(press('a'))).toBe(true);
    expect(suppressedWhileTyping(press('Delete'))).toBe(true);
    expect(suppressedWhileTyping(press('a', { ctrlKey: true }))).toBe(true);
    expect(suppressedWhileTyping(press('z', { ctrlKey: true }))).toBe(true);
    // Opening a file has no meaning inside a text field, so it stays global.
    expect(suppressedWhileTyping(press('o', { ctrlKey: true }))).toBe(false);
  });
});

/**
 * The behaviour the Layers panel's rename field depends on. Before the
 * dispatcher was extracted these keys reached the window handler unguarded, so
 * typing a layer name toggled Compare, the pixel inspector, and the grid.
 */
describe('suppression while renaming a layer', () => {
  const renaming = element('input', { type: 'text' });

  it('sends Delete to the text, not to the layer', () => {
    expect(resolveShortcut(press('Delete', { target: renaming }), context())).toBeNull();
    // The same key outside a field still deletes the layer.
    expect(resolveShortcut(press('Delete'), context())).toEqual({
      type: 'layer',
      action: 'delete'
    });
  });

  it('sends Ctrl+A to the text, not to Select All on the image', () => {
    expect(
      resolveShortcut(press('a', { ctrlKey: true, target: renaming }), context())
    ).toBeNull();
    expect(resolveShortcut(press('a', { ctrlKey: true }), context())).toEqual({
      type: 'mask',
      action: 'select_all'
    });
  });

  it('sends Ctrl+Z to the text field rather than undoing the document', () => {
    expect(
      resolveShortcut(press('z', { ctrlKey: true, target: renaming }), context())
    ).toBeNull();
    expect(resolveShortcut(press('z', { ctrlKey: true }), context())).toEqual({
      type: 'binding',
      action: 'Undo'
    });
  });

  it.each([
    ['c', 'Compare'],
    ['i', 'Pixel inspector'],
    ['r', 'Crop'],
    ['t', 'Straighten']
  ])('types "%s" into the field instead of running %s', (key) => {
    expect(resolveShortcut(press(key, { target: renaming }), context())).toBeNull();
  });

  it('withholds the single-letter selection tools too', () => {
    for (const key of ['m', 'l', 'w', 'b', 'e', 'q']) {
      expect(resolveShortcut(press(key, { target: renaming }), context())).toBeNull();
    }
  });

  /**
   * File-level commands mean the same thing wherever the caret is, so they stay
   * live. Structural layer edits do not: creating or grouping a layer while its
   * name is half typed would leave the rename field pointing at nothing.
   */
  it('keeps file commands live while typing but withholds layer edits', () => {
    expect(resolveShortcut(press('s', { ctrlKey: true, target: renaming }), context())).toEqual({
      type: 'binding',
      action: 'Export image'
    });
    expect(resolveShortcut(press('o', { ctrlKey: true, target: renaming }), context())).toEqual({
      type: 'binding',
      action: 'Open image'
    });
    for (const event of [
      press('n', { ctrlKey: true, shiftKey: true, target: renaming }),
      press('j', { ctrlKey: true, target: renaming }),
      press('g', { ctrlKey: true, target: renaming }),
      press('t', { ctrlKey: true, target: renaming })
    ]) {
      expect(resolveShortcut(event, context())).toBeNull();
    }
  });
});

describe('adjustment and numeric fields', () => {
  it('suppresses unrelated shortcuts in an adjustment parameter field', () => {
    const numberField = element('input', { type: 'number' });
    expect(resolveShortcut(press('c', { target: numberField }), context())).toBeNull();
    expect(resolveShortcut(press('Delete', { target: numberField }), context())).toBeNull();
    expect(
      resolveShortcut(press('a', { ctrlKey: true, target: numberField }), context())
    ).toBeNull();
  });

  it('leaves a focused slider its own arrow keys during a transform', () => {
    const slider = element('input', { type: 'range' });
    expect(
      resolveShortcut(press('ArrowLeft', { target: slider }), context({ transformActive: true }))
    ).toBeNull();
    expect(resolveShortcut(press('ArrowLeft'), context({ transformActive: true }))).toEqual({
      type: 'transform_nudge',
      direction: 'left',
      large: false
    });
  });

  it('still lets a focused slider use the letter shortcuts', () => {
    const slider = element('input', { type: 'range' });
    expect(resolveShortcut(press('c', { target: slider }), context())).toEqual({
      type: 'tool',
      tool: 'color_range'
    });
  });
});

describe('layer shortcuts', () => {
  it.each([
    ['n', { ctrlKey: true, shiftKey: true }, 'new'],
    ['j', { ctrlKey: true }, 'duplicate'],
    ['g', { ctrlKey: true }, 'group'],
    ['g', { ctrlKey: true, shiftKey: true }, 'ungroup']
  ])('maps %s to the %s layer command', (key, modifiers, action) => {
    expect(resolveShortcut(press(key, modifiers), context())).toEqual({ type: 'layer', action });
  });

  it('does nothing without a layer tree', () => {
    expect(
      resolveShortcut(press('j', { ctrlKey: true }), context({ hasLayers: false }))
    ).toBeNull();
  });

  it('does not delete when no layer is active', () => {
    expect(resolveShortcut(press('Delete'), context({ hasActiveLayer: false }))).toBeNull();
  });

  it('does nothing at all with no image open', () => {
    expect(resolveShortcut(press('Delete'), context({ hasImage: false }))).toBeNull();
    expect(resolveShortcut(press('q'), context({ hasImage: false }))).toBeNull();
    // The configurable bindings still work, so a photo can be opened.
    expect(resolveShortcut(press('o', { ctrlKey: true }), context({ hasImage: false }))).toEqual({
      type: 'binding',
      action: 'Open image'
    });
  });
});

describe('transform shortcuts', () => {
  it('toggles the transform box with Ctrl+T', () => {
    expect(resolveShortcut(press('t', { ctrlKey: true }), context())).toEqual({
      type: 'transform',
      action: 'toggle'
    });
  });

  it.each([
    ['h', 'flip_horizontal'],
    ['u', 'flip_vertical'],
    ['r', 'reset']
  ])('maps Ctrl+Shift+%s to %s', (key, action) => {
    expect(resolveShortcut(press(key, { ctrlKey: true, shiftKey: true }), context())).toEqual({
      type: 'transform',
      action
    });
  });

  /**
   * The workspace already binds the bare letters C, I, R, and T, and the
   * selection tools own M, L, W, B, E, and Q. A transform key that claimed one
   * would silently break an existing shortcut.
   */
  it('never claims a key an existing shortcut already uses', () => {
    for (const [key, expected] of [
      ['c', { type: 'tool', tool: 'color_range' }],
      ['i', { type: 'binding', action: 'Pixel inspector' }],
      ['r', { type: 'binding', action: 'Crop' }],
      ['t', { type: 'binding', action: 'Straighten' }]
    ] as const) {
      expect(resolveShortcut(press(key), context())).toEqual(expected);
    }
  });

  it('nudges by one pixel, and by ten with Shift', () => {
    const active = context({ transformActive: true });
    expect(resolveShortcut(press('ArrowRight'), active)).toEqual({
      type: 'transform_nudge',
      direction: 'right',
      large: false
    });
    expect(resolveShortcut(press('ArrowUp', { shiftKey: true }), active)).toEqual({
      type: 'transform_nudge',
      direction: 'up',
      large: true
    });
  });

  it('does not nudge when the transform box is closed', () => {
    expect(resolveShortcut(press('ArrowRight'), context())).toBeNull();
  });

  it('cancels with Escape and finishes with Enter', () => {
    const active = context({ transformActive: true });
    expect(resolveShortcut(press('Escape'), active)).toEqual({ type: 'transform_cancel' });
    expect(resolveShortcut(press('Enter'), active)).toEqual({ type: 'transform_commit' });
  });

  it('leaves Escape and Enter to a field being typed in', () => {
    const active = context({ transformActive: true });
    const field = element('input', { type: 'text' });
    expect(resolveShortcut(press('Escape', { target: field }), active)).toBeNull();
    expect(resolveShortcut(press('Enter', { target: field }), active)).toBeNull();
  });
});

describe('selection and settings', () => {
  it('closes the settings dialog with Escape before anything else', () => {
    expect(resolveShortcut(press('Escape'), context({ settingsOpen: true }))).toEqual({
      type: 'close_settings'
    });
  });

  it('toggles between paired selection tools', () => {
    expect(resolveShortcut(press('m'), context({ selectionTool: 'rectangle' }))).toEqual({
      type: 'tool',
      tool: 'ellipse'
    });
    expect(resolveShortcut(press('m'), context({ selectionTool: 'ellipse' }))).toEqual({
      type: 'tool',
      tool: 'rectangle'
    });
    expect(resolveShortcut(press('l'), context({ selectionTool: 'freehand' }))).toEqual({
      type: 'tool',
      tool: 'polygon'
    });
  });

  it('cancels a running mask operation with Escape', () => {
    expect(resolveShortcut(press('Escape'), context({ selectionBusy: true }))).toEqual({
      type: 'cancel_mask_operation'
    });
  });

  it('hands the whole keyboard to a modal that owns it', () => {
    expect(resolveShortcut(press('Delete'), context({ modalOpen: true }))).toBeNull();
    expect(resolveShortcut(press('o', { ctrlKey: true }), context({ modalOpen: true }))).toBeNull();
  });
});

describe('configurable bindings', () => {
  it('matches the workspace bindings the settings page edits', () => {
    expect(bindingAction(press('o', { ctrlKey: true }), defaultShortcuts)).toBe('Open image');
    expect(bindingAction(press('y', { ctrlKey: true }), defaultShortcuts)).toBe('Redo');
    expect(bindingAction(press('+'), defaultShortcuts)).toBe('Zoom in');
    expect(bindingAction(press('F9'), defaultShortcuts)).toBeUndefined();
  });

  it('ignores a bare modifier keypress', () => {
    expect(bindingAction(press('Control', { ctrlKey: true }), defaultShortcuts)).toBeUndefined();
  });

  it('resolves a re-bound key', () => {
    const bindings = [{ action: 'Compare', keys: 'F2' }];
    expect(resolveShortcut(press('F2'), context({ bindings }))).toEqual({
      type: 'binding',
      action: 'Compare'
    });
  });
});
