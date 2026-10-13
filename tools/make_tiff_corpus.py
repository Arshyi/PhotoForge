"""A corpus of TIFFs written by tifffile (a TIFF writer that is not PhotoForge) with, for each, the
RGBA values PhotoForge must read from it, computed here with numpy from the array that was written.

Usage: python -I make_tiff_corpus.py OUT_DIR

Writes OUT_DIR/manifest.json and, per case, NAME.tif and NAME.expect.f32 (RGBA, float32 little
endian, in the document's space: 0..1 sRGB-encoded for 8-bit documents, linear light for
high-precision ones).
"""
import os
import sys
sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
from pfenv import CARGO_SH, CRATE_PATH, REPO_PATH, WORK_PATH  # noqa: E402
import itertools
import json
import os
import struct
import sys
import zlib

import numpy as np
import tifffile

out_dir = sys.argv[1]
os.makedirs(out_dir, exist_ok=True)
cases = []


def srgb_decode(c):
    c = np.asarray(c, dtype=np.float64)
    return np.where(c <= 0.04045, c / 12.92, ((c + 0.055) / 1.055) ** 2.4)


def picture(h, w, channels, dtype, seed=1):
    rng = np.random.default_rng(seed)
    y, x = np.mgrid[0:h, 0:w]
    base = np.stack([(x * 255 / max(w - 1, 1)), (y * 255 / max(h - 1, 1)),
                     ((x + y) * 255 / max(w + h - 2, 1))], axis=-1)
    base = np.concatenate([base, 255 - base[..., :1]], axis=-1)[..., :max(channels, 1)]
    noise = rng.integers(0, 24, size=base.shape)
    arr = np.clip(base + noise, 0, 255)
    if dtype == np.uint8:
        return arr.astype(np.uint8)
    if dtype == np.uint16:
        return (arr * 257).astype(np.uint16)
    if dtype == np.uint32:
        return (arr * 16843009).astype(np.uint32)
    if dtype in (np.float32, np.float64, np.float16):
        # Includes values above 1 and below 0, as scene-referred floats have.
        f = arr / 255.0
        f[..., 0] *= 2.5
        f[0, 0, :] = -0.25
        return f.astype(dtype)
    raise ValueError(dtype)


def orient(arr, orientation):
    t = arr.transpose(1, 0, 2) if arr.ndim == 3 else arr.T
    return {
        1: arr,
        2: arr[:, ::-1],
        3: arr[::-1, ::-1],
        4: arr[::-1, :],
        5: t,
        6: t[:, ::-1],
        7: t[::-1, ::-1],
        8: t[::-1, :],
    }[orientation]


def to_rgba_expected(arr, photometric, alpha_kind, dtype, palette=None):
    """arr: H x W x C (C=1 grey / palette index, 3 RGB, +1 alpha) in file dtype. Returns the float
    RGBA the document must hold, and whether that document is high precision."""
    a = arr.astype(np.float64)
    if photometric == 'palette':
        idx = arr[..., 0].astype(int)
        rgb = palette[idx] / 65535.0
        rgba = np.concatenate([rgb, np.ones(rgb.shape[:2] + (1,))], axis=-1)
        return rgba, False
    if np.issubdtype(dtype, np.integer):
        full = float(np.iinfo(dtype).max)
        a = a / full
    c = a.shape[-1]
    if photometric in ('minisblack', 'miniswhite'):
        grey = a[..., :1]
        if photometric == 'miniswhite':
            grey = 1.0 - grey
        colour = np.repeat(grey, 3, axis=-1)
        alpha = a[..., 1:2] if c > 1 else np.ones_like(grey)
    else:
        colour = a[..., :3]
        alpha = a[..., 3:4] if c > 3 else np.ones(a.shape[:2] + (1,))
    if c > (1 if photometric in ('minisblack', 'miniswhite') else 3) and alpha_kind == 'associated':
        safe = np.where(alpha > 0, alpha, 1.0)
        colour = np.where(alpha > 0, colour / safe, 0.0)
    if c > (1 if photometric in ('minisblack', 'miniswhite') else 3) and alpha_kind == 'unspecified':
        alpha = np.ones_like(alpha)
    rgba = np.concatenate([colour, alpha], axis=-1)
    return rgba, None


