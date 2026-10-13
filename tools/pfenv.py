"""Where things are, for the scripts in this directory.

The scripts here were written one at a time while the project was built and each used to carry the
paths of the machine it was written on. They now take them from here, so a clone on any machine
works with no editing.

REPO_PATH   the repository root (this directory's parent)
CRATE_PATH  src-tauri, the Rust crate
WORK_PATH   scratch space for sample corpora, dumps, reports and screenshots. It is never
            committed: it defaults to <repo>/.work (ignored by git) and PF_WORK moves it.
CARGO_SH    scripts/cargo-ext.sh, the cargo wrapper the mutation harnesses run (PF_CARGO overrides)
"""
import os
from pathlib import Path

REPO_PATH = Path(__file__).resolve().parents[1]
CRATE_PATH = REPO_PATH / 'src-tauri'
WORK_PATH = Path(os.environ.get('PF_WORK') or (REPO_PATH / '.work'))
CARGO_SH = Path(os.environ.get('PF_CARGO') or (REPO_PATH / 'scripts' / 'cargo-ext.sh'))


def fit_line_ends(text, old, new=None):
    """``old`` (and ``new``) in the line-ending convention of ``text``.

    Patterns are written with ``\\n``. A file checked out with ``core.autocrlf=true`` has ``\\r\\n``,
    and a pattern that spans lines then matches nothing; a replacement must keep the file's own
    convention too, or it leaves a line of the other kind behind.
    """
    crlf = '\r\n' in text

    def fit(value):
        value = value.replace('\r\n', '\n')
        return value.replace('\n', '\r\n') if crlf else value

    return fit(old) if new is None else (fit(old), fit(new))


def dry_run(items):
    """Handles ``--dry`` for a mutation harness, and refuses any other ``--option``.

    ``items`` is an iterable of (label, path, old): the text a mutant replaces and the file it is
    in. With ``--dry`` this prints how often each occurs (it must occur exactly once), writes
    nothing, and exits 0 if every pattern matches, 1 if one does not. Without ``--dry`` it returns
    and the harness runs as it always did. A pattern is matched in whatever line-ending convention
    the file has (``fit_line_ends``), as the harnesses do when they make the change.

    An unrecognised ``--option`` exits 2 before anything is touched: a harness that ignores its
    arguments would otherwise read ``--dry`` as "run every mutant", editing source files in place.
    """
    import sys
    flags = [argument for argument in sys.argv[1:] if argument.startswith('--')]
    unknown = [flag for flag in flags if flag != '--dry']
    if unknown:
        print('unknown option %s; the only option is --dry (check every pattern, change nothing)'
              % ' '.join(unknown))
        sys.exit(2)
    if '--dry' not in flags:
        return
    bad = 0
    count = 0
    for label, path, old in items:
        count += 1
        text = open(path, encoding='utf-8', newline='').read()
        found = text.count(fit_line_ends(text, old))
        if found != 1:
            bad += 1
        print('%s %-3s %s' % ('ok ' if found == 1 else 'BAD', found, label))
    print('%d of %d patterns match exactly once' % (count - bad, count))
    sys.exit(1 if bad else 0)
