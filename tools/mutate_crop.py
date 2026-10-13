"""Mutation checks for cutting a developed DNG to its declared picture. Usage: python -I mutate_crop.py [--dry] [index ...]"""
import os
import sys
sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
from pfenv import CARGO_SH, CRATE_PATH, REPO_PATH, WORK_PATH  # noqa: E402
import subprocess
import sys

ROOT = str(CRATE_PATH)
DNG = ROOT + r'\src\raw\dng.rs'
DEVELOP = ROOT + r'\src\raw\develop.rs'
EDITOR = ROOT + r'\src\commands\editor.rs'
RAWCMD = ROOT + r'\src\commands\raw.rs'
BATCH = ROOT + r'\src\application\batch.rs'
PROJECT = ROOT + r'\src\layers\project.rs'
RAW = ROOT + r'\src\raw.rs'

RAW_T = ['test', '--lib', 'raw::']
BATCH_T = ['test', '--lib', 'application::batch']
CROP = ['test', '--test', 'raw_crop']

MUTANTS = [
    ('the active area origin is ignored', DNG,
     '(area_x + x, area_y + y, width, height)', '(x, y, width, height)', RAW_T),
    ('a crop outside the active area is believed', DNG,
     'Some((x, y, width, height)) if within(x, y, width, height) => {',
     'Some((x, y, width, height)) => {', RAW_T),
    ('a picture that is the whole sensor is a cut', DNG,
     '(rect.pixels() != u64::from(self.width) * u64::from(self.height)).then_some(rect)',
     'Some(rect)', RAW_T),
    ('the left edge is not cut', DEVELOP,
     'let x0 = scale(u64::from(rect.x), width, source_width).min(width - 1);',
     'let x0 = 0_u32.min(width - 1);', RAW_T),
    ('the right edge is the width', DEVELOP,
     'let x1 = scale(rect.right(), width, source_width).clamp(x0 + 1, width);',
     'let x1 = scale(u64::from(rect.width), width, source_width).clamp(x0 + 1, width);', RAW_T),
    ('the picture is not cut', DEVELOP,
     'Some(rect) => developed.cropped(&rect)?,', 'Some(_) => developed,', RAW_T),
    ('a whole open is not cut', EDITOR,
     'let developed = crate::raw::develop::develop_sensor_picture(',
     'let developed = crate::raw::develop::develop_sensor(', CROP),
    ('the layer forgets its cut', EDITOR,
     '        orientation_applied: applied,\n        crop,\n',
     '        orientation_applied: applied,\n        crop: None,\n', CROP),
    ('re-developing does not cut', RAWCMD,
     'developed = developed.cropped(rect)?;', 'let _ = rect;', CROP),
    ('the layer export is not cut', RAWCMD,
     'Some(rect) => developed.cropped(rect),', 'Some(_) => Ok(developed),', CROP),
    ('a raw layer is not cut when added', RAWCMD,
     'crop: sensor.picture_rect(),', 'crop: None,', CROP),
    ('a batch is not cut', BATCH,
     'crate::raw::develop::develop_bytes_picture(', 'crate::raw::develop::develop_bytes(', BATCH_T),
    ('a recorded cut is version two', PROJECT,
     '|| raw.crop.is_some())', '|| raw.crop.is_some() && false)', CROP),
    ('a region may be cut', RAW,
     '            if self.view.is_some() {\n'
     '                return Err(RawError::InvalidMetadata(\n'
     '                    "a region of a sensor is in the sensor\'s coordinates and is not cut".into(),',
     '            if false {\n'
     '                return Err(RawError::InvalidMetadata(\n'
     '                    "a region of a sensor is in the sensor\'s coordinates and is not cut".into(),',
     RAW_T),
    ('a crop may lie off the sensor', RAW,
     'if !crop.is_within(self.reference.width, self.reference.height) {', 'if false {', RAW_T),
    ('no cut is always written', RAW,
     '    #[serde(default, skip_serializing_if = "Option::is_none")]\n    pub crop:',
     '    #[serde(default)]\n    pub crop:', RAW_T),
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
