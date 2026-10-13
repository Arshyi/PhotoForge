"""Downloads the PSD/PSB test fixtures of github.com/psd-tools/psd-tools (MIT licence) into a scratch folder.
They are data for the reader's tests and are never committed, executed or redistributed. Sizes are checked against the
repository's own tree listing."""
import concurrent.futures
import json
import os
import sys
import urllib.request

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
from pfenv import WORK_PATH  # noqa: E402

TREE = str(WORK_PATH / 'psd-tools-tree.json')
DEST = str(WORK_PATH / 'psd-samples' / 'psd-tools')
BASE = 'https://raw.githubusercontent.com/psd-tools/psd-tools/main/'
API = 'https://api.github.com/repos/psd-tools/psd-tools/git/trees/main?recursive=1'

os.makedirs(WORK_PATH, exist_ok=True)
if not os.path.exists(TREE):
    request = urllib.request.Request(API, headers={'User-Agent': 'photoforge-validation'})
    with urllib.request.urlopen(request, timeout=60) as response:
        open(TREE, 'wb').write(response.read())
tree = json.load(open(TREE))['tree']
items = [e for e in tree if e['type'] == 'blob' and e['path'].lower().endswith(('.psd', '.psb'))]


def fetch(entry):
    target = os.path.join(DEST, entry['path'].replace('/', os.sep))
    os.makedirs(os.path.dirname(target), exist_ok=True)
    if os.path.exists(target) and os.path.getsize(target) == entry['size']:
        return entry['path'], 'cached', entry['size']
    url = BASE + entry['path']
    with urllib.request.urlopen(url, timeout=60) as response:
        data = response.read()
    if len(data) != entry['size']:
        return entry['path'], f"SIZE MISMATCH {len(data)} != {entry['size']}", len(data)
    with open(target, 'wb') as handle:
        handle.write(data)
    return entry['path'], 'ok', len(data)


bad = []
total = 0
with concurrent.futures.ThreadPoolExecutor(max_workers=8) as pool:
    for path, status, size in pool.map(fetch, items):
        total += size
        if status not in ('ok', 'cached'):
            bad.append((path, status))
print(f'files: {len(items)}; bytes: {total}; problems: {len(bad)}')
for item in bad[:20]:
    print('PROBLEM', item)
