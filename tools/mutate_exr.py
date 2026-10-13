"""Break the OpenEXR code in named ways and confirm that the tests notice.

Each mutation is a (file, old, new) replacement made once, the tests run, and the file restored
(with its modification time moved forward, so cargo rebuilds it). Usage: mutate_exr.py [name...]
"""
import os
import sys
sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
from pfenv import (CARGO_SH, CRATE_PATH, REPO_PATH, WORK_PATH, dry_run,  # noqa: E402
                   fit_line_ends)
import os
import subprocess
import sys
import time

SRC = str(CRATE_PATH / 'src')
IMPORT = SRC + r'\openexr\import.rs'
EXPORT = SRC + r'\openexr\export.rs'
COLOUR = SRC + r'\openexr\colour.rs'
COMMANDS = SRC + r'\commands\layers.rs'

MUTATIONS = {
    'no-unpremultiply': (IMPORT, 'r /= a;\n                g /= a;\n                b /= a;', 'let _ = a;', 'lib'),
    'primaries-ignored': (IMPORT, 'let matrix = if admission.primaries.is_rec709() {', 'let matrix = if true {', 'lib'),
    'dwaa-not-refused': (IMPORT, 'Compression::DWAA(_) => Some("DWAA"),', 'Compression::DWAA(_) => None,', 'lib'),
    'luma-chroma-read-as-grey': (IMPORT, 'if channels.contains_key("RY") && channels.contains_key("BY") {', 'if false {', 'lib'),
    'unfilled-alpha-obeyed': (IMPORT, '&& raw.iter().all(|p| !(p.alpha > 0.0))', '&& false', 'lib'),
    'empty-picture-made-opaque': (IMPORT, '.any(|p| p.red != 0.0 || p.green != 0.0 || p.blue != 0.0)', '.any(|_| true)', 'lib'),
    'passes-visible': (IMPORT, 'layer.visible = index == 0;', 'layer.visible = true;', 'lib'),
    'alpha-not-clamped': (IMPORT, 'alpha = a.clamp(0.0, 1.0);', 'alpha = a;', 'lib'),
    'offset-ignored': (IMPORT, 'layer.transform.translate_x = admission.offset.0 as f32;', 'layer.transform.translate_x = 0.0;', 'lib'),
    'infinity-passes': (IMPORT, '} else if value.is_infinite() {', '} else if false {', 'lib'),
    'store-budget-ignored': (IMPORT, 'while fitting > 1 && layer_bytes * fitting as u64 > limits.store_bytes {', 'while false && layer_bytes * fitting as u64 > limits.store_bytes {', 'lib'),
    'dimension-limit-ignored': (IMPORT, 'if w > hard || h > hard {', 'if false {', 'lib'),
    'entry-budget-ignored': (IMPORT, 'if layer_bytes > limits.entry_bytes {', 'if false {', 'lib'),
    'integer-channels-read': (IMPORT, 'if red.sample_type == SampleType::U32 {', 'if false {', 'lib'),
    'emissive-uncounted': (IMPORT, 'findings.emissive += 1;', 'findings.emissive += 0;', 'lib'),
    'deep-not-refused': (IMPORT, 'if header.deep {\n            deep_parts += 1;', 'if false {\n            deep_parts += 1;', 'lib'),
    'export-straight-alpha': (EXPORT, '(p.red * p.alpha, p.green * p.alpha, p.blue * p.alpha, p.alpha)', '(p.red, p.green, p.blue, p.alpha)', 'lib'),
    'export-wrong-primaries': (EXPORT, 'red: Vec2(0.6400, 0.3300),', 'red: Vec2(0.7000, 0.3300),', 'lib'),
    'export-half-not-clamped': (EXPORT, 'f16::from_f32(v.clamp(-HALF_MAX, HALF_MAX))', 'f16::from_f32(v)', 'lib'),
    'export-always-alpha': (EXPORT, 'let has_alpha = picture.pixels().iter().any(|p| p.alpha < 1.0);', 'let has_alpha = true;', 'lib'),
    'export-half-is-single': (EXPORT, 'Some(16) => 16,', 'Some(16) => 32,', 'lib'),
    'export-edits-ignored': (EXPORT, 'pipeline_typed(rendered, &options.operations, None)?', 'pipeline_typed(rendered, &[], None)?', 'lib'),
    'export-clamp-uncounted': (EXPORT, 'clamped += 1;', 'clamped += 0;', 'lib'),
    'export-bad-depth-accepted': (EXPORT, 'Some(other) => {\n            return Err(AppError::ProcessingFailure(', 'Some(other) if other == 8 => 32,\n        Some(other) => {\n            return Err(AppError::ProcessingFailure(', 'lib'),
    'adaptation-skipped': (COLOUR, 'let full = multiply(&target_inverse, &multiply(&adaptation, &source));', 'let _ = &adaptation;\n        let full = multiply(&target_inverse, &source);', 'lib'),
    'tolerance-zero': (COLOUR, 'const SAME_TOLERANCE: f32 = 1.5e-3;', 'const SAME_TOLERANCE: f32 = 1.0e-9;', 'lib'),
    'extension-unchecked': (COMMANDS, 'if !is_exr || !destination.is_absolute() ||', 'if !destination.is_absolute() ||', 'ipc'),
}


def run(name):
    path, old, new, scope = MUTATIONS[name]
    original = open(path, encoding='utf-8', newline='').read()
    old, new = fit_line_ends(original, old, new)
    assert original.count(old) == 1, (name, original.count(old))
    open(path, 'w', encoding='utf-8', newline='').write(original.replace(old, new))
    os.utime(path, None)
    try:
        if scope == 'lib':
            command = ['bash', str(CARGO_SH), 'test', '--lib', 'openexr', '--',
                       '--skip', 'every_truncation_and_changed_byte']
        else:
            command = ['bash', str(CARGO_SH), 'test', '--test', 'exr_ipc']
        result = subprocess.run(command, capture_output=True, text=True)
        text = result.stdout + result.stderr
        caught = result.returncode != 0
        failed = [line.split(' ... ')[0].replace('test ', '') for line in text.splitlines() if line.endswith('FAILED')]
        compile_error = 'error[' in text or 'could not compile' in text
        return caught, failed, compile_error
    finally:
        open(path, 'w', encoding='utf-8', newline='').write(original)
        os.utime(path, (time.time() + 5, time.time() + 5))


dry_run([(name, spec[0], spec[1]) for name, spec in MUTATIONS.items()])


if __name__ == '__main__':
    names = sys.argv[1:] or list(MUTATIONS)
    summary = []
    for name in names:
        caught, failed, compile_error = run(name)
        status = 'CAUGHT' if caught and not compile_error else ('COMPILE ERROR' if compile_error else 'SURVIVED')
        print(f'{name}: {status}  {failed[:3]}', flush=True)
        summary.append((name, status))
    print()
    survived = [n for n, s in summary if s != 'CAUGHT']
    print(f'{len(summary) - len(survived)} of {len(summary)} caught; not caught: {survived}')
