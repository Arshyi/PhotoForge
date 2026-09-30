/**
 * Deciding whether the window may close.
 *
 * This lives outside the component because of how Tauri delivers the request.
 * The moment the frontend registers a close-requested listener, Tauri vetoes
 * every close itself and closes the window only if the handler declines to
 * prevent it. The frontend is therefore the sole authority on whether
 * PhotoForge can be quit at all, and the one rule that matters is that no
 * combination of state may leave the user unable to close it.
 *
 * Keeping the decision pure makes that rule something a test can check across
 * every input, rather than a property buried in an event callback.
 */

export interface CloseState {
  /** A file, layer, export or recovery operation has not finished. */
  busy: boolean;
  /** The document holds changes that have not been saved. */
  dirty: boolean;
  /** A previous close request was already refused because of `busy`. */
  alreadyWarned: boolean;
}

export type CloseDecision =
  /** Let the window close. */
  | { action: 'close' }
  /** Hold the window open and ask; `prompt` selects the wording. */
  | { action: 'prompt'; prompt: 'busy' | 'dirty' };

export function decideClose(state: CloseState): CloseDecision {
  if (state.busy) {
    // Asking twice is how the user escapes a wedged operation. A flag that
    // never clears must cost them a second click, not the application.
    return state.alreadyWarned ? { action: 'close' } : { action: 'prompt', prompt: 'busy' };
  }
  if (state.dirty) return { action: 'prompt', prompt: 'dirty' };
  return { action: 'close' };
}

/**
 * The decision to use when working out the real one fails.
 *
 * Always closing. Whether to warn about unsaved changes is not worth trapping
 * someone in the application over, and a handler that throws while Tauri has
 * already vetoed the close is exactly how a close button comes to do nothing.
 */
export const closeOnFailure: CloseDecision = { action: 'close' };
