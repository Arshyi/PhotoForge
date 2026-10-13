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
