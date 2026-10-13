"""Mutation checks for the SVG importer: break one thing, expect a test to fail, restore."""
import os
import sys
sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
from pfenv import (CARGO_SH, CRATE_PATH, REPO_PATH, WORK_PATH, dry_run,  # noqa: E402
                   fit_line_ends)
import os
import shutil
import subprocess

ROOT = str(CRATE_PATH)
CARGO = str(CARGO_SH)
F = 'src/svg/import.rs'

MUTANTS = [
    ('a child transform applied after its group', F,
     'Some(local) => parent_matrix.then_after(local),\n                None => {\n                    self.note(\n                        ReportKind::Approximated,\n                        Some(&name),\n                        "A transform that could not be read was ignored.",\n                    );\n                    *parent_matrix\n                }\n            },\n        };\n        let reference',
     'Some(local) => local.then_after(*parent_matrix),\n                None => {\n                    self.note(\n                        ReportKind::Approximated,\n                        Some(&name),\n                        "A transform that could not be read was ignored.",\n                    );\n                    *parent_matrix\n                }\n            },\n        };\n        let reference'),
    ('colours not decoded to linear light', F,
     'srgb_decode(c[0]),\n                srgb_decode(c[1]),\n                srgb_decode(c[2]),',
     'c[0],\n                c[1],\n                c[2],'),
    ('a DTD allowed', F, 'allow_dtd: false,', 'allow_dtd: true,'),
    ('fill none read as black', F,
     'if value == "none" {\n            return Some(Paint::None);\n        }',
     'if value == "none" {\n            return Some(Paint::Colour([0.0, 0.0, 0.0, 1.0]));\n        }'),
    ('group opacity dropped', F,
     'layer.opacity = resolved.opacity;\n        layer.visible = resolved.style.visible;\n        self.report.groups_imported += 1;',
     'layer.visible = resolved.style.visible;\n        self.report.groups_imported += 1;'),
    ('the arc sweep flag inverted', F,
     'if !sweep && delta > 0.0 {\n        delta -= std::f64::consts::TAU;\n    } else if sweep && delta < 0.0 {\n        delta += std::f64::consts::TAU;\n    }',
     'if sweep && delta > 0.0 {\n        delta -= std::f64::consts::TAU;\n    } else if !sweep && delta < 0.0 {\n        delta += std::f64::consts::TAU;\n    }'),
    ('the view box not centred', F,
     '(f64::from(canvas_width) - vw * scale) / 2.0,', '0.0,'),
    ('stroke width not scaled by the transform', F,
     'let stroke_width = f64::from(resolved.style.stroke_width) * matrix.mean_scale();',
     'let stroke_width = f64::from(resolved.style.stroke_width);'),
    ('the shape limit not applied', F,
     'if self.layers >= MAX_LAYERS {', 'if false && self.layers >= MAX_LAYERS {'),
    ('a path error not reported', F,
     '                if !ok {\n                    self.note(', '                if false && !ok {\n                    self.note('),
]

TESTS = [
    ['test', '--offline', '--all-features', '--lib', 'svg::import'],
    ['test', '--offline', '--all-features', '--test', 'svg_import_ipc'],
]


def run(args):
    result = subprocess.run(['bash', CARGO] + args, capture_output=True, text=True, timeout=900)
    text = result.stdout + result.stderr
    failed = [l.strip() for l in text.splitlines() if l.startswith('test ') and l.rstrip().endswith('FAILED')]
    return failed, ('error[' in text or 'could not compile' in text)


dry_run([(m[0], os.path.join(ROOT, m[1]), m[2]) for m in MUTANTS])


for name, path, old, new in MUTANTS:
    full = os.path.join(ROOT, path)
    original = open(full, 'rb').read().decode()
    old, new = fit_line_ends(original, old, new)
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
