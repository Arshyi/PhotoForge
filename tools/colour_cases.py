"""For the pixel layers psd_tools_check.py calls far (a change of depth or of colour space on
import), bring original and export to the same display encoding and compare again.

  * a 32-bit original holds linear light; a 16-bit export holds sRGB-encoded values: encode the
    original.
  * a 32-bit export holds linear light; an 8-bit original with a wide-gamut profile: convert the
    original to sRGB with LittleCMS (Pillow) and encode the export.
"""
import os
import sys
sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
from pfenv import CARGO_SH, CRATE_PATH, REPO_PATH, WORK_PATH  # noqa: E402
import io
import sys

import numpy as np
from PIL import ImageCms
from psd_tools import PSDImage

sys.stdout.reconfigure(encoding='utf-8')


def enc(v):
    v = np.clip(np.asarray(v, dtype=np.float64), 0.0, 1.0)
    return np.where(v <= 0.0031308, v * 12.92, 1.055 * np.power(v, 1 / 2.4) - 0.055)


def rgba(a):
    a = np.asarray(a, dtype=np.float64)
    if a.shape[-1] == 3:
        a = np.concatenate([a, np.ones_like(a[..., :1])], axis=-1)
    return a


def main():
    index = {}
    for line in open(str(WORK_PATH / 'out' / 'all2' / 'index.txt'), encoding='utf8'):
        n, p = line.rstrip('\n').split('\t')
        index[n] = p
    rows = []
    for n, path in index.items():
        try:
            before = PSDImage.open(path)
            after = PSDImage.open(str(WORK_PATH / 'out' / 'all2' / f'{n}.psd'))
        except Exception:  # noqa: BLE001
            continue
        icc = before.image_resources.get_data(1039)
        wide = bool(icc) and b'sRGB' not in icc[:200] and before.depth != 32
        if before.depth == after.depth and not wide:
            continue
        mine = [l for l in after.descendants() if l.kind == 'pixel']
        theirs = [l for l in before.descendants() if l.kind == 'pixel']
        for a_layer, b_layer in zip(mine, theirs):
            if a_layer.name != b_layer.name:
                continue
            a = rgba(a_layer.numpy())
            b = rgba(b_layer.numpy())
            if a.shape != b.shape:
                continue
            visible = (a[..., 3] > 0) | (b[..., 3] > 0)
            if not visible.any():
                continue
            if after.depth == 32:
                shown = enc(a[..., :3])
            else:
                shown = a[..., :3]
            if before.depth == 32:
                original = enc(b[..., :3])
                how = 'linear 32-bit encoded'
            elif wide:
                # topil() already converts through the document's profile (apply_icc defaults on)
                original = np.asarray(b_layer.topil().convert('RGB'), dtype=np.float64) / 255.0
                how = 'psd-tools topil() (ICC applied)'
            else:
                original = b[..., :3]
                how = 'as is'
            d = np.abs(shown - original)[visible]
            rows.append((n, a_layer.name, before.depth, after.depth, how, float(d.max()) * 255, float(d.mean()) * 255))
    for n, name, src, dst, how, worst, mean in rows:
        print(f'{n} {name!r:24} {src}->{dst} bit  {how:30} max {worst:6.2f}  mean {mean:5.2f} levels')
    print(len(rows), 'layers')


main()
