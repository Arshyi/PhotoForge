import { readFileSync } from 'node:fs';
import { resolve } from 'node:path';
import { describe, expect, it } from 'vitest';
// The stylesheet as text: layout is not computed in jsdom, so these are the rules that keep the
// window's layout (checked in the real window by tools/validate/layout_matrix.ps1) from regressing
// to what it was before Phase 15.12. Read from disk: the test setup does not process CSS imports.
const css = readFileSync(resolve(__dirname, 'styles.css'), 'utf8').replace(/\r\n/g, '\n');

/** The declarations of the first rule whose selector is exactly `selector`. */
function ruleBody(selector: string): string {
  const escaped = selector.replace(/[.*+?^${}()|[\]\\]/g, '\\$&');
  const match = new RegExp(`(?:^|\\n)${escaped}\\s*\\{([^}]*)\\}`).exec(css);
  if (!match) throw new Error(`no rule for ${selector}`);
  return match[1];
}

describe('the window layout', () => {
  it('has one column that cannot be wider than the window', () => {
    // With no explicit column the header's width set the shell's: under about 1,350px the right
    // panel and the Settings button were pushed past the window's edge.
    expect(ruleBody('.app-shell')).toMatch(/grid-template-columns:\s*minmax\(0,\s*1fr\)/);
    expect(ruleBody('.topbar')).toMatch(/min-width:\s*0/);
  });

  it('gives up the header buttons\' labels, not their place, before the header overflows', () => {
    const query = /@media \(max-width: 1400px\)\s*\{([^}]*\{[^}]*\}[^}]*)\}/.exec(css);
    expect(query, 'a media query at 1400px').not.toBeNull();
    expect(query![1]).toMatch(/\.topbar button:not\(\.primary\) \.label\s*\{\s*display:\s*none/);
  });

  it('draws the editing panel\'s own controls in the app\'s look, at the card scale', () => {
    const rule = /(?:^|\n)aside button,\s*aside select,\s*aside input:not\([^)]*\)\s*\{([^}]*)\}/.exec(css);
    expect(rule, 'a rule for aside button, select and input').not.toBeNull();
    expect(rule![1]).toMatch(/background:\s*var\(--surface-soft\)/);
    expect(rule![1]).toMatch(/font-size:\s*0\.6rem/);
    // It has to beat `button, input { font: inherit }`, so it may not be a zero-specificity rule.
    expect(css).not.toMatch(/:where\(aside\)/);
  });
});
