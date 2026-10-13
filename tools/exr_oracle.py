"""Compare PhotoForge's OpenEXR import and export against the OpenEXR reference implementation.

For every sample the Rust test (tests/exr_real_files.rs) imported and exported, this reads the same
file with the reference reader (OpenEXR 3.5.2 python bindings) and checks:

  * the canvas, the layer size and its position against the display and data windows;
  * the imported pixels against the reference samples taken through the same convention
    (premultiplied -> straight, chromaticities -> sRGB primaries by an independent implementation);
  * the exported single-precision file, read by the reference reader, against the composite of
    what was imported (premultiplied), and its header (chromaticities, data window, compression);
  * the exported half-precision file, to half-float tolerance.
"""
import os
import sys
sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
from pfenv import CARGO_SH, CRATE_PATH, REPO_PATH, WORK_PATH  # noqa: E402
import json
import os
import sys

import numpy as np
import OpenEXR

SAMPLES = str(WORK_PATH / 'exr-samples')
DUMP = str(WORK_PATH / 'exr-dump')

REC709 = dict(r=(0.64, 0.33), g=(0.30, 0.60), b=(0.15, 0.06), w=(0.3127, 0.3290))


def xyz_matrix(p):
    # Primaries as (x, y, z) column vectors, scaled so that RGB (1,1,1) is the white point (Y=1).
    cols = np.array([[p[k][0], p[k][1], 1 - p[k][0] - p[k][1]] for k in 'rgb'], dtype=np.float64).T
    wx, wy = p['w']
    white = np.array([wx / wy, 1.0, (1 - wx - wy) / wy])
    s = np.linalg.solve(cols, white)
    return cols * s


def bradford(src_white, dst_white):
    m = np.array([[0.8951, 0.2664, -0.1614], [-0.7502, 1.7135, 0.0367], [0.0389, -0.0685, 1.0296]])
    def xyz(w):
        return np.array([w[0] / w[1], 1.0, (1 - w[0] - w[1]) / w[1]])
    s = m @ xyz(src_white)
    d = m @ xyz(dst_white)
    return np.linalg.inv(m) @ np.diag(d / s) @ m


def to_srgb_matrix(chroma):
    if chroma is None:
        return None
    c = [float(v) for v in chroma]
    p = dict(r=(c[0], c[1]), g=(c[2], c[3]), b=(c[4], c[5]), w=(c[6], c[7]))
    same = all(abs(p[k][i] - REC709[k][i]) <= 1.5e-3 for k in p for i in (0, 1))
    if same:
        return None
    return np.linalg.inv(xyz_matrix(REC709)) @ bradford(p['w'], REC709['w']) @ xyz_matrix(p)


def expected_picture(part):
    """Straight-alpha, finite, converted RGBA float32 of the part's main picture, or None."""
    channels = part.channels
    names = list(channels)
    arr = None
    alpha = None
    if 'RGBA' in channels:
        a = channels['RGBA'].pixels.astype(np.float32)
        rgb, alpha = a[..., :3], a[..., 3]
    elif 'RGB' in channels:
        rgb = channels['RGB'].pixels.astype(np.float32)
    elif all(k in channels for k in ('R', 'G', 'B')):
        rgb = np.stack([channels[k].pixels.astype(np.float32) for k in 'RGB'], axis=-1)
        if 'A' in channels:
            alpha = channels['A'].pixels.astype(np.float32)
    elif 'Y' in channels:
        y = channels['Y'].pixels.astype(np.float32)
        rgb = np.stack([y, y, y], axis=-1)
        if 'A' in channels:
            alpha = channels['A'].pixels.astype(np.float32)
    else:
        return None, names
    h, w = rgb.shape[:2]
    rgb = np.nan_to_num(rgb, nan=0.0, posinf=65504.0, neginf=-65504.0)
    # An alpha channel that is zero everywhere over colour that is there was never filled.
    if alpha is not None and not np.any(alpha > 0) and np.any(rgb != 0):
        alpha = None
    if alpha is None:
        a = np.ones((h, w), dtype=np.float32)
        straight = rgb
    else:
        alpha = np.nan_to_num(alpha, nan=0.0, posinf=65504.0, neginf=-65504.0)
        a_clamped = np.clip(alpha, 0.0, 1.0)
        with np.errstate(divide='ignore', invalid='ignore'):
            straight = np.where((alpha > 0)[..., None], rgb / np.where(alpha > 0, alpha, 1)[..., None], 0.0)
        a = a_clamped
    return np.concatenate([straight, a[..., None]], axis=-1).astype(np.float32), names


def window(header, key):
    v = np.array(header[key]).reshape(-1)
    return int(v[0]), int(v[1]), int(v[2]), int(v[3])


