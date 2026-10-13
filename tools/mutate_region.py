"""Mutation checks for choosing a RAW region on the picture. Usage: python -I mutate_region.py [--dry] [index ...]"""
import os
import sys
sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
from pfenv import CARGO_SH, CRATE_PATH, REPO_PATH, WORK_PATH  # noqa: E402
import subprocess
import sys

ROOT = str(CRATE_PATH)
ORIENT = ROOT + r'\src\orientation.rs'
DNG = ROOT + r'\src\raw\dng.rs'
EDITOR = ROOT + r'\src\commands\editor.rs'
RAWCMD = ROOT + r'\src\commands\raw.rs'
SRCDNG = ROOT + r'\src\source\dng.rs'
PROBE = ROOT + r'\src\source\probe.rs'
CMDSRC = ROOT + r'\src\commands\source.rs'
RAW = ROOT + r'\src\raw.rs'

ORI_T = ['test', '--lib', 'orientation::']
DNG_T = ['test', '--lib', 'raw::dng']
RAW_T = ['test', '--lib', 'raw::tests']
RP = ['test', '--test', 'raw_region_picture']
UP = ['test', '--test', 'raw_upright']
META = ['test', '--test', 'raw_metadata_ipc']

MUTANTS = [
    ('unplace quarter turn 6 is the forward turn', ORIENT,
     '        6 => (y, height - 1 - x),', '        6 => (height - 1 - y, x),', ORI_T),
    ('unplace quarter turn 8 is the forward turn', ORIENT,
     '        8 => (width - 1 - y, x),', '        8 => (y, width - 1 - x),', ORI_T),
    ('unplace transverse 7 is the transpose', ORIENT,
     '        7 => (width - 1 - y, height - 1 - x),', '        7 => (y, x),', ORI_T),
    ('unplace mirror 2 is nothing', ORIENT,
     'pub const fn unplace(orientation: u8, width: u32, height: u32, x: u32, y: u32) -> (u32, u32) {\n'
     '    match orientation {\n        2 => (width - 1 - x, y),',
     'pub const fn unplace(orientation: u8, width: u32, height: u32, x: u32, y: u32) -> (u32, u32) {\n'
     '    match orientation {\n        2 => (x, y),', ORI_T),
    ('the picture offset x is dropped', DNG,
     '            x: self.rect.x + first.0.min(last.0),', '            x: first.0.min(last.0),', DNG_T),
    ('the picture offset y is dropped', DNG,
     '            y: self.rect.y + first.1.min(last.1),', '            y: first.1.min(last.1),', DNG_T),
    ('the chosen width is one short', DNG,
     '            y: self.rect.y + first.1.min(last.1),\n            width: first.0.abs_diff(last.0) + 1,',
     '            y: self.rect.y + first.1.min(last.1),\n            width: first.0.abs_diff(last.0),', DNG_T),
    ('a rectangle off the picture is mapped', DNG,
     '        if !chosen.is_within(width, height) {\n            return None;\n        }',
     '        if false {\n            return None;\n        }', DNG_T),
    ('a sensor rectangle across the border is placed', DNG,
     '            || rect.right() > self.rect.right()\n', '', DNG_T),
    ('a sensor rectangle is placed without the picture offset', DNG,
     '        let (local_x, local_y) = (rect.x - self.rect.x, rect.y - self.rect.y);',
     '        let (local_x, local_y) = (rect.x % self.rect.width, rect.y % self.rect.height);', DNG_T),
    ('the presented size ignores the turn', DNG,
     '        crate::orientation::oriented_size(self.orientation, self.rect.width, self.rect.height)',
     '        (self.rect.width, self.rect.height)', DNG_T),
    ('the header geometry ignores the orientation tag', DNG,
     '        orientation: u8::try_from(read_orientation(&file)).unwrap_or(1),',
     '        orientation: 1,', DNG_T),
    ('the header geometry ignores the default crop', DNG,
     '        read_default_crop(raw),\n    )\n    .unwrap_or(whole);',
     '        None,\n    )\n    .unwrap_or(whole);', DNG_T),
    ('a region is opened as the rectangle chosen, not the photosites', EDITOR,
     'let rect = geometry.to_sensor(&chosen).ok_or_else(|| {', 'let rect = Some(chosen).ok_or_else(|| {', RP),
    ('a region is not turned', EDITOR,
     '.turned_upright(u16::from(applied))', '.turned_upright(1)', RP),
    ('the layer records the chosen rectangle as its view', EDITOR,
     '(developed, summary, applied, None, Some(rect))', '(developed, summary, applied, None, Some(chosen))', RP),
    ('the tag is not spent for a region', EDITOR,
     'found.orientation_applied = true;', 'found.orientation_applied = region.is_none();', UP),
    ('the layer forgets the turn of a region', EDITOR,
     '(developed, summary, applied, None, Some(rect))', '(developed, summary, 1, None, Some(rect))', RP),
    ('re-developing a region does not turn it', RAWCMD,
     '                let developed = developed.turned_upright(applied)?;\n                Ok((summary, developed))',
     '                let _ = applied;\n                Ok((summary, developed))', RP),
    ('exporting a region does not turn it', RAWCMD,
     '                .and_then(|developed| developed.turned_upright(applied))\n                .map_err(map)?,',
     '                .map_err(map)?,', RP),
    ('where a region lies on the picture is asked the wrong way', RAWCMD,
     '.and_then(|geometry| geometry.from_sensor(&view)))', '.and_then(|geometry| geometry.to_sensor(&view)))', RP),
    ('the chooser preview is not cut', SRCDNG,
     'developed = developed.cropped(&rect).map_err(map_raw)?;', 'let _ = rect;', RP),
    ('the chooser preview is not turned', SRCDNG,
     '.turned_upright(u16::from(geometry.orientation))', '.turned_upright(1)', RP),
    ('the probed size is the frame', PROBE,
     '            Some(geometry) => geometry.presented_size(),', '            Some(_) => (self.width, self.height),', RP),
    ('the dialog is told the sensor size', CMDSRC,
     '        let (width, height) = probed.presented();', '        let (width, height) = (probed.width, probed.height);', RP),
    ('a region may not be turned', RAW,
     '        if let Some(crop) = &self.crop {\n            if self.view.is_some() {',
     '        if self.view.is_some() && self.orientation_applied != 1 {\n'
     '            return Err(RawError::InvalidMetadata("a region is not turned".into()));\n'
     '        }\n'
     '        if let Some(crop) = &self.crop {\n            if self.view.is_some() {', RAW_T),
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
        # The restored file keeps its old mtime on some filesystems; make cargo see it.
        import os
        os.utime(path, None)
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
