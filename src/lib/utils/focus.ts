/**
 * Giving focus back.
 *
 * A dialog takes the keyboard when it opens. When it closes the person should be
 * where they were, not at the top of the page: a keyboard or screen-reader user who
 * has to find their place again has been made to pay for opening a dialog.
 *
 * Call this where the dialog is created, before it moves focus into itself, and call
 * the function it returns when the dialog goes. If the element that had focus is
 * gone by then (a dialog opened from the command palette finds the palette's own
 * input remembered, and the palette closes first), focus goes to the header's
 * Commands button, which is always there, rather than to nothing.
 */
const FALLBACK = 'header button[title^="Command palette"]';

export function rememberFocus(): () => void {
  const previous =
    typeof document !== 'undefined' && document.activeElement instanceof HTMLElement && document.activeElement !== document.body
      ? document.activeElement
      : null;
  return () => {
    // After the update that removes the dialog has finished, so a page made inert
    // for the dialog's sake is live again and can take focus.
    queueMicrotask(() => {
      const target = previous?.isConnected ? previous : document.querySelector<HTMLElement>(FALLBACK);
      target?.focus?.();
    });
  };
}
