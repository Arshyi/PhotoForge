import type { ShortcutBinding } from '../types/editor';
import type { SelectionTool } from '../selections/types';
import { normalizeShortcut } from './workspace';

/**
 * The window-level keyboard dispatcher, as a pure function.
 *
 * The application used to decide this inline inside its `keydown` listener,
 * where it could only be exercised by driving a real window. Resolving an event
 * to an intent here means the suppression rules — above all, that a key typed
 * into a text field belongs to that field and not to the image — can be tested
 * directly and cannot drift.
 */

export type NudgeDirection = 'left' | 'right' | 'up' | 'down';

export type ShortcutIntent =
  | { type: 'layer'; action: 'new' | 'duplicate' | 'group' | 'ungroup' | 'delete' }
  | { type: 'transform'; action: 'toggle' | 'flip_horizontal' | 'flip_vertical' | 'reset' }
  | { type: 'transform_nudge'; direction: NudgeDirection; large: boolean }
  | { type: 'transform_cancel' }
  | { type: 'transform_commit' }
  | { type: 'mask'; action: 'select_all' | 'deselect' | 'invert' }
  | { type: 'overlay_visibility' }
  | { type: 'tool'; tool: SelectionTool }
  | { type: 'cancel_mask_operation' }
  | { type: 'close_settings' }
  | { type: 'binding'; action: string };

export interface ShortcutContext {
  /** An image is open, so the image-editing shortcuts are meaningful. */
  hasImage: boolean;
  /** The document carries a layer tree. */
  hasLayers: boolean;
  hasActiveLayer: boolean;
  /** The transform box is on the canvas, so it owns the arrow keys. */
  transformActive: boolean;
  selectionBusy: boolean;
  selectionTool: SelectionTool;
  settingsOpen: boolean;
  /** A modal that takes over the keyboard entirely. */
  modalOpen: boolean;
  bindings: ShortcutBinding[];
}

/** Only what the dispatcher reads, so tests need not build a DOM event. */
export interface ShortcutEvent {
  key: string;
  ctrlKey?: boolean;
  metaKey?: boolean;
  shiftKey?: boolean;
  altKey?: boolean;
  target?: EventTarget | null;
}

/** Input types that are not places to type. */
const NON_TEXT_INPUT_TYPES = new Set([
  'button',
  'checkbox',
  'color',
  'file',
  'image',
  'radio',
  'range',
  'reset',
  'submit'
]);

/**
 * Whether the event landed somewhere the user is typing.
 *
 * A `select` counts: typing a letter into one jumps to the matching option, so
 * a global single-letter shortcut would fight it.
 */
export function isTextEntryTarget(target: EventTarget | null | undefined): boolean {
  const element = target as HTMLElement | null | undefined;
  if (!element || typeof element.tagName !== 'string') return false;
  const tag = element.tagName.toLowerCase();
  if (tag === 'textarea' || tag === 'select') return true;
  if (element.isContentEditable) return true;
  if (element.getAttribute?.('contenteditable') === 'true') return true;
  if (tag !== 'input') return false;
  const type = (element.getAttribute?.('type') ?? 'text').toLowerCase();
  return !NON_TEXT_INPUT_TYPES.has(type);
}

/** Whether the focused control handles the arrow keys itself. */
export function ownsArrowKeys(target: EventTarget | null | undefined): boolean {
  const element = target as HTMLElement | null | undefined;
  if (!element || typeof element.tagName !== 'string') return false;
  const tag = element.tagName.toLowerCase();
  if (tag === 'input' || tag === 'textarea' || tag === 'select') return true;
  if (element.isContentEditable) return true;
  const role = element.getAttribute?.('role');
  return role === 'slider' || role === 'spinbutton' || role === 'listbox';
}

/**
 * Command keys a focused text field owns.
 *
 * Everything else with Ctrl held — opening, exporting, creating a layer — has
 * no meaning inside a text field, so it stays available while typing. These do:
 * Ctrl+A selects the text, Ctrl+Z undoes the typing, and so on.
 */
const TEXT_FIELD_COMMANDS = new Set(['a', 'c', 'v', 'x', 'z', 'y']);

/**
 * Whether a shortcut must be withheld because the user is typing.
 *
 * Any plain keystroke belongs to the field. A command keystroke belongs to the
 * field only when the field itself defines it.
 */
export function suppressedWhileTyping(event: ShortcutEvent): boolean {
  const command = Boolean(event.ctrlKey || event.metaKey);
  if (!command) return true;
  return TEXT_FIELD_COMMANDS.has(event.key.toLowerCase());
}