def main():
    names = sorted(n[:-5] for n in os.listdir(DUMP) if n.endswith('.json'))
    worst = {}
    problems = []
    checked = 0
    refused = []
    for stem in names:
        account = json.load(open(os.path.join(DUMP, stem + '.json')))
        src = os.path.join(SAMPLES, stem + '.exr')
        if account['status'] != 'ok':
            refused.append((stem, account['message']))
            continue
        ref = OpenEXR.File(src)
        part = ref.parts[0]
        header = part.header
        dx0, dy0, dx1, dy1 = window(header, 'dataWindow')
        wx0, wy0, wx1, wy1 = window(header, 'displayWindow')
        data_w, data_h = dx1 - dx0 + 1, dy1 - dy0 + 1
        canvas = (wx1 - wx0 + 1, wy1 - wy0 + 1)
        if tuple(account['canvas']) != canvas:
            problems.append((stem, 'canvas', account['canvas'], canvas))
            continue
        layer0 = account['layers'][0]
        if (layer0['width'], layer0['height']) != (data_w, data_h):
            problems.append((stem, 'layer size', layer0, (data_w, data_h)))
        if (layer0['translateX'], layer0['translateY']) != (dx0 - wx0, dy0 - wy0):
            problems.append((stem, 'layer position', layer0, (dx0 - wx0, dy0 - wy0)))

        expected, channel_names = expected_picture(part)
        if expected is None:
            problems.append((stem, 'no expected picture', channel_names))
            continue
        chroma = header.get('chromaticities')
        matrix = to_srgb_matrix(chroma) if chroma is not None else None
        if matrix is not None:
            e = expected.copy()
            e[..., :3] = np.einsum('ij,hwj->hwi', matrix, expected[..., :3])
            expected = e
        got = np.fromfile(os.path.join(DUMP, f'{stem}.0.f32'), dtype='<f4').reshape(data_h, data_w, 4)
        scale = np.maximum(np.abs(expected), 1.0)
        diff = float(np.max(np.abs(got - expected) / scale))
        worst[stem] = diff
        if diff > 2e-5:
            problems.append((stem, 'import differs from the reference', diff))

        # The exported file, read by the reference reader.
        for suffix, tolerance in (('out', 2e-5), ('out16', 2e-3)):
            out = OpenEXR.File(os.path.join(DUMP, f'{stem}.{suffix}.exr'))
            out_part = out.parts[0]
            h = out_part.header
            ow = window(h, 'dataWindow')
            if (ow[2] - ow[0] + 1, ow[3] - ow[1] + 1) != canvas:
                problems.append((stem, f'{suffix} data window', ow, canvas))
            c = h.get('chromaticities')
            if c is None or to_srgb_matrix(c) is not None:
                problems.append((stem, f'{suffix} chromaticities', c))
            if str(h['compression']).split('.')[-1] != 'ZIP_COMPRESSION':
                problems.append((stem, f'{suffix} compression', h['compression']))
            # Expected composite: the first (visible) layer on the canvas, premultiplied.
            canvas_buf = np.zeros((canvas[1], canvas[0], 4), dtype=np.float32)
            tx, ty = int(layer0['translateX']), int(layer0['translateY'])
            x0, y0 = max(tx, 0), max(ty, 0)
            x1, y1 = min(tx + data_w, canvas[0]), min(ty + data_h, canvas[1])
            if x1 > x0 and y1 > y0:
                canvas_buf[y0:y1, x0:x1] = got[y0 - ty:y1 - ty, x0 - tx:x1 - tx]
            pre = canvas_buf.copy()
            pre[..., :3] *= pre[..., 3:4]
            exported, _ = None, None
            oc = out_part.channels
            if 'RGBA' in oc:
                arr = oc['RGBA'].pixels.astype(np.float32)
            elif 'RGB' in oc:
                rgb = oc['RGB'].pixels.astype(np.float32)
                arr = np.concatenate([rgb, np.ones(rgb.shape[:2] + (1,), np.float32)], axis=-1)
            else:
                problems.append((stem, f'{suffix} channels', list(oc)))
                continue
            if suffix == 'out16':
                pre_cmp = np.clip(pre, -65504.0, 65504.0)
            else:
                pre_cmp = pre
            scale = np.maximum(np.abs(pre_cmp), 1.0)
            d = float(np.max(np.abs(arr - pre_cmp) / scale))
            worst[f'{stem}:{suffix}'] = d
            if d > tolerance:
                problems.append((stem, f'{suffix} differs from the composite', d))
            # Opaque pictures are written without an alpha channel.
            opaque = bool(np.all(canvas_buf[..., 3] == 1.0))
            if opaque != ('RGBA' not in oc):
                problems.append((stem, f'{suffix} alpha channel presence', opaque, list(oc)))
        checked += 1

    print(f'{len(names)} accounts, {checked} files compared, {len(refused)} refused')
    for stem, message in refused:
        print('  refused', stem, '->', message[:150])
    imp = sorted(((v, k) for k, v in worst.items() if ':' not in k), reverse=True)[:5]
    print('largest import differences (relative to max(|v|,1)):', [(k, f'{v:.2e}') for v, k in imp])
    exp = sorted(((v, k) for k, v in worst.items() if ':out' in k and ':out16' not in k), reverse=True)[:5]
    print('largest f32 export differences:', [(k, f'{v:.2e}') for v, k in exp])
    exp16 = sorted(((v, k) for k, v in worst.items() if ':out16' in k), reverse=True)[:5]
    print('largest f16 export differences:', [(k, f'{v:.2e}') for v, k in exp16])
    print('problems:', len(problems))
    for p in problems:
        print('  ', p)
    return 1 if problems else 0


if __name__ == '__main__':
    sys.exit(main())