def add(name, data, expect_doc, orientation=1, **kw):
    path = os.path.join(out_dir, name + '.tif')
    tifffile.imwrite(path, data, **kw)
    expected = orient(expect_doc, orientation).astype(np.float32)
    with open(os.path.join(out_dir, name + '.expect.f32'), 'wb') as f:
        f.write(np.ascontiguousarray(expected, dtype='<f4').tobytes())
    return path, expected.shape


def record(name, path, shape, high, tolerance, note=''):
    cases.append({'name': name, 'file': os.path.basename(path), 'height': int(shape[0]),
                  'width': int(shape[1]), 'high': bool(high), 'tolerance': tolerance, 'note': note})


# ---- integer RGB / RGBA across compression, predictor, layout, tiling, byte order ------------
compressions = [None, 'packbits', 'lzw', 'adobe_deflate']
for dtype, comp, planar, tile, bo in itertools.product(
        [np.uint8, np.uint16], compressions, ['contig', 'separate'], [None, (16, 16)], ['<', '>']):
    # Keep the matrix to a size that runs in seconds: every combination of the first two axes
    # with the others sampled.
    key = (np.dtype(dtype).itemsize, str(comp), planar, tile, bo)
    if zlib.crc32(repr(key).encode()) % 3:
        continue
    h, w = 29, 37
    data = picture(h, w, 3, dtype)
    kw = dict(photometric='rgb', compression=comp, planarconfig=planar, byteorder=bo)
    if tile:
        kw['tile'] = tile
    else:
        kw['rowsperstrip'] = 7
    if comp in ('lzw', 'adobe_deflate'):
        kw['predictor'] = 'horizontal'
    name = 'rgb_%s_%s_%s_%s_%s' % (np.dtype(dtype).name, comp or 'none', planar, 'tile' if tile else 'strip', 'be' if bo == '>' else 'le')
    rgba, _ = to_rgba_expected(data, 'rgb', 'none', dtype)
    high = dtype != np.uint8
    exp = srgb_decode(rgba[..., :3]) if high else rgba[..., :3]
    exp = np.concatenate([exp, rgba[..., 3:4]], axis=-1)
    written = np.moveaxis(data, -1, 0) if planar == 'separate' else data
    path, shape = add(name, written, exp, **kw)
    record(name, path, shape, high, 1e-6 if high else 0.0)

# ---- alpha: associated, unassociated, unspecified --------------------------------------------
for dtype, kind in itertools.product([np.uint8, np.uint16], ['associated', 'unassociated', 'unspecified']):
    h, w = 21, 33
    data = picture(h, w, 4, dtype, seed=5)
    if kind == 'associated':
        # Make the colour a valid premultiplication.
        full = float(np.iinfo(dtype).max)
        a = data[..., 3:4].astype(np.float64) / full
        data[..., :3] = np.round(data[..., :3].astype(np.float64) * a).astype(dtype)
        data[0, 0, 3] = 0
        data[0, 0, :3] = 0
    extra = {'associated': 'assocalpha', 'unassociated': 'unassalpha', 'unspecified': 'unspecified'}[kind]
    name = 'rgba_%s_%s' % (np.dtype(dtype).name, kind)
    rgba, _ = to_rgba_expected(data, 'rgb', {'associated': 'associated', 'unassociated': 'none', 'unspecified': 'unspecified'}[kind], dtype)
    high = dtype != np.uint8
    exp = np.concatenate([srgb_decode(rgba[..., :3]) if high else rgba[..., :3], rgba[..., 3:4]], axis=-1)
    path, shape = add(name, data, exp, photometric='rgb', extrasamples=[
        {'associated': 1, 'unassociated': 2, 'unspecified': 0}[kind]], compression='lzw', predictor='horizontal')
    # 8-bit associated alpha rounds as the reader does; allow one level.
    record(name, path, shape, high, (1.5 / 255.0) if (dtype == np.uint8 and kind == 'associated') else (1e-6 if high else 0.0))