/** Resolves the configurable bindings the workspace settings expose. */
export function bindingAction(
  event: ShortcutEvent,
  bindings: ShortcutBinding[]
): string | undefined {
  const parts: string[] = [];
  if (event.ctrlKey || event.metaKey) parts.push('ctrl');
  if (event.altKey) parts.push('alt');
  if (event.shiftKey) parts.push('shift');
  const key = event.key === ' ' ? 'space' : event.key.toLowerCase();
  if (!['control', 'meta', 'alt', 'shift'].includes(key)) parts.push(key);
  const normalized = normalizeShortcut(parts.join('+'));
  return bindings.find((binding) => normalizeShortcut(binding.keys) === normalized)?.action;
}

const NUDGE_KEYS: Record<string, NudgeDirection> = {
  ArrowLeft: 'left',
  ArrowRight: 'right',
  ArrowUp: 'up',
  ArrowDown: 'down'
};

const TOOL_KEYS: Record<string, SelectionTool> = {
  w: 'magic_wand',
  c: 'color_range',
  b: 'brush',
  e: 'eraser'
};

/**
 * Maps a keystroke to the single action it should perform, or `null`.
 *
 * Order matters: the transform box claims the arrow keys and Escape while it is
 * open, layer commands come before selection commands because they use
 * combinations selections left free, and the configurable bindings are resolved
 * last so a user-chosen key can never shadow a built-in editing command.
 */
export function resolveShortcut(
  event: ShortcutEvent,
  context: ShortcutContext
): ShortcutIntent | null {
  if (context.modalOpen) return null;

  if (event.key === 'Escape' && context.settingsOpen) return { type: 'close_settings' };

  const typing = isTextEntryTarget(event.target);
  // Withheld before anything else is considered, so no branch below can leak a
  // command into a field the user is typing in.
  if (typing && suppressedWhileTyping(event)) return null;

  const command = Boolean(event.ctrlKey || event.metaKey);
  const key = event.key.toLowerCase();

  if (context.transformActive && !typing) {
    if (event.key === 'Escape') return { type: 'transform_cancel' };
    if (event.key === 'Enter') return { type: 'transform_commit' };
    const direction = NUDGE_KEYS[event.key];
    if (direction && !command && !event.altKey && !ownsArrowKeys(event.target)) {
      return { type: 'transform_nudge', direction, large: Boolean(event.shiftKey) };
    }
  }

  if (context.hasImage && !typing) {
    if (context.hasLayers) {
      if (command && event.shiftKey && key === 'n') return { type: 'layer', action: 'new' };
      if (command && !event.shiftKey && key === 'j') return { type: 'layer', action: 'duplicate' };
      if (command && !event.shiftKey && key === 'g') return { type: 'layer', action: 'group' };
      if (command && event.shiftKey && key === 'g') return { type: 'layer', action: 'ungroup' };
      if (!command && !event.altKey && event.key === 'Delete' && context.hasActiveLayer) {
        return { type: 'layer', action: 'delete' };
      }
      // Every transform key holds Ctrl. The bare letters T, R, C, I, Q, M, L,
      // W, B, and E are already spoken for by the workspace bindings and the
      // selection tools, so a transform must never claim one.
      if (command && !event.shiftKey && !event.altKey && key === 't') {
        return { type: 'transform', action: 'toggle' };
      }
      if (command && event.shiftKey && key === 'h') {
        return { type: 'transform', action: 'flip_horizontal' };
      }
      if (command && event.shiftKey && key === 'u') {
        return { type: 'transform', action: 'flip_vertical' };
      }
      if (command && event.shiftKey && key === 'r') {
        return { type: 'transform', action: 'reset' };
      }
    }

    if (command && key === 'a') return { type: 'mask', action: 'select_all' };
    if (command && !event.shiftKey && key === 'd') return { type: 'mask', action: 'deselect' };
    if (command && event.shiftKey && key === 'i') return { type: 'mask', action: 'invert' };
    if (!command && !event.altKey && key === 'q') return { type: 'overlay_visibility' };
    if (!command && !event.altKey && key === 'm') {
      return {
        type: 'tool',
        tool: context.selectionTool === 'rectangle' ? 'ellipse' : 'rectangle'
      };
    }
    if (!command && !event.altKey && key === 'l') {
      return { type: 'tool', tool: context.selectionTool === 'freehand' ? 'polygon' : 'freehand' };
    }
    if (!command && !event.altKey && TOOL_KEYS[key]) {
      return { type: 'tool', tool: TOOL_KEYS[key] };
    }
    if (event.key === 'Escape' && context.selectionBusy) return { type: 'cancel_mask_operation' };
  }

  const action = bindingAction(event, context.bindings);
  return action ? { type: 'binding', action } : null;
}
