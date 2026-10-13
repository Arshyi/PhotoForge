"""Mutation checks for the three fixes found by the packaged validation. Usage: python -I mutate_fixes.py"""
import os
import sys
sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
from pfenv import CARGO_SH, CRATE_PATH, REPO_PATH, WORK_PATH  # noqa: E402
import os
import subprocess

REPO = str(REPO_PATH) + '\\'
ROOT = REPO + r'src-tauri' + '\\'

CARGO = ['bash', str(CARGO_SH)]
RF = CARGO + ['test', '--test', 'raw_flow', 'the_largest_region']
ADM = CARGO + ['test', '--lib', 'resources::admission']
RP = CARGO + ['test', '--test', 'raw_region_picture', 'a_turned_region']
VITEST = ['cmd', '/c', 'npx', 'vitest', 'run', 'src/lib/components/RegionSelector.test.ts']

MUTANTS = [
    ('the planner offers the whole pixel limit again (flow)', ROOT + r'src\resources\admission.rs',
     'let pixel_cap = u128::from(limits.working_pixels().saturating_sub(STORE_PREVIEW_PIXELS))\n        .min(probe.pixels());',
     'let pixel_cap = u128::from(limits.working_pixels()).min(probe.pixels());', RF, ROOT),
    ('the planner offers the whole pixel limit again (planner)', ROOT + r'src\resources\admission.rs',
     'let pixel_cap = u128::from(limits.working_pixels().saturating_sub(STORE_PREVIEW_PIXELS))\n        .min(probe.pixels());',
     'let pixel_cap = u128::from(limits.working_pixels()).min(probe.pixels());', ADM, ROOT),
    ('a turned region layer must be the sensor\'s way round', ROOT + r'src\layers\model.rs',
     'if want_width != *width || want_height != *height {',
     'if view.width != *width || view.height != *height {', RP, ROOT),
    ('the chooser stage is not limited in width', REPO + r'src\lib\components\RegionSelector.svelte',
     '; max-width:calc(62vh * ${sourceWidth} / ${sourceHeight})`}', '`}', VITEST, REPO),
]


def main():
    for name, path, old, new, command, cwd in MUTANTS:
        original = open(path, encoding='utf-8', newline='').read()
        crlf = '\r\n' in original
        pattern = old.replace('\n', '\r\n') if crlf else old
        replacement = new.replace('\n', '\r\n') if crlf else new
        count = original.count(pattern)
        if count != 1:
            print(name, 'BAD PATTERN', count, flush=True)
            continue
        try:
            open(path, 'w', encoding='utf-8', newline='').write(original.replace(pattern, replacement))
            proc = subprocess.run(command, cwd=cwd, capture_output=True, text=True,
                                  encoding='utf-8', errors='replace', timeout=1500)
        finally:
            open(path, 'w', encoding='utf-8', newline='').write(original)
            os.utime(path, None)
        output = proc.stdout + proc.stderr
        if 'could not compile' in output or 'error[' in output:
            verdict = 'COMPILE ERROR'
        elif proc.returncode != 0:
            failed = [line.strip() for line in output.splitlines() if 'FAILED' in line or 'FAIL ' in line or '×' in line]
            verdict = 'caught: ' + '; '.join(failed[:2])
        else:
            verdict = 'SURVIVED'
        print(name, '->', verdict, flush=True)


main()