# ---- grey, white-is-zero, bilevel, palette ----------------------------------------------------
for dtype, photometric in itertools.product([np.uint8, np.uint16], ['minisblack', 'miniswhite']):
    data = picture(25, 40, 1, dtype, seed=3)
    name = 'grey_%s_%s' % (np.dtype(dtype).name, photometric)
    rgba, _ = to_rgba_expected(data, photometric, 'none', dtype)
    high = dtype != np.uint8
    exp = np.concatenate([srgb_decode(rgba[..., :3]) if high else rgba[..., :3], rgba[..., 3:4]], axis=-1)
    path, shape = add(name, data[..., 0], exp[..., :], photometric=photometric, compression='adobe_deflate', predictor='horizontal', rowsperstrip=5)
    record(name, path, shape, high, 1e-6 if high else 0.0)

rng = np.random.default_rng(9)
bilevel = rng.integers(0, 2, size=(23, 50)).astype(bool)
for comp in [None, 'packbits', 'adobe_deflate']:
    name = 'bilevel_%s' % (comp or 'none')
    grey = bilevel.astype(np.float64)
    exp = np.stack([grey, grey, grey, np.ones_like(grey)], axis=-1)
    path, shape = add(name, bilevel, exp, photometric='minisblack', compression=comp)
    record(name, path, shape, False, 0.0)

palette = (np.random.default_rng(4).integers(0, 65535, size=(256, 3))).astype(np.uint16)
idx = np.random.default_rng(6).integers(0, 256, size=(19, 31)).astype(np.uint8)
exp, _ = to_rgba_expected(idx[..., None], 'palette', 'none', np.uint8, palette=palette)
path, shape = add('palette_8bit', idx, exp, photometric='palette', colormap=palette.T, compression='lzw')
record('palette_8bit', path, shape, False, 1.0 / 255.0 + 1e-6)

# ---- floating point, with and without the predictor -------------------------------------------
for dtype, comp, pred, bo in itertools.product([np.float32, np.float16, np.float64], [None, 'adobe_deflate'], [False, True], ['<', '>']):
    if pred and comp is None:
        continue
    if dtype == np.float64 and bo == '>':
        continue
    data = picture(17, 26, 3, dtype, seed=7)
    name = 'float_%s_%s_%s_%s' % (np.dtype(dtype).name, comp or 'none', 'pred' if pred else 'nopred', 'be' if bo == '>' else 'le')
    kw = dict(photometric='rgb', compression=comp, byteorder=bo)
    if pred:
        kw['predictor'] = 'floatingpoint'
    rgba, _ = to_rgba_expected(data, 'rgb', 'none', dtype)
    path, shape = add(name, data, rgba, **kw)
    record(name, path, shape, True, 1e-6 if dtype != np.float16 else 1e-3)

# ---- orientation ------------------------------------------------------------------------------
for orientation in range(1, 9):
    data = picture(11, 17, 3, np.uint8, seed=2)
    name = 'orientation_%d' % orientation
    rgba, _ = to_rgba_expected(data, 'rgb', 'none', np.uint8)
    path, shape = add(name, data, rgba, orientation=orientation, photometric='rgb',
                      extratags=[(274, 'H', 1, orientation, True)])
    record(name, path, shape, False, 0.0)

# ---- metadata and an embedded profile ---------------------------------------------------------
xmp = (b'<x:xmpmeta xmlns:x="adobe:ns:meta/"><rdf:RDF xmlns:rdf="http://www.w3.org/1999/02/22-rdf-syntax-ns#">'
       b'<rdf:Description xmlns:dc="http://purl.org/dc/elements/1.1/"><dc:title>Corpus picture</dc:title>'
       b'</rdf:Description></rdf:RDF></x:xmpmeta>')
iptc = bytes([0x1C, 2, 120, 0, 6]) + b'corpus'
data = picture(15, 20, 3, np.uint8, seed=8)
rgba, _ = to_rgba_expected(data, 'rgb', 'none', np.uint8)
path, shape = add('metadata', data, rgba, photometric='rgb', extratags=[
    (270, 's', 0, 'A description', True), (271, 's', 0, 'Acme', True), (272, 's', 0, 'Cam-One', True),
    (305, 's', 0, 'Corpus 1.0', True), (315, 's', 0, 'A. Photographer', True),
    (700, 'B', len(xmp), xmp, True), (33723, 'B', len(iptc), iptc, True)])
record('metadata', path, shape, False, 0.0, note='metadata')

