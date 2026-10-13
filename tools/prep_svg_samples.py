import os
import sys
sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
from pfenv import CARGO_SH, CRATE_PATH, REPO_PATH, WORK_PATH  # noqa: E402
import os
import re

src = str(WORK_PATH / 'svg-samples')
dst = str(WORK_PATH / 'svg-samples-256')
os.makedirs(dst, exist_ok=True)
count = 0
for name in sorted(os.listdir(src)):
    text = open(os.path.join(src, name), encoding='utf8').read()
    match = re.search(r'<svg\b[^>]*>', text)
    if not match:
        continue
    tag = match.group(0)
    new = re.sub(r'\s(width|height)="[^"]*"', '', tag)
    new = new[:-1].rstrip() + ' width="256" height="256">'
    if 'viewBox' not in new:
        continue
    open(os.path.join(dst, name), 'w', encoding='utf8').write(text.replace(tag, new, 1))
    count += 1
print(count, 'prepared')
