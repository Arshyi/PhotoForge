"""Mutation checks for the 15.3 metadata code: break one thing, expect a test to fail, restore."""
import os
import sys
sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
from pfenv import CARGO_SH, CRATE_PATH, REPO_PATH, WORK_PATH  # noqa: E402
import os
import shutil
import subprocess
import sys

ROOT = str(CRATE_PATH)
CARGO = str(CARGO_SH)

MUTANTS = [
    ('GPS directory left in the block (only its pointer removed)', 'src/metadata/exif.rs',
     '            erase_ifd(&mut out, gps)?;\n', '            let _ = gps;\n'),
    ('thumbnail bytes left in the block', 'src/metadata/exif.rs',
     '            zero(&mut out, offset, length)?;\n', '            let _ = (offset, length);\n'),
    ('serial numbers and owner kept', 'src/metadata/exif.rs',
     '            removed.identifiers |= delete_tags(&mut out, order, exif_at, &IDENTIFYING_TAGS)? > 0;\n',
     ''),
    ('orientation not applied when a file is opened', 'src/infrastructure/image_io.rs',
     '            decoded.apply_orientation(turn);\n', '            let _ = turn;\n'),
    ('the default policy keeps everything', 'src/metadata/mod.rs',
     '    KeepAll,\n    /// The default: the place and the identifiers go, the description of the picture stays.\n    #[default]\n    RemovePrivate,',
     '    #[default]\n    KeepAll,\n    /// The default: the place and the identifiers go, the description of the picture stays.\n    RemovePrivate,'),
    ('an XMP packet that names a place is kept', 'src/metadata/mod.rs',
     '        if private && findings.any() {', '        if false && private && findings.any() {'),
    ('IPTC place fields kept', 'src/metadata/iptc.rs',
     '        if set.record == 2 && LOCATION_DATASETS.contains(&set.number) {\n            continue;\n        }\n',
     ''),
    ('IPTC with no place in PNG reported as kept', 'src/metadata/mod.rs',
     '        if present && !stored {', '        if false && present && !stored {'),
]

TESTS = [
    ['test', '--all-features', '--lib', 'metadata::'],
    ['test', '--all-features', '--test', 'metadata_ipc'],
]


def run(args):
    cmd = ['bash', CARGO.replace('\\', '/')] + args
    result = subprocess.run(cmd, capture_output=True, text=True, timeout=900)
    text = result.stdout + result.stderr
    failed = [line.strip() for line in text.splitlines() if line.startswith('test ') and line.rstrip().endswith('FAILED')]
    compile_error = 'error[' in text or 'could not compile' in text
    return failed, compile_error, result.returncode


def main():
    only = sys.argv[1:]
    for name, path, old, new in MUTANTS:
        if only and not any(o in name for o in only):
            continue
        full = os.path.join(ROOT, path)
        original = open(full, 'rb').read().decode()
        crlf = '\r\n' in original
        o, n = (old.replace('\n', '\r\n'), new.replace('\n', '\r\n')) if crlf else (old, new)
        if original.count(o) != 1:
            print(f'SKIP  {name}: pattern found {original.count(o)} times')
            continue
        backup = full + '.mutbak'
        shutil.copyfile(full, backup)
        try:
            open(full, 'wb').write(original.replace(o, n).encode())
            caught = []
            broken = False
            for args in TESTS:
                failed, compile_error, _ = run(args)
                if compile_error:
                    broken = True
                    break
                caught += failed
            if broken:
                print(f'BROKEN {name}: the mutant did not compile')
            elif caught:
                print(f'CAUGHT {name}: {len(caught)} failing; e.g. {caught[0][:110]}')
            else:
                print(f'SURVIVED {name}')
        finally:
            shutil.move(backup, full)
            # A restored file keeps the old time of its copy; cargo would then take the last mutant build as current.
            os.utime(full, None)
    print('restored')


main()
