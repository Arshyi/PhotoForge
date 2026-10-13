"""Break the clipboard code in named ways and confirm that the tests notice.

Usage: mutate_clip.py [name...]. The real-clipboard tests replace the clipboard, so this runs them
(with PHOTOFORGE_CLIPBOARD_TESTS=1); back the clipboard up first.
"""
import os
import sys
sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
from pfenv import CARGO_SH, CRATE_PATH, REPO_PATH, WORK_PATH  # noqa: E402
import os
import subprocess
import sys
import time

SRC = str(CRATE_PATH / 'src')
DIB = SRC + r'\clipboard\dib.rs'
MOD = SRC + r'\clipboard\mod.rs'
CMD = SRC + r'\commands\clipboard.rs'

# (file, old, new, scope): scope 'lib' runs the unit tests, 'real' the real-clipboard tests.
MUTATIONS = {
    'alpha-dropped': (DIB, 'let alpha = a.map_or(255, |a| a.value(value));\n                    any_alpha |= alpha != 0 && a.is_some();\n                    Rgba([r.value(value), g.value(value), b.value(value), alpha])\n                }\n                _ => unreachable',
                      'let alpha = 255;\n                    any_alpha |= alpha != 0 && a.is_some();\n                    Rgba([r.value(value), g.value(value), b.value(value), alpha])\n                }\n                _ => unreachable', 'lib'),
    'rows-not-flipped': (DIB, 'let source_row = if top_down { row } else { height - 1 - row };', 'let source_row = if top_down { height - 1 - row } else { row };', 'lib'),
    'mask-table-not-skipped': (DIB, '        cursor += 12;\n    }', '        cursor += 0;\n    }', 'lib'),
    'unfilled-alpha-obeyed': (DIB, 'if has_alpha_channel && !any_alpha {', 'if false && has_alpha_channel && !any_alpha {', 'lib'),
    'dib-limit-ignored': (DIB, '    if pixels > max_pixels {\n        return Err(DibError::TooLarge {', '    if false && pixels > max_pixels {\n        return Err(DibError::TooLarge {', 'lib'),
    'png-limit-ignored': (MOD, '    if pixels > max_pixels {\n        return Err(ClipboardError::TooLarge {', '    if false && pixels > max_pixels {\n        return Err(ClipboardError::TooLarge {', 'lib'),
    'palette-ignored': (DIB, 'let [r, g, b] = palette.get(index).copied().unwrap_or([0, 0, 0]);', 'let [r, g, b] = [index as u8, index as u8, index as u8];', 'lib'),
    'selection-alpha-ignored': (CMD, 'pixel.0[3] = ((alpha + 127) / 255) as u8;', 'pixel.0[3] = pixel.0[3];', 'lib'),
    'selection-not-cropped': (CMD, '        return Ok(\n            image::imageops::crop_imm(&image, bounds.0, bounds.1, bounds.2, bounds.3).to_image(),\n        );\n    }\n    if trim_to_content {',
                              '        let _ = bounds;\n        return Ok(image);\n    }\n    if trim_to_content {', 'lib'),
    'layer-not-trimmed': (CMD, '    if trim_to_content {', '    if false && trim_to_content {', 'lib'),
    'png-not-written': (MOD, '    clipboard.set(png_format, &png)?;\n', '', 'real'),
    'dibv5-not-written': (MOD, '    clipboard.set(win::dibv5_format(), &dib)?;\n    Ok(Written {', '    let _ = &dib;\n    Ok(Written {', 'real'),
    'dib-preferred-over-png': (MOD, '    if let Some(bytes) = clipboard.get(png_format)? {', '    if let Some(bytes) = clipboard.get(png_format).ok().flatten().filter(|_| false) {', 'real'),
    'clipboard-not-emptied': (MOD, '    clipboard.clear()?;\n    clipboard.set(png_format, &png)?;', '    clipboard.set(png_format, &png)?;', 'real'),
    'has-image-always-true': (MOD, '        Err(_) => false,\n    }\n}\n\n#[cfg(not(windows))]\npub fn write_image', '        Err(_) => true,\n    }\n}\n\n#[cfg(not(windows))]\npub fn write_image', 'real'),
}


def run(name):
    path, old, new, scope = MUTATIONS[name]
    original = open(path, encoding='utf-8', newline='').read()
    assert original.count(old) == 1, (name, original.count(old))
    open(path, 'w', encoding='utf-8', newline='').write(original.replace(old, new))
    os.utime(path, None)
    try:
        env = dict(os.environ)
        if scope == 'lib':
            command = ['bash', str(CARGO_SH), 'test', '--lib', 'clipboard']
        else:
            env['PHOTOFORGE_CLIPBOARD_TESTS'] = '1'
            command = ['bash', str(CARGO_SH), 'test', '--test', 'clipboard_real',
                       '--test', 'clipboard_ipc', '--', '--test-threads=1']
        result = subprocess.run(command, capture_output=True, text=True, env=env,
                                cwd=str(CRATE_PATH))
        text = result.stdout + result.stderr
        failed = [line.split(' ... ')[0].replace('test ', '') for line in text.splitlines() if line.endswith('FAILED')]
        compile_error = 'error[' in text or 'could not compile' in text
        return result.returncode != 0, failed, compile_error
    finally:
        open(path, 'w', encoding='utf-8', newline='').write(original)
        os.utime(path, (time.time() + 5, time.time() + 5))


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
