import { describe, expect, it } from 'vitest';
import { closeOnFailure, decideClose, type CloseState } from './close';

/** Every combination of the three inputs. */
const everyState: CloseState[] = [false, true].flatMap((busy) =>
  [false, true].flatMap((dirty) =>
    [false, true].map((alreadyWarned) => ({ busy, dirty, alreadyWarned }))
  )
);

describe('deciding whether the window may close', () => {
  /**
   * The rule the whole module exists for. Tauri vetoes the close itself once a
   * close-requested listener is registered, so a state that neither closes nor
   * puts a prompt on screen is an application that cannot be quit.
   */
  it('always either closes or asks, for every state', () => {
    for (const state of everyState) {
      const decision = decideClose(state);
      if (decision.action === 'prompt') {
        expect(['busy', 'dirty']).toContain(decision.prompt);
      } else {
        expect(decision.action).toBe('close');
      }
    }
    expect(everyState).toHaveLength(8);
  });

  it('closes a clean idle document without asking', () => {
    expect(decideClose({ busy: false, dirty: false, alreadyWarned: false })).toEqual({
      action: 'close'
    });
    // A stale warning from an operation that has since finished must not linger
    // into a decision about a clean document.
    expect(decideClose({ busy: false, dirty: false, alreadyWarned: true })).toEqual({
      action: 'close'
    });
  });

  it('asks before discarding unsaved changes', () => {
    expect(decideClose({ busy: false, dirty: true, alreadyWarned: false })).toEqual({
      action: 'prompt',
      prompt: 'dirty'
    });
  });

  it('warns once while busy and then gets out of the way', () => {
    expect(decideClose({ busy: true, dirty: false, alreadyWarned: false })).toEqual({
      action: 'prompt',
      prompt: 'busy'
    });
    // The escape hatch: a busy flag that never clears costs a second click, not
    // the application. Without this, a wedged operation leaves Task Manager as
    // the only way to quit.
    expect(decideClose({ busy: true, dirty: false, alreadyWarned: true })).toEqual({
      action: 'close'
    });
    expect(decideClose({ busy: true, dirty: true, alreadyWarned: true })).toEqual({
      action: 'close'
    });
  });

  /** Busy is the more urgent warning, so it is the one shown. */
  it('reports the busy operation ahead of the unsaved changes', () => {
    expect(decideClose({ busy: true, dirty: true, alreadyWarned: false })).toEqual({
      action: 'prompt',
      prompt: 'busy'
    });
  });

  it('closes when the decision itself could not be made', () => {
    expect(closeOnFailure).toEqual({ action: 'close' });
  });

  it('reads no state beyond what it is given', () => {
    const state: CloseState = { busy: true, dirty: true, alreadyWarned: false };
    const frozen = Object.freeze({ ...state });
    expect(() => decideClose(frozen)).not.toThrow();
    expect(frozen).toEqual(state);
  });
});
