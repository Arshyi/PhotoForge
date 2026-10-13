"""Mutation checks for the SVG exporter: break one thing, expect a test to fail, restore."""
import os
import sys
sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
from pfenv import CARGO_SH, CRATE_PATH, REPO_PATH, WORK_PATH  # noqa: E402
import os
import shutil
import subprocess
import sys

ROOT = str(CRATE_PATH)
CARGO = str(CARGO_SH)

MUTANTS = [
    ('translation left out of the matrix', 'src/svg/export.rs',
     'let e = cx + transform.translate_x - (a * cx + c * cy);', 'let e = cx - (a * cx + c * cy);'),
    ('colours not encoded to sRGB', 'src/svg/export.rs',
     '(srgb_encode(linear.clamp(0.0, 1.0)) * 255.0).round() as u8', '(linear.clamp(0.0, 1.0) * 255.0).round() as u8'),
    ('open geometry filled', 'src/svg/export.rs',
     '.filter(|c| c.alpha > 0.0 && shape.geometry.is_closed());', '.filter(|c| c.alpha > 0.0 && true);'),
    ('names not escaped', 'src/svg/export.rs',
     "'<' => out.push_str(\"&lt;\"),", "'<' => out.push('<'),"),
    ('bevel written as miter', 'src/svg/export.rs',
     'LineJoin::Bevel => paint.push_str(" stroke-linejoin=\\"bevel\\""),',
     'LineJoin::Bevel => paint.push_str(" stroke-linejoin=\\"miter\\""),'),
    ('hidden layers shown', 'src/svg/export.rs',
     'attributes.push_str(" display=\\"none\\"");', 'let _ = &mut attributes;'),
    ('the pass-through note never made', 'src/svg/export.rs',
     'if !isolated && (layer.opacity < 1.0 || layer.blend_mode != BlendMode::Normal) {',
     'if false && !isolated && (layer.opacity < 1.0 || layer.blend_mode != BlendMode::Normal) {'),
    ('a bevel built on one side only (the stroker)', 'src/vector/stroke.rs',
     '[1.0_f32, -1.0]', '[1.0_f32]'),
]

TESTS = [
    ['test', '--all-features', '--lib', 'svg::'],
    ['test', '--all-features', '--lib', 'vector::stroke'],
    ['test', '--all-features', '--test', 'svg_export_ipc'],
]


def run(args):
    result = subprocess.run(['bash', CARGO] + args, capture_output=True, text=True, timeout=900)
    text = result.stdout + result.stderr
    failed = [l.strip() for l in text.splitlines() if l.startswith('test ') and l.rstrip().endswith('FAILED')]
    return failed, ('error[' in text or 'could not compile' in text)


for name, path, old, new in MUTANTS:
    if 'pass-through' not in name:
        continue
    full = os.path.join(ROOT, path)
    original = open(full, 'rb').read().decode()
    if original.count(old) != 1:
        print(f'SKIP     {name}: pattern found {original.count(old)} times')
        continue
    backup = full + '.mutbak'
    shutil.copyfile(full, backup)
    try:
        open(full, 'wb').write(original.replace(old, new).encode())
        caught, broken = [], False
        for args in TESTS:
            failed, compile_error = run(args)
            if compile_error:
                broken = True
                break
            caught += failed
        if broken:
            print(f'BROKEN   {name}')
        elif caught:
            print(f'CAUGHT   {name}: {len(caught)} failing, e.g. {caught[0][:100]}')
        else:
            print(f'SURVIVED {name}')
    finally:
        shutil.move(backup, full)
        os.utime(full, None)
print('restored')
