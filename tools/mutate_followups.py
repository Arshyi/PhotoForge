"""Break the follow-up fixes in named ways and confirm that the tests notice."""
import os
import sys
sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
from pfenv import CARGO_SH, CRATE_PATH, REPO_PATH, WORK_PATH, dry_run  # noqa: E402
import os
import subprocess
import sys
import time

SRC = str(CRATE_PATH / 'src')
CRATE = str(CRATE_PATH)
PROJECT = SRC + r'\layers\project.rs'
DIB = SRC + r'\clipboard\dib.rs'
MOD = SRC + r'\clipboard\mod.rs'
IMAGE_IO = SRC + r'\infrastructure\image_io.rs'
COLOUR = SRC + r'\openexr\colour.rs'
BLEND = SRC + r'\layers\blend.rs'
MASK = SRC + r'\commands\mask.rs'

LIB = ['test', '--lib']
MUTATIONS = {
    'clipping-not-a-reason-for-v3': (PROJECT, '.any(|layer| layer.clipped || layer.blend_mode.is_photoshop_only());', '.any(|layer| layer.blend_mode.is_photoshop_only());', LIB + ['layers::project']),
    'blend-modes-not-a-reason-for-v3': (PROJECT, '.any(|layer| layer.clipped || layer.blend_mode.is_photoshop_only());', '.any(|layer| layer.clipped);', LIB + ['layers::project']),
    'always-v3': (PROJECT, '    if newer {\n        PROJECT_FORMAT_VERSION\n    } else {\n        BASE_FORMAT_VERSION\n    }', '    if newer || true {\n        PROJECT_FORMAT_VERSION\n    } else {\n        BASE_FORMAT_VERSION\n    }', LIB + ['layers::project']),
    'v3-not-readable': (PROJECT, 'if !(1..=PROJECT_FORMAT_VERSION).contains(&version) {', 'if !(1..=BASE_FORMAT_VERSION).contains(&version) {', LIB + ['layers::project']),
    'one-photoshop-mode-forgotten': (BLEND, '                | Self::HardMix\n                | Self::Subtract', '                | Self::Subtract', LIB + ['layers::project']),
    'colour-table-read-as-pixels': (DIB, '    } else if colours_used as usize <= 256 {\n        colours_used as usize\n    } else {', '    } else if colours_used as usize <= 256 {\n        0\n    } else {', LIB + ['clipboard']),
    'mask-overflow': (DIB, 'if u64::from(max) & (u64::from(max) + 1) != 0 {', 'if max & (max + 1) != 0 {', LIB + ['clipboard']),
    'png-byte-count-overflows': (MOD, '            bytes: pixels.saturating_mul(4),\n            limit: max_pixels.saturating_mul(4),', '            bytes: pixels * 4,\n            limit: max_pixels * 4,', LIB + ['clipboard']),
    'windows-hard-link-not-seen': (IMAGE_IO, '    matches!((identity(left), identity(right)), (Some(a), Some(b)) if a == b)', '    let _ = (identity(left), identity(right));\n    false', LIB + ['infrastructure::image_io']),
    'primaries-conditioning-removed': (COLOUR, 'if !(spread > 0.0) || volume / spread < 1e-3 {', 'if !(spread > 0.0) {', LIB + ['openexr::colour']),
    'mask-png-over-the-source': (MASK, '            crate::infrastructure::image_io::refuse_to_replace_source(&source, &destination)?;\n            export_png_snapshot_with_progress(', '            let _ = &source;\n            export_png_snapshot_with_progress(', ['test', '--test', 'mask_export_guard']),
    'mask-file-over-the-source': (MASK, '            crate::infrastructure::image_io::refuse_to_replace_source(&source, &destination)?;\n            save_mask_with_progress(', '            let _ = &source;\n            save_mask_with_progress(', ['test', '--test', 'mask_export_guard']),
}


def run(name):
    path, old, new, test_args = MUTATIONS[name]
    original = open(path, encoding='utf-8', newline='').read()
    crlf = '\r\n' in original
    if crlf:
        old, new = old.replace('\n', '\r\n'), new.replace('\n', '\r\n')
    assert original.count(old) == 1, (name, original.count(old))
    open(path, 'w', encoding='utf-8', newline='').write(original.replace(old, new))
    os.utime(path, None)
    try:
        command = ['bash', str(CARGO_SH)] + test_args
        result = subprocess.run(command, capture_output=True, text=True, cwd=CRATE)
        text = result.stdout + result.stderr
        failed = [line.split(' ... ')[0].replace('test ', '') for line in text.splitlines() if line.endswith('FAILED')]
        compile_error = 'error[' in text or 'could not compile' in text
        return result.returncode != 0, failed, compile_error
    finally:
        open(path, 'w', encoding='utf-8', newline='').write(original)
        os.utime(path, (time.time() + 5, time.time() + 5))


dry_run([(name, spec[0], spec[1]) for name, spec in MUTATIONS.items()])


if __name__ == '__main__':
    names = sys.argv[1:] or list(MUTATIONS)
    summary = []
    for name in names:
        caught, failed, compile_error = run(name)
        status = 'COMPILE ERROR' if compile_error else ('CAUGHT' if caught else 'SURVIVED')
        print(f'{name}: {status}  {failed[:3]}', flush=True)
        summary.append((name, status))
    survivors = [n for n, s in summary if s != 'CAUGHT']
    print(f'\n{len(summary) - len(survivors)} of {len(summary)} caught; not caught: {survivors}')
