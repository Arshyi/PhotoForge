"""Download a small, varied set of OpenEXR sample images (AcademySoftwareFoundation/openexr-images,
BSD-3-Clause) into a scratch directory. Nothing is committed or executed."""
import os
import sys
sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
from pfenv import CARGO_SH, CRATE_PATH, REPO_PATH, WORK_PATH  # noqa: E402
import json
import os
import sys
import urllib.request

OUT = str(WORK_PATH / 'exr-samples')
os.makedirs(OUT, exist_ok=True)
API = 'https://api.github.com/repos/AcademySoftwareFoundation/openexr-images/git/trees/main?recursive=1'
RAW = 'https://raw.githubusercontent.com/AcademySoftwareFoundation/openexr-images/main/'


def get(url):
    request = urllib.request.Request(url, headers={'User-Agent': 'photoforge-validation'})
    with urllib.request.urlopen(request, timeout=60) as response:
        return response.read()


tree = json.loads(get(API))
files = [(e['path'], e['size']) for e in tree['tree'] if e['type'] == 'blob' and e['path'].lower().endswith('.exr')]
print(len(files), 'exr files in the repository')
chosen = [(p, s) for p, s in files if s <= 2_500_000]
print(len(chosen), 'of them are 2.5 MB or smaller')
manifest = []
for path, size in chosen:
    name = path.replace('/', '__')
    target = os.path.join(OUT, name)
    if not os.path.exists(target):
        try:
            data = get(RAW + path)
        except Exception as error:  # noqa: BLE001
            print('failed', path, error)
            continue
        with open(target, 'wb') as handle:
            handle.write(data)
    manifest.append({'path': path, 'size': size, 'file': name})
with open(os.path.join(OUT, 'manifest.json'), 'w') as handle:
    json.dump(manifest, handle, indent=1)
print('downloaded', len(manifest))
