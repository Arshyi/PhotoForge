"""Checks PhotoForge's turned developments of a real photograph against Pillow's transposes of
the unturned one (the table Pillow's own ImageOps.exif_transpose uses), channel by channel in
float, so the rule is checked by a program that is not PhotoForge.

Usage: python -I upright_oracle.py DUMP_DIR
"""
import os
import sys
sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
from pfenv import CARGO_SH, CRATE_PATH, REPO_PATH, WORK_PATH  # noqa: E402
import json
import os
import sys

import numpy as np
from PIL import Image

d = sys.argv[1]
T = Image.Transpose
METHOD = {2: T.FLIP_LEFT_RIGHT, 3: T.ROTATE_180, 4: T.FLIP_TOP_BOTTOM, 5: T.TRANSPOSE,
          6: T.ROTATE_270, 7: T.TRANSVERSE, 8: T.ROTATE_90}


def load(name):
    meta = json.load(open(os.path.join(d, name + '.json')))
    raw = np.fromfile(os.path.join(d, name + '.f32'), dtype='<f4')
    return raw.reshape(meta['height'], meta['width'], 4)


base = load('orientation-1')
problems = []
for k, method in METHOD.items():
    got = load('orientation-%d' % k)
    want = np.stack([np.asarray(Image.fromarray(np.ascontiguousarray(base[..., c]), 'F').transpose(method))
                     for c in range(4)], axis=-1)
    if got.shape != want.shape:
        problems.append('orientation %d: shape %r, Pillow %r' % (k, got.shape, want.shape))
    elif not np.array_equal(got, want):
        problems.append('orientation %d: %d values differ' % (k, int((got != want).sum())))
    else:
        print('orientation %d: %dx%d identical to Pillow\'s transpose' % (k, got.shape[1], got.shape[0]))
print('problems:', problems or 'none')
sys.exit(1 if problems else 0)
