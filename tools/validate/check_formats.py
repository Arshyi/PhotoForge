"""Reads what the packaged window wrote, with programs that are not PhotoForge."""
import io
import os
import sys
import xml.dom.minidom

import numpy as np
import OpenEXR
import resvg_py
from PIL import Image
from psd_tools import PSDImage

sys.path.insert(0, os.path.join(os.path.dirname(os.path.abspath(__file__)), '..'))
from pfenv import WORK_PATH  # noqa: E402

OUT = str(WORK_PATH / 'validate' / 'out' / 'formats')
EXR = str(WORK_PATH / 'exr-samples')
SVG = str(WORK_PATH / 'svg-samples')
PSD = str(WORK_PATH / 'psd-samples')


def picture(path):
    """RGB(A) float32 of an EXR's first part, by the reference reader."""
    part = OpenEXR.File(path).parts[0]
    channels = part.channels
    if 'RGBA' in channels:
        a = channels['RGBA'].pixels.astype(np.float32)
        return a[..., :3], a[..., 3]
    if 'RGB' in channels:
        return channels['RGB'].pixels.astype(np.float32), None
    rgb = np.stack([channels[k].pixels.astype(np.float32) for k in 'RGB'], axis=-1)
    alpha = channels['A'].pixels.astype(np.float32) if 'A' in channels else None
    return rgb, alpha


print('== EXR: exported file vs the source it was imported from (reference reader)')
for stem in ('TestImages__SquaresSwirls', 'TestImages__WideColorGamut', 'ScanLines__Desk'):
    src_rgb, src_a = picture(os.path.join(EXR, stem + '.exr'))
    out = os.path.join(OUT, stem + '.exr')
    f = OpenEXR.File(out)
    header = f.parts[0].header
    out_rgb, out_a = picture(out)
    same_shape = src_rgb.shape == out_rgb.shape
    diff = float(np.max(np.abs(np.nan_to_num(src_rgb) - np.nan_to_num(out_rgb)))) if same_shape else float('nan')
    scale = float(np.max(np.abs(np.nan_to_num(src_rgb)))) or 1.0
    comp = header.get('compression')
    print(f'  {stem}: shape {src_rgb.shape} -> {out_rgb.shape}; max abs diff {diff:.6g} (peak {scale:.4g}); '
          f'compression {comp}; chromaticities {tuple(np.round(np.array(header.get("chromaticities", [])), 4))}')
    png = Image.open(os.path.join(OUT, stem + '.png'))
    print(f'    png export: {png.size} {png.mode}')

print('== SVG: well-formed, and renders like the source (resvg)')
for stem in ('bootstrap-airplane-fill', 'bootstrap-apple-music', 'bootstrap-8-circle-fill'):
    out = os.path.join(OUT, stem + '.svg')
    xml.dom.minidom.parse(out)

    def render(path):
        data = bytes(resvg_py.svg_to_bytes(svg_path=path, width=64, height=64))
        return np.asarray(Image.open(io.BytesIO(data)).convert('RGBA'), dtype=np.int16)

    ours, theirs = render(out), render(os.path.join(SVG, stem + '.svg'))
    diff = np.abs(ours - theirs)
    print(f'  {stem}: {os.path.getsize(out)} bytes; vs source at 64x64: max {int(diff.max())}, '
          f'pixels over 16 levels: {int((diff.max(axis=-1) > 16).sum())}')

print('== PSD: psd-tools opens each export and agrees with its source')
pairs = [
    ('bootstrap-airplane-fill.psd', None),
    ('bootstrap-apple-music.psd', None),
    ('bootstrap-8-circle-fill.psd', None),
    ('psd-2layers-out.psd', os.path.join(PSD, 'psd-tools', 'tests', 'psd_files', '2layers.psd')),
    ('psd-adjustment-fillers-out.psd', os.path.join(PSD, 'psd-tools', 'tests', 'psd_files', 'adjustment-fillers.psd')),
    ('psd-layer_effects-out.psd', os.path.join(PSD, 'psd-tools', 'tests', 'psd_files', 'layer_effects.psd')),
    ('psd-32bit-out.psd', os.path.join(PSD, 'psd-tools', 'tests', 'psd_files', '32bit.psd')),
]
for name, source in pairs:
    exported = PSDImage.open(os.path.join(OUT, name))
    layers = [layer.name for layer in exported]
    line = f'  {name}: {exported.width}x{exported.height} depth {exported.depth} layers {len(layers)} {layers[:4]}'
    if source:
        original = PSDImage.open(source)
        line += f' | source {original.width}x{original.height} depth {original.depth} layers {len(list(original))}'
        try:
            a = np.asarray(exported.composite().convert('RGBA'), dtype=np.int16)
            b = np.asarray(original.composite().convert('RGBA'), dtype=np.int16)
            if a.shape == b.shape:
                line += f' | composite max diff {int(np.abs(a - b).max())}'
        except Exception as error:  # noqa: BLE001
            line += f' | composite compare failed: {error}'
    print(line)
