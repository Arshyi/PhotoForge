import { readFileSync, readdirSync, statSync } from 'node:fs';
import { join, resolve } from 'node:path';
import { describe, expect, it } from 'vitest';

/**
 * The window's permissions against the Tauri calls the interface makes.
 *
 * Found by running the packaged application: the close button did nothing, in 0.13.0 and
 * in 0.14.0 until this was fixed. As soon as the interface registers a close-requested
 * handler, Tauri vetoes every close and the handler's wrapper calls `destroy()` once the
 * handler has declined to prevent it — and `destroy` is not in the default window
 * permissions, so the call is refused and the window stays open for ever. Unit tests of the
 * close decision could not see that; nothing short of the permission file can.
 */
const root = resolve(__dirname, '../../..');
const capability = JSON.parse(readFileSync(join(root, 'src-tauri/capabilities/default.json'), 'utf-8')) as {
  permissions: string[];
};

function sources(directory: string, found: string[] = []): string[] {
  for (const name of readdirSync(directory)) {
    const path = join(directory, name);
    if (statSync(path).isDirectory()) sources(path, found);
    else if (/\.(ts|svelte)$/.test(name) && !/\.test\./.test(name)) found.push(path);
  }
  return found;
}

const code = sources(join(root, 'src')).map((path) => readFileSync(path, 'utf-8')).join('\n');

describe('what the window is allowed to ask of Tauri', () => {
  it('may destroy itself, because the interface closes it that way', () => {
    // Both routes end in `destroy`: the wrapper after an unprevented close request, and the
    // in-app "Close anyway" confirmation.
    expect(code).toContain('onCloseRequested');
    expect(code).toContain('.destroy()');
    expect(capability.permissions).toContain('core:window:allow-destroy');
  });

  it('is granted nothing beyond the default set, destroy, and the two file dialogs', () => {
    // A new permission is a decision, and should be a visible one.
    expect([...capability.permissions].sort()).toEqual([
      'core:default',
      'core:window:allow-destroy',
      'dialog:allow-open',
      'dialog:allow-save'
    ]);
  });

  it('uses only window calls whose permission it has, or has been given', () => {
    // Every method called on the current window, found in the source. Those that are read-only
    // are in the default set; `destroy` is granted explicitly; anything else would be refused
    // at run time, silently from the tests' point of view, so a new one has to be added here
    // on purpose, together with its permission.
    const called = new Set([...code.matchAll(/getCurrentWindow\(\)\s*\.\s*([A-Za-z]+)/g)].map((match) => match[1]));
    const allowed = new Set(['onCloseRequested', 'destroy']);
    const unexpected = [...called].filter((name) => !allowed.has(name));
    expect(unexpected, `window calls that may need a permission: ${unexpected.join(', ')}`).toEqual([]);
    expect(called.has('destroy')).toBe(true);
  });
});
