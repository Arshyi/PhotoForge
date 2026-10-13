"""Checks the corpus generator's expectations against two independent readers (tifffile and
Pillow), so the expectations the Rust tests are held to are themselves not a guess."""
import os
import sys
sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
from pfenv import CARGO_SH, CRATE_PATH, REPO_PATH, WORK_PATH  # noqa: E402
import json
import os
import sys

import numpy as np
import tifffile
from PIL import Image, ImageOps

d = sys.argv[1]
manifest = json.load(open(os.path.join(d, 'manifest.json')))
bad = []
for case in manifest:
    if 'refused' in case:
        continue
    path = os.path.join(d, case['file'])
    expected = np.fromfile(os.path.join(d, case['name'] + '.expect.f32'), dtype='<f4').reshape(case['height'], case['width'], 4)
    if case['name'].startswith('orientation_'):
        with Image.open(path) as im:
            upright = np.asarray(ImageOps.exif_transpose(im.convert('RGB')))
        if upright.shape[:2] != expected.shape[:2] or not np.array_equal(upright, np.round(expected[..., :3] * 255).astype(np.uint8)):
            bad.append((case['name'], 'orientation differs from Pillow'))
        continue
    arr = tifffile.imread(path)
    if arr.ndim == 2:
        arr = arr[..., None]
    if arr.ndim == 3 and arr.shape[0] in (3, 4) and arr.shape[-1] not in (3, 4, 1):
        arr = np.moveaxis(arr, 0, -1)
    # The reader's own reading of the same file, for the plain 8-bit RGB cases.
    if case['name'].startswith('rgb_uint8') and not case['high']:
        got = arr[..., :3].astype(np.float64) / 255.0
        if not np.allclose(got, expected[..., :3], atol=1e-6):
            bad.append((case['name'], 'tifffile differs from the expectation'))
print('checked', len(manifest), 'cases;', 'problems:', bad or 'none')
sys.exit(1 if bad else 0)
