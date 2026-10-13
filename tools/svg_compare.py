"""Render the exported SVGs with resvg (an independent renderer) and compare them with
PhotoForge's own render of the same documents. Read-only."""
import os
import sys
sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
from pfenv import CARGO_SH, CRATE_PATH, REPO_PATH, WORK_PATH  # noqa: E402
import glob
import io
import os
import sys

import numpy as np
import resvg_py
from PIL import Image

sys.stdout.reconfigure(encoding='utf-8')
d = str(WORK_PATH / 'out' / 'svg-cases')
for svg in sorted(glob.glob(os.path.join(d, '*.svg'))):
    name = os.path.basename(svg)[:-4]
    png = svg[:-4] + '.png'
    theirs = np.asarray(Image.open(io.BytesIO(bytes(resvg_py.svg_to_bytes(svg_path=svg)))).convert('RGBA'), dtype=np.int16)
    ours = np.asarray(Image.open(png).convert('RGBA'), dtype=np.int16)
    if theirs.shape != ours.shape:
        print(name, 'SIZE', theirs.shape, ours.shape)
        continue
    diff = np.abs(theirs - ours)
    # Premultiplied difference, so transparent pixels with different colour do not count.
    a1 = theirs[..., 3:4] / 255.0
    a2 = ours[..., 3:4] / 255.0
    pre = np.abs(theirs[..., :3] * a1 - ours[..., :3] * a2)
    worst = pre.max()
    big = (pre.max(axis=-1) > 24).mean() * 100
    mean = pre.mean()
    alpha = np.abs(theirs[..., 3] - ours[..., 3])
    print(f'{name:18} premultiplied mean {mean:5.2f}  max {worst:6.1f}  pixels off by more than 24: {big:5.2f}%  alpha max {alpha.max()}')
