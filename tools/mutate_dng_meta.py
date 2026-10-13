"""Mutation checks for the DNG metadata increment: break the code one way at a time and require a
test to fail. Every file is restored (and verified) after each mutant, however the run ends.

Usage: python -I mutate_dng_meta.py [first_index]
"""
import os
import sys
sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
from pfenv import (CARGO_SH, CRATE_PATH, REPO_PATH, WORK_PATH, dry_run,  # noqa: E402
                   fit_line_ends)
import subprocess
import sys

ROOT = str(CRATE_PATH)
TIFF = ROOT + r'\src\metadata\tiff.rs'
MOD = ROOT + r'\src\metadata\mod.rs'
PROJECT = ROOT + r'\src\layers\project.rs'
EDITOR = ROOT + r'\src\commands\editor.rs'
BATCH = ROOT + r'\src\application\batch.rs'

LIB_METADATA = ['test', '--lib', 'metadata::']
LIB_PROJECT = ['test', '--lib', 'layers::project']
LIB_BATCH = ['test', '--lib', 'application::batch']
IPC = ['test', '--test', 'raw_metadata_ipc']

MUTANTS = [
    ('orientation is not a descriptive tag', TIFF,
     '0x010E, 0x010F, 0x0110, 0x0112, 0x011A', '0x010E, 0x010F, 0x0110, 0x0111, 0x011A', LIB_METADATA),
    ('the maker note is let through', TIFF,
     'TAG_MAKER_NOTE | TAG_INTEROP_IFD | TAG_EXIF_IFD | TAG_GPS_IFD',
     'TAG_INTEROP_IFD | TAG_EXIF_IFD | TAG_GPS_IFD', LIB_METADATA),
    ('private data is not noticed', TIFF,
     'if ifd0.find(TAG_DNG_PRIVATE_DATA).is_some() {',
     'if ifd0.find(TAG_DNG_PRIVATE_DATA).is_some() && false {', LIB_METADATA),
    ('an Exif maker note is not noticed', TIFF,
     'if exif_ifd.find(TAG_MAKER_NOTE).is_some() {', 'if false {', LIB_METADATA),
    ('no per-field cap', TIFF,
     'if declared > MAX_FIELD_BYTES as u64 || cost > budget.left as u64 {',
     'if cost > budget.left as u64 {', LIB_METADATA),
    ('no block budget', TIFF,
     'if declared > MAX_FIELD_BYTES as u64 || cost > budget.left as u64 {',
     'if declared > MAX_FIELD_BYTES as u64 {', LIB_METADATA),
    ('entries are not sorted', TIFF,
     'entries.sort_by_key(|(tag, _)| *tag);', '', LIB_METADATA),
    ('values are not padded to even offsets', TIFF,
     '''values.extend_from_slice(&field.value);
                        if values.len() % 2 == 1 {
                            values.push(0);
                        }''',
     'values.extend_from_slice(&field.value);', LIB_METADATA),
    ('the XMP terminator stays', TIFF,
     'while bytes.last() == Some(&0) {', 'while false && bytes.last() == Some(&0) {', LIB_METADATA),
    ('no Photoshop-resource IPTC', TIFF,
     '(TAG_PHOTOSHOP, &[1, 7][..])', '(TAG_IPTC_NAA, &[1, 7][..])', LIB_METADATA),
    ('unknown field types are copied', TIFF,
     'const KNOWN_KINDS: std::ops::RangeInclusive<u16> = 1..=12;',
     'const KNOWN_KINDS: std::ops::RangeInclusive<u16> = 1..=99;', LIB_METADATA),
    ('the GPS directory is not followed', TIFF,
     'if let Some(gps_ifd) = follow(file, order, ifd0, TAG_GPS_IFD) {',
     'if let Some(gps_ifd) = follow(file, order, ifd0, TAG_GPS_IFD).filter(|_| false) {', LIB_METADATA),
    ('a repeated tag is not skipped', TIFF,
     'if out.iter().any(|field| field.tag == entry.tag) {', 'if false {', LIB_METADATA),
    ('the interoperability pointer is not rebuilt', TIFF,
     'directory.links.push((TAG_INTEROP_IFD, interop_index));', '', LIB_METADATA),
    ('oversize is not flagged', TIFF,
     'left |= left_behind::OVERSIZE;', 'left |= 0;', LIB_METADATA),
    ('summary ignores what was left behind', MOD,
     'has_maker_note: exif.has_maker_note || self.left_behind & left_behind::MAKER_NOTE != 0,',
     'has_maker_note: exif.has_maker_note,', LIB_METADATA),
    ('the export does not say what was left', MOD,
     'if source.left_behind & left_behind::MAKER_NOTE != 0 {', 'if false {', LIB_METADATA),
    ('a project forgets what was left', PROJECT,
     'metadata.left_behind = value & known;', 'metadata.left_behind = 0;', LIB_PROJECT),
    ('a RAW document holds no metadata', EDITOR,
     'source_metadata: source_metadata.map(std::sync::Arc::new),', 'source_metadata: None,', IPC),
    ('a RAW open shows no summary', EDITOR,
     'source_metadata: source_metadata.as_ref().map(|found| found.summary()),',
     'source_metadata: None,', IPC),
    ('a batch of DNGs drops their metadata', BATCH,
     '(developed.image.into(), path.clone(), source_metadata)',
     '(developed.image.into(), path.clone(), None)', LIB_BATCH),
]


def run(args):
    proc = subprocess.run(
        ['bash', str(CARGO_SH)] + args,
        capture_output=True, text=True, encoding='utf-8', errors='replace', timeout=1500)
    return proc.returncode, proc.stdout + proc.stderr


def main():
    only = {int(value) for value in sys.argv[1:]}
    results = []
    for index, (name, path, old, new, command) in enumerate(MUTANTS):
        if only and index not in only:
            continue
        original = open(path, encoding='utf-8', newline='').read()
        old, new = fit_line_ends(original, old, new)
        count = original.count(old)
        if count != 1:
            results.append((index, name, 'BAD PATTERN (%d matches)' % count))
            print(index, name, 'BAD PATTERN', count, flush=True)
            continue
        try:
            open(path, 'w', encoding='utf-8', newline='').write(original.replace(old, new))
            code, output = run(command)
        finally:
            open(path, 'w', encoding='utf-8', newline='').write(original)
        assert open(path, encoding='utf-8', newline='').read() == original, 'restore failed'
        if 'error[' in output or 'could not compile' in output:
            verdict = 'COMPILE ERROR'
        elif code != 0 and 'test result: FAILED' in output:
            failed = [line.strip() for line in output.splitlines() if line.strip().endswith('FAILED')
                      and line.strip().startswith('test ')]
            verdict = 'caught by %d test(s): %s' % (len(failed), '; '.join(failed[:2]))
        elif code == 0:
            verdict = 'SURVIVED'
        else:
            verdict = 'UNCLEAR rc=%d' % code
        results.append((index, name, verdict))
        print(index, name, '->', verdict, flush=True)
    print('---- summary')
    for index, name, verdict in results:
        print(index, name, '->', verdict)


dry_run([(m[0], m[1], m[2]) for m in MUTANTS])


main()
