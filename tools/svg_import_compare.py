"""Render the real SVG icons with resvg and compare them with PhotoForge's import + render."""
import os
import sys
sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
from pfenv import CARGO_SH, CRATE_PATH, REPO_PATH, WORK_PATH  # noqa: E402
import io
import os
import sys

import numpy as np
import resvg_py
from PIL import Image

sys.stdout.reconfigure(encoding='utf-8')
src = str(WORK_PATH / 'svg-samples-256')
out = str(WORK_PATH / 'out' / 'svg-real')
rows = []
errors = []
for line in open(os.path.join(out, 'summary.tsv'), encoding='utf8'):
    parts = line.rstrip('\n').split('\t')
    if parts[1] != 'ok':
        errors.append((parts[0], parts[2] if len(parts) > 2 else ''))
        continue
    name = parts[0]
    theirs = Image.open(io.BytesIO(bytes(resvg_py.svg_to_bytes(svg_path=os.path.join(src, name + '.svg'))))).convert('RGBA')
    ours = Image.open(os.path.join(out, name + '.png')).convert('RGBA')
    if theirs.size != ours.size:
        rows.append((name, 999.0, 0.0, 'size'))
        continue
    a = np.asarray(theirs, dtype=np.float32)
    b = np.asarray(ours, dtype=np.float32)
    # Premultiplied, over white: how different the two look.
    def over_white(x):
        alpha = x[..., 3:4] / 255.0
        return x[..., :3] * alpha + 255.0 * (1 - alpha)
    d = np.abs(over_white(a) - over_white(b)).max(axis=-1)
    rows.append((name, float(d.mean()), float((d > 64).mean() * 100), parts[2:6]))
rows.sort(key=lambda r: -r[1])
means = np.array([r[1] for r in rows])
print(f'{len(rows)} compared, {len(errors)} not imported')
print(f'mean difference per file (0-255): median {np.median(means):.3f}, 90th percentile {np.percentile(means, 90):.3f}, max {means.max():.3f}')
print(f'files with mean difference above 1.0: {(means > 1.0).sum()}; above 3.0: {(means > 3.0).sum()}')
print('worst:')
for name, mean, over, info in rows[:12]:
    print(f'  {name:42} mean {mean:6.3f}  pixels off by >64: {over:5.2f}%  {info}')
for name, text in errors[:10]:
    print('ERROR', name, text[:100])
