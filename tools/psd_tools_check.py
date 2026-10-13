"""Open every PhotoForge export with psd-tools (an independent, mature reader) and compare it with
the Photoshop-written original it came from. Read-only. Not part of PhotoForge.

usage: venv\\Scripts\\python.exe -I psd_tools_check.py INDEX_TXT EXPORT_DIR

For each export: does psd-tools open it; are its layers a subset of the original's, in the same
order, with the same name, kind, visibility, opacity, blend mode and clipping; are the pixels of
each matched pixel layer and its rectangle the same; and how close is the merged image the file
stores to the original's stored merged image.
"""
import os
import sys
sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
from pfenv import CARGO_SH, CRATE_PATH, REPO_PATH, WORK_PATH  # noqa: E402
import sys
import time
sys.stdout.reconfigure(encoding='utf-8')
import traceback

import numpy as np
from psd_tools import PSDImage


def describe(layer):
    kind = 'group' if layer.is_group() else layer.kind
    return dict(
        name=layer.name, kind=kind, visible=layer.visible,
        opacity=layer.opacity, blend=str(layer.blend_mode), clip=bool(layer.clipping),
        rect=(layer.left, layer.top, layer.right, layer.bottom), layer=layer,
    )


def flat(psd):
    return [describe(layer) for layer in psd.descendants()]


def rgba(array):
    """Any of grey / grey+alpha / RGB / RGBA as float RGBA (alpha 1 when the layer has none)."""
    if array is None:
        return None
    a = np.asarray(array, dtype=np.float32)
    if a.ndim == 2:
        a = a[..., None]
    c = a.shape[-1]
    if c == 1:
        a = np.concatenate([a, a, a, np.ones_like(a)], axis=-1)
    elif c == 2:
        a = np.concatenate([a[..., :1], a[..., :1], a[..., :1], a[..., 1:2]], axis=-1)
    elif c == 3:
        a = np.concatenate([a, np.ones_like(a[..., :1])], axis=-1)
    return a


def pixels(layer):
    return rgba(layer.numpy())


def main():
    index_path, export_dir = sys.argv[1], sys.argv[2]
    rows = []
    for line in open(index_path, encoding='utf8'):
        parts = line.rstrip('\n').split('\t')
        if len(parts) == 2 and not parts[1].startswith('FAILED'):
            rows.append((parts[0], parts[1]))
    totals = dict(files=0, opened=0, failed=0, subset=0, order=0, props=0, rects=0, pixel_layers=0,
                  pixel_same=0, pixel_close=0, pixel_far=0, pixel_shape=0, no_layers=0, merged_same=0, merged_close=0,
                  merged_far=0, merged_na=0)
    problems = []
    started = time.time()
    for number, original in rows:
        totals['files'] += 1
        exported = f'{export_dir}\\{number}.psd'
        try:
            after = PSDImage.open(exported)
            before = PSDImage.open(original)
            mine, theirs = flat(after), flat(before)
        except Exception as error:  # noqa: BLE001
            totals['failed'] += 1
            problems.append(f'{number} {original.rsplit(chr(92), 1)[-1]}: could not be opened: {type(error).__name__}: {error}')
            continue
        totals['opened'] += 1
        if not theirs:
            totals['no_layers'] += 1
            theirs = None
        # Pair each exported layer with the next unused original layer of the same name and kind,
        # scanning forward so that order is checked as well.
        if theirs is None:
            totals['subset'] += 1
            totals['order'] += 1
            totals['props'] += 1
            totals['rects'] += 1
            continue
        cursor, matched, ok_subset, ok_order = 0, [], True, True
        used = set()
        for layer in mine:
            found = None
            for j in range(len(theirs)):
                if j in used:
                    continue
                other = theirs[j]
                if other['name'] == layer['name'] and other['kind'] == layer['kind']:
                    found = j
                    break
            if found is None:
                ok_subset = False
                problems.append(f'{number}: exported layer {layer["name"]!r} ({layer["kind"]}) not in the original')
                continue
            if found < cursor:
                ok_order = False
            cursor = max(cursor, found)
            used.add(found)
            matched.append((layer, theirs[found]))
        totals['subset'] += ok_subset
        totals['order'] += ok_order
        props_ok = rects_ok = True
        for mine_layer, their_layer in matched:
            for key in ('visible', 'blend', 'clip'):
                if mine_layer[key] != their_layer[key]:
                    props_ok = False
                    problems.append(f'{number}: {mine_layer["name"]!r} {key}: {their_layer[key]} -> {mine_layer[key]}')
            if abs(mine_layer['opacity'] - their_layer['opacity']) > 1:
                props_ok = False
                problems.append(f'{number}: {mine_layer["name"]!r} opacity {their_layer["opacity"]} -> {mine_layer["opacity"]}')
            if mine_layer['kind'] == 'pixel':
                totals['pixel_layers'] += 1
                if mine_layer['rect'] != their_layer['rect']:
                    rects_ok = False
                    problems.append(f'{number}: {mine_layer["name"]!r} rect {their_layer["rect"]} -> {mine_layer["rect"]}')
                    continue
                try:
                    a, b = pixels(mine_layer['layer']), pixels(their_layer['layer'])
                except Exception as error:  # noqa: BLE001
                    problems.append(f'{number}: {mine_layer["name"]!r} pixels: {type(error).__name__}: {error}')
                    continue
                if a is None or b is None or a.shape != b.shape:
                    totals['pixel_shape'] += 1
                    problems.append(f'shape: {number}: {mine_layer["name"]!r} {None if a is None else a.shape} vs {None if b is None else b.shape}')
                    continue
                # Only where there is a picture: beneath transparency the colour is whatever.
                visible = (a[..., 3] > 0) | (b[..., 3] > 0)
                diff = np.abs(a - b)[visible] if visible.any() else np.zeros(1)
                worst = float(diff.max()) if diff.size else 0.0
                if worst == 0.0:
                    totals['pixel_same'] += 1
                elif worst <= 2.0 / 255:
                    totals['pixel_close'] += 1
                else:
                    totals['pixel_far'] += 1
                    problems.append(f'pixels: {number}: {mine_layer["name"]!r} differ by up to {worst * 255:.1f} levels')
        totals['props'] += props_ok
        totals['rects'] += rects_ok
        # The merged image the file stores, against the original's stored one.
        try:
            m, o = rgba(after.numpy()), rgba(before.numpy())
            if m.shape != o.shape:
                totals['merged_na'] += 1
            else:
                worst = float(np.abs(m - o).mean()) * 255
                if worst == 0.0:
                    totals['merged_same'] += 1
                elif worst < 1.0:
                    totals['merged_close'] += 1
                else:
                    totals['merged_far'] += 1
        except Exception:  # noqa: BLE001
            totals['merged_na'] += 1
    print(f'{time.time() - started:.0f} s')
    for key, value in totals.items():
        print(f'{key:14} {value}')
    print(f'{len(problems)} notes')
    groups = {}
    for line in problems:
        key = line.split(':')[0] if line.split(':')[0] in ('shape', 'pixels') else (
            'not in the original' if 'not in the original' in line else
            'could not be opened' if 'could not be opened' in line else
            'opacity' if ' opacity ' in line else 'property')
        groups.setdefault(key, []).append(line)
    for key, lines in groups.items():
        print(f'--- {key}: {len(lines)}')
        for line in lines[:12]:
            print('  ', line)


if __name__ == '__main__':
    try:
        main()
    except Exception:  # noqa: BLE001
        traceback.print_exc()
        sys.exit(1)
