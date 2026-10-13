import os
import sys
sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
from pfenv import CARGO_SH, CRATE_PATH, REPO_PATH, WORK_PATH  # noqa: E402
import subprocess
import sys

root = str(REPO_PATH)
files = subprocess.run(['git', 'ls-files', '-m', '-o', '--exclude-standard'], cwd=root, capture_output=True, text=True).stdout.split('\n')
files += subprocess.run(['git', 'ls-files'], cwd=root, capture_output=True, text=True).stdout.split('\n')
seen = set()
bad = 0
for f in files:
    if not f or f in seen:
        continue
    seen.add(f)
    if not f.endswith(('.rs', '.ts', '.svelte', '.md', '.json', '.js', '.toml', '.html', '.css')):
        continue
    try:
        data = open(root + '\\' + f, 'rb').read()
    except OSError:
        continue
    odd = [(i, b) for i, b in enumerate(data) if b < 32 and b not in (9, 10, 13)]
    if odd:
        bad += 1
        i, b = odd[0]
        print(f, len(odd), 'first at', i, hex(b), repr(data[max(0, i - 30):i + 30]))
print('files scanned', len(seen), 'with control characters', bad)