# ---- more than one page (a thumbnail after the picture) ---------------------------------------
data = picture(40, 60, 3, np.uint8, seed=10)
small = picture(10, 15, 3, np.uint8, seed=11)
path = os.path.join(out_dir, 'pages.tif')
with tifffile.TiffWriter(path) as tw:
    tw.write(data, photometric='rgb')
    tw.write(small, photometric='rgb', subfiletype=1)
rgba, _ = to_rgba_expected(data, 'rgb', 'none', np.uint8)
with open(os.path.join(out_dir, 'pages.expect.f32'), 'wb') as f:
    f.write(np.ascontiguousarray(rgba, dtype='<f4').tobytes())
record('pages', path, rgba.shape, False, 0.0, note='pages')

# ---- files written by libtiff through Pillow (a second writer, and a different codebase) ------
from PIL import Image


def pillow_case(name, image, expect_rgba, high=False, tolerance=0.0, **save_kw):
    path = os.path.join(out_dir, name + '.tif')
    image.save(path, **save_kw)
    with open(os.path.join(out_dir, name + '.expect.f32'), 'wb') as f:
        f.write(np.ascontiguousarray(expect_rgba.astype(np.float32), dtype='<f4').tobytes())
    record(name, path, expect_rgba.shape, high, tolerance)


rgb = picture(31, 43, 3, np.uint8, seed=21)
rgb_expect = np.concatenate([rgb / 255.0, np.ones(rgb.shape[:2] + (1,))], axis=-1)
for comp in ['raw', 'tiff_lzw', 'tiff_adobe_deflate', 'packbits']:
    pillow_case('pillow_rgb_%s' % comp, Image.fromarray(rgb, 'RGB'), rgb_expect, compression=comp)

rgba = picture(31, 43, 4, np.uint8, seed=22)
rgba_expect = rgba / 255.0
pillow_case('pillow_rgba_lzw', Image.fromarray(rgba, 'RGBA'), rgba_expect, compression='tiff_lzw')

grey = picture(37, 25, 1, np.uint8, seed=23)[..., 0]
grey_expect = np.repeat((grey / 255.0)[..., None], 3, axis=-1)
grey_expect = np.concatenate([grey_expect, np.ones(grey.shape + (1,))], axis=-1)
pillow_case('pillow_grey_lzw', Image.fromarray(grey, 'L'), grey_expect, compression='tiff_lzw')

# 16-bit grey (Pillow's I;16): high precision, read as sRGB.
g16 = (picture(29, 17, 1, np.uint16, seed=24)[..., 0]).astype(np.uint16)
g16_expect = np.repeat((g16 / 65535.0)[..., None], 3, axis=-1)
g16_expect = np.concatenate([srgb_decode(g16_expect), np.ones(g16.shape + (1,))], axis=-1)
pillow_case('pillow_grey16', Image.fromarray(g16, 'I;16'), g16_expect, high=True, tolerance=1e-6)

# A paletted picture.
pal = Image.fromarray(np.random.default_rng(31).integers(0, 16, size=(20, 30)).astype(np.uint8), 'P')
pal.putpalette([c for i in range(16) for c in (i * 16, 255 - i * 16, (i * 37) % 256)] + [0] * (768 - 48))
pal_rgb = np.asarray(pal.convert('RGB')).astype(np.float64) / 255.0
pal_expect = np.concatenate([pal_rgb, np.ones(pal_rgb.shape[:2] + (1,))], axis=-1)
pillow_case('pillow_palette', pal, pal_expect, tolerance=1.0 / 255.0 + 1e-6)

# What is refused, by name: the manifest says which word the refusal must contain.
refusals = []
jpeg = Image.fromarray(rgb, 'RGB')
jpeg_path = os.path.join(out_dir, 'pillow_jpeg.tif')
jpeg.save(jpeg_path, compression='jpeg')
refusals.append({'name': 'pillow_jpeg', 'file': 'pillow_jpeg.tif', 'refused': 'JPEG'})
cmyk_path = os.path.join(out_dir, 'pillow_cmyk.tif')
Image.fromarray(picture(9, 9, 4, np.uint8, seed=25), 'CMYK').save(cmyk_path)
refusals.append({'name': 'pillow_cmyk', 'file': 'pillow_cmyk.tif', 'refused': 'CMYK'})
cases.extend(refusals)

with open(os.path.join(out_dir, 'manifest.json'), 'w') as f:
    json.dump(cases, f, indent=1)
print(len(cases), 'cases')
