"""Download a few dozen real-world SVG icons (MIT / CC0 sets) into a scratch folder.

feathericons/feather (MIT), twbs/icons (MIT), simple-icons/simple-icons (CC0-1.0).
Scratch only: never committed, never executed; each file is parsed as data.
"""
import os
import sys
sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
from pfenv import CARGO_SH, CRATE_PATH, REPO_PATH, WORK_PATH  # noqa: E402
import json
import os
import re
import urllib.request

OUT = str(WORK_PATH / 'svg-samples')
SOURCES = [
    ('feather', 'feathericons/feather', 'main', 'icons', 70),
    ('bootstrap', 'twbs/icons', 'main', 'icons', 90),
    ('simple', 'simple-icons/simple-icons', 'develop', 'icons', 70),
]


def get(url):
    request = urllib.request.Request(url, headers={'User-Agent': 'photoforge-fixture-check'})
    with urllib.request.urlopen(request, timeout=60) as response:
        return response.read()


os.makedirs(OUT, exist_ok=True)
total = 0
for tag, repo, branch, folder, count in SOURCES:
    listing = json.loads(get(f'https://api.github.com/repos/{repo}/contents/{folder}?ref={branch}'))
    names = sorted(entry['name'] for entry in listing if entry['name'].endswith('.svg'))
    step = max(1, len(names) // count)
    chosen = names[::step][:count]
    got = 0
    for name in chosen:
        data = get(f'https://raw.githubusercontent.com/{repo}/{branch}/{folder}/{name}')
        if len(data) > 200_000:
            continue
        with open(os.path.join(OUT, f'{tag}-{name}'), 'wb') as handle:
            handle.write(data)
        got += 1
    print(tag, repo, len(names), 'listed,', got, 'downloaded')
    total += got
print(total, 'files in', OUT)
