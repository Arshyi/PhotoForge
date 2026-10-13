"""Mutation checks for turning a developed DNG upright. Usage: python -I mutate_upright.py [--dry] [index ...]"""
import os
import sys
sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
from pfenv import CARGO_SH, CRATE_PATH, REPO_PATH, WORK_PATH  # noqa: E402
import subprocess
import sys

ROOT = str(CRATE_PATH)
ORIENT = ROOT + r'\src\orientation.rs'
DEVELOP = ROOT + r'\src\raw\develop.rs'
EDITOR = ROOT + r'\src\commands\editor.rs'
RAWCMD = ROOT + r'\src\commands\raw.rs'
BATCH = ROOT + r'\src\application\batch.rs'
PROJECT = ROOT + r'\src\layers\project.rs'
RAW = ROOT + r'\src\raw.rs'

ORIENT_T = ['test', '--lib', 'orientation::']
RAW_T = ['test', '--lib', 'raw::tests']
BATCH_T = ['test', '--lib', 'application::batch']
UPRIGHT = ['test', '--test', 'raw_upright']

MUTANTS = [
    ('a half turn is not made', ORIENT, '        3 => pixels.reverse(),', '        3 => {}', ORIENT_T),
    ('a left-to-right mirror is not made', ORIENT,
     '''            for row in pixels.chunks_exact_mut(w) {
                row.reverse();
            }''', '''            for _row in pixels.chunks_exact_mut(w) {}''', ORIENT_T),
    ('a top-to-bottom mirror is a half turn', ORIENT,
     'pixels.swap(y * w + x, (h - 1 - y) * w + x);',
     'pixels.swap(y * w + x, (h - 1 - y) * w + (w - 1 - x));', ORIENT_T),
    ('the transpose is not one', ORIENT, '        5 => (y, x),', '        5 => (x, y),', ORIENT_T),
    ('a quarter turn goes the other way', ORIENT,
     '        6 => (height - 1 - y, x),', '        6 => (y, width - 1 - x),', ORIENT_T),
    ('the sensor is developed with no turn', DEVELOP,
     '.turned_upright(sensor.orientation)', '.turned_upright(1)', UPRIGHT),
    ('a whole open is not turned', EDITOR,
     'let developed = crate::raw::develop::develop_sensor_upright(',
     'let developed = crate::raw::develop::develop_sensor(', UPRIGHT),
    ('the tag is not spent', EDITOR,
     'found.orientation_applied = region.is_none();', 'found.orientation_applied = false;', UPRIGHT),
    ('the layer forgets its turn', EDITOR,
     '        orientation_applied: applied,\n', '        orientation_applied: 1,\n', UPRIGHT),
    ('re-developing does not turn', RAWCMD,
     'develop_sensor(&sensor, &for_worker, scale)?.turned_upright(applied)?;',
     'develop_sensor(&sensor, &for_worker, scale)?;', UPRIGHT),
    ('the layer export is not turned', RAWCMD,
     '''                    .and_then(|developed| developed.turned_upright(applied))
''', '', UPRIGHT),
    ('a raw layer is not turned when added', RAWCMD,
     'let developed = develop_sensor_upright(&sensor, &parameters, scale)?;',
     'let developed = develop_sensor(&sensor, &parameters, scale)?;', UPRIGHT),
    ('a batch is not turned', BATCH,
     'crate::raw::develop::develop_bytes_upright(', 'crate::raw::develop::develop_bytes(', BATCH_T),
    ('a batch does not spend the tag', BATCH,
     'found.orientation_applied = true;', 'found.orientation_applied = false;', BATCH_T),
    ('a recorded turn is version two', PROJECT,
     '.is_some_and(|raw| raw.orientation_applied != 1)',
     '.is_some_and(|raw| raw.orientation_applied != 99 && false)', UPRIGHT),
    ('a region may be turned', RAW,
     'if self.view.is_some() && self.orientation_applied != 1 {', 'if false {', RAW_T),
    ('no turn is always written', RAW,
     'fn is_no_orientation(value: &u8) -> bool {\n    *value == 1\n}',
     'fn is_no_orientation(value: &u8) -> bool {\n    *value == 0\n}', RAW_T),
]


def run(args):
    proc = subprocess.run(
        ['bash', str(CARGO_SH)] + args,
        capture_output=True, text=True, encoding='utf-8', errors='replace', timeout=1500)
    return proc.returncode, proc.stdout + proc.stderr


def main():
    args = sys.argv[1:]
    dry = '--dry' in args
    only = {int(a) for a in args if a.isdigit()}
    results = []
    for index, (name, path, old, new, command) in enumerate(MUTANTS):
        if only and index not in only:
            continue
        original = open(path, encoding='utf-8', newline='').read()
        crlf = '\r\n' in original
        pattern = old.replace('\n', '\r\n') if crlf else old
        replacement = new.replace('\n', '\r\n') if crlf else new
        count = original.count(pattern)
        if dry or count != 1:
            print(index, name, 'matches', count, flush=True)
            if count != 1:
                results.append((index, name, 'BAD PATTERN (%d matches)' % count))
            continue
        try:
            open(path, 'w', encoding='utf-8', newline='').write(original.replace(pattern, replacement))
            code, output = run(command)
        finally:
            open(path, 'w', encoding='utf-8', newline='').write(original)
        assert open(path, encoding='utf-8', newline='').read() == original, 'restore failed'
        if 'error[' in output or 'could not compile' in output:
            verdict = 'COMPILE ERROR'
        elif code != 0 and 'test result: FAILED' in output:
            failed = [line.strip() for line in output.splitlines()
                      if line.strip().endswith('FAILED') and line.strip().startswith('test ')]
            verdict = 'caught by %d test(s): %s' % (len(failed), '; '.join(failed[:2]))
        elif code == 0:
            verdict = 'SURVIVED'
        else:
            verdict = 'UNCLEAR rc=%d' % code
        results.append((index, name, verdict))
        print(index, name, '->', verdict, flush=True)
    if not dry:
        print('---- summary')
        for index, name, verdict in results:
            print(index, name, '->', verdict)


main()
