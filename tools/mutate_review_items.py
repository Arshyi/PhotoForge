"""Mutation checks for the three review items fixed after 0.15.0 (Phase 15.11): highlights under the
blend modes defined on 0 to 1, the PSD importer not decoding channels it does not use, the PSD
exporter's guard and piecewise writing, and the SVG importer and exporter changes.

Usage: python -I tools/mutate_review_items.py [--dry] [index ...]

Each mutant is one replacement in one source file; the test filter that should notice it is run
(cargo test --lib <filter>); the file is restored, and its modification time moved forward so cargo
rebuilds. Never format, commit or build in another window while it runs. With --dry it only checks
that each pattern still matches exactly once.
"""
import os
import subprocess
import sys
import time

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
from pfenv import CARGO_SH, CRATE_PATH, dry_run, fit_line_ends  # noqa: E402

SRC = CRATE_PATH / 'src'
LINEAR = str(SRC / 'layers' / 'linear.rs')
PSD_IMPORT = str(SRC / 'psd' / 'import.rs')
PSD_EXPORT = str(SRC / 'psd' / 'export.rs')
SVG_IMPORT = str(SRC / 'svg' / 'import.rs')
SVG_EXPORT = str(SRC / 'svg' / 'export.rs')

# (name, file, old, new, test filter)
MUTANTS = [
    ('a backdrop highlight is not added back', LINEAR,
     '        blended[i] + highlight\n', '        blended[i]\n', 'layers::linear'),
    ('a source highlight is added back too', LINEAR,
     '        blended[i] + highlight\n', '        blended[i] + highlight + (s[i] - 1.0).max(0.0)\n',
     'layers::linear'),
    ('Multiply is clamped again', LINEAR,
     'BlendMode::Multiply => return std::array::from_fn(|i| b[i] * s[i]),',
     'BlendMode::Multiply => return std::array::from_fn(|i| (b[i] * s[i]).min(1.0)),',
     'layers::linear'),
    ('the in-range operands are bounded below white', LINEAR,
     '            value.clamp(0.0, 1.0)\n        } else {\n            0.0',
     '            value.clamp(0.0, 0.9)\n        } else {\n            0.0', 'layers::linear'),
    ('a backdrop below white is pushed down too', LINEAR,
     '(b[i] - 1.0).max(0.0)', '(b[i] - 1.0)', 'layers::linear'),
    ('the importer decodes every channel again', PSD_IMPORT,
     'if channel.id < -1 || channel.id >= colour_planes {', 'if channel.id < -1 {',
     'psd::tests::importer'),
    ('the importer reads three planes for grey too', PSD_IMPORT,
     'let colour_planes: i16 = if gray { 1 } else { 3 };',
     'let colour_planes: i16 = if gray { 3 } else { 3 };', 'psd::tests::importer'),
    ('the export guard forgets the rendered picture', PSD_EXPORT,
     'let rendered = canvas_pixels * if float { 16 } else { 4 };',
     'let rendered = 0;', 'psd::export::estimate_tests'),
    ('the export guard counts the largest layer once', PSD_EXPORT,
     'let peak = (layers_raw + largest_layer).max(layers_raw + rendered + 2 * canvas_raw);',
     'let peak = layers_raw.max(layers_raw + rendered + 2 * canvas_raw);',
     'psd::export::estimate_tests'),
    ('the layer information is not padded', PSD_EXPORT,
     '    pieces.push(&padding);\n', '', 'psd::tests::exporter'),
    ('the layer data is written before the records', PSD_EXPORT,
     'let mut pieces: Vec<&[u8]> = vec![&out, &section_head, &info_head];\n    pieces.extend(layer_data(&writer.layers));',
     'let mut pieces: Vec<&[u8]> = vec![&out, &section_head];\n    pieces.extend(layer_data(&writer.layers));\n    pieces.push(&info_head);',
     'psd::tests'),
    ('hsl hue is read as sixty degrees to the sector', SVG_IMPORT,
     'let k = (offset + hue / 30.0).rem_euclid(12.0);', 'let k = (offset + hue / 60.0).rem_euclid(12.0);',
     'svg::import'),
    ('a rebeccapurple of another value', SVG_IMPORT,
     '("rebeccapurple", 0x663399),\n    ("red", 0xff0000),',
     '("rebeccapurple", 0x663398),\n    ("red", 0xff0000),', 'svg::import'),
    ('a title is not a name', SVG_IMPORT,
     'title.or_else(|| node.attribute("id").map(str::to_string))',
     'node.attribute("id").map(str::to_string).or(title.filter(|_| false))', 'svg::import'),
    ('the normalised diagonal is not divided', SVG_IMPORT,
     '((viewport.0 * viewport.0 + viewport.1 * viewport.1) / 2.0).sqrt()',
     '((viewport.0 * viewport.0 + viewport.1 * viewport.1) / 1.0).sqrt()', 'svg::import'),
    ('percentages swap the sides of the viewport', SVG_IMPORT,
     '        let (cx, cy) = self.viewport;\n', '        let (cy, cx) = self.viewport;\n',
     'svg::import'),
    ('an isolated group is written plain', SVG_EXPORT,
     'if isolated && layer.opacity >= 1.0 && css_blend(layer.blend_mode).is_none() {',
     'if false && isolated && layer.opacity >= 1.0 && css_blend(layer.blend_mode).is_none() {',
     'svg::export'),
    ('a pass-through group is written isolated', SVG_EXPORT,
     'if isolated && layer.opacity >= 1.0 && css_blend(layer.blend_mode).is_none() {',
     'if layer.opacity >= 1.0 && css_blend(layer.blend_mode).is_none() {', 'svg::export'),
]


def run(index):
    name, path, old, new, filter_ = MUTANTS[index]
    original = open(path, encoding='utf-8', newline='').read()
    old, new = fit_line_ends(original, old, new)
    if original.count(old) != 1:
        return 'BAD PATTERN (%d matches)' % original.count(old)
    try:
        open(path, 'w', encoding='utf-8', newline='').write(original.replace(old, new))
        os.utime(path, None)
        result = subprocess.run(['bash', str(CARGO_SH), 'test', '--lib', filter_],
                                capture_output=True, text=True, encoding='utf-8', errors='replace',
                                timeout=1500)
    finally:
        open(path, 'w', encoding='utf-8', newline='').write(original)
        os.utime(path, (time.time() + 5, time.time() + 5))
    text = result.stdout + result.stderr
    if 'error[' in text or 'could not compile' in text:
        return 'COMPILE ERROR'
    if result.returncode == 0:
        return 'SURVIVED'
    failed = [line.split(' ... ')[0].replace('test ', '') for line in text.splitlines()
              if line.endswith('FAILED')]
    return 'caught by %d: %s' % (len(failed), '; '.join(failed[:2]))


dry_run([(m[0], m[1], m[2]) for m in MUTANTS])

if __name__ == '__main__':
    chosen = [int(a) for a in sys.argv[1:] if a.isdigit()] or range(len(MUTANTS))
    for index in chosen:
        print(index, MUTANTS[index][0], '->', run(index), flush=True)
