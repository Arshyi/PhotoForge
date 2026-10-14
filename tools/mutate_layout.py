"""Mutation checks for the window layout rules of Phase 15.11 (src/styles.css): break a rule, expect
src/styles.layout.test.ts to fail, restore.

Usage: python -I tools/mutate_layout.py [--dry] [index ...]
"""
import os
import subprocess
import sys
import time

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
from pfenv import REPO_PATH, dry_run, fit_line_ends  # noqa: E402

CSS = str(REPO_PATH / 'src' / 'styles.css')

MUTANTS = [
    ('the shell has no column of its own', CSS,
     '  grid-template-columns: minmax(0, 1fr);\n  grid-template-rows: 56px minmax(0, 1fr) 42px 28px;',
     '  grid-template-rows: 56px minmax(0, 1fr) 42px 28px;'),
    ('the header may be as wide as its content', CSS,
     '  z-index: 5;\n  min-width: 0;\n  display: flex;', '  z-index: 5;\n  display: flex;'),
    ('the header keeps its labels', CSS,
     '  .topbar button:not(.primary) .label { display: none; }', '  .topbar button:not(.primary) .label { display: inline; }'),
    ('the panel controls are the browser\'s again', CSS,
     'aside button,\naside select,\naside input:not(', 'aside .no-such-class,\naside .none,\naside input.none:not('),
    ('the panel controls are at the old size', CSS,
     '  background: var(--surface-soft);\n  font: inherit;\n  font-size: 0.6rem;\n}\naside button { cursor: pointer; }',
     '  background: var(--surface-soft);\n  font: inherit;\n  font-size: 1rem;\n}\naside button { cursor: pointer; }'),
]

dry_run([(m[0], m[1], m[2]) for m in MUTANTS])


def run(index):
    name, path, old, new = MUTANTS[index]
    original = open(path, encoding='utf-8', newline='').read()
    old, new = fit_line_ends(original, old, new)
    if original.count(old) != 1:
        return 'BAD PATTERN (%d matches)' % original.count(old)
    try:
        open(path, 'w', encoding='utf-8', newline='').write(original.replace(old, new))
        os.utime(path, None)
        result = subprocess.run(['cmd', '/c', 'npx', 'vitest', 'run', 'src/styles.layout.test.ts'],
                                cwd=str(REPO_PATH), capture_output=True, text=True,
                                encoding='utf-8', errors='replace', timeout=300)
    finally:
        open(path, 'w', encoding='utf-8', newline='').write(original)
        os.utime(path, (time.time() + 5, time.time() + 5))
    return 'SURVIVED' if result.returncode == 0 else 'caught'


if __name__ == '__main__':
    chosen = [int(a) for a in sys.argv[1:] if a.isdigit()] or range(len(MUTANTS))
    for index in chosen:
        print(index, MUTANTS[index][0], '->', run(index), flush=True)
