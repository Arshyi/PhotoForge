"""Mutation checks for the TIFF increment: break the code one way at a time and require a test to
fail. Every file is restored (and verified) after each mutant, however the run ends.

Usage: python -I mutate_tiff.py [--dry] [index ...]
"""
import os
import sys
sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
from pfenv import CARGO_SH, CRATE_PATH, REPO_PATH, WORK_PATH  # noqa: E402
import subprocess
import sys

ROOT = str(CRATE_PATH)
IMPORT = ROOT + r'\src\tiff\import.rs'
CODEC = ROOT + r'\src\tiff\codec.rs'
STRUCT = ROOT + r'\src\tiff\structure.rs'
EXPORT = ROOT + r'\src\tiff\export.rs'
GATHER = ROOT + r'\src\tiff\metadata.rs'
MTIFF = ROOT + r'\src\metadata\tiff.rs'
IMAGE_IO = ROOT + r'\src\infrastructure\image_io.rs'
COLOR_IO = ROOT + r'\src\infrastructure\color_io.rs'

LIB = ['test', '--lib', 'tiff::']
IPC = ['test', '--test', 'tiff_ipc']

MUTANTS = [
    ('orientation 6 is not rotated', IMPORT, '6 => (height - 1 - y, x),', '6 => (y, x),', LIB),
    ('white is zero is not inverted', IMPORT,
     'let invert = layout.kind == Kind::WhiteIsZero;', 'let invert = false;', LIB),
    ('8-bit premultiplied alpha is kept premultiplied', IMPORT,
     'let straight = (u32::from(*channel) * 255 + alpha / 2) / alpha;',
     'let straight = u32::from(*channel);', LIB),
    ('the floating-point predictor is read in the file order', IMPORT,
     '''        let order = if layout.predictor == 3 {
            Order::Big
        } else {
            order
        };''', '        let order = order;', LIB),
    ('16-bit is not decoded from sRGB', IMPORT,
     'pixel.red = srgb_decode(pixel.red);', 'pixel.red = pixel.red;', LIB),
    ('no admission by the budget', IMPORT,
     'if raster_bytes > limits.entry_bytes || raster_bytes > limits.store_bytes {', 'if false {', LIB),
    ('overlapping strips are read', IMPORT, 'if declared > tiff.length() {', 'if false {', LIB),
    ('a reduced copy can be the picture', IMPORT,
     'if subfile & 0b101 == 0 && stored &&', 'if stored &&', LIB),
    ('unassociated alpha is not alpha', IMPORT,
     'Some(2) => Alpha::Straight,', 'Some(2) => Alpha::None,', LIB),
    ('8-bit is scaled by 256', IMPORT,
     '8 => f32::from(row[index]) / 255.0,', '8 => f32::from(row[index]) / 256.0,', LIB),
    ('palette entries are 8-bit', IMPORT,
     'f32::from(*component) / 65535.0', 'f32::from(*component) / 255.0', LIB),
    ('non-finite samples are not counted', IMPORT, 'findings.non_finite += 1;', '', LIB),
    ('the profile is not applied', IMPORT,
     'transform.apply(&encoded[..used], &mut linear[..used])?;',
     'let _ = (&transform, &encoded, &mut linear, used);', LIB),
    ('tiles are placed transposed', IMPORT,
     'let (x, y) = (tx * width, ty * height);', 'let (x, y) = (ty * width, tx * height);', LIB),
    ('every plane is the first', IMPORT, 'let plane = index / per_plane;', 'let plane = 0;', LIB),
    ('bits stored least significant first are read', IMPORT,
     'if first(tiff, TAG_FILL_ORDER)?.unwrap_or(1) != 1 {', 'if false {', LIB),
    ('the maker note is not noticed (picture)', GATHER,
     'let maker_note = ifd0.has(TAG_DNG_PRIVATE_DATA)', 'let maker_note = false && ifd0.has(TAG_DNG_PRIVATE_DATA)', LIB),
    ('the byte order of a block is lost (picture)', GATHER,
     'big_endian: tiff.order() == Order::Big,', 'big_endian: false,', LIB),
    ('LZW changes width late', CODEC,
     'if next + 1 >= (1 << width) && width < 12 {', 'if next >= (1 << width) && width < 12 {', LIB),
    ('PackBits repeats one too many', CODEC,
     'let count = 1 - control as isize;', 'let count = 2 - control as isize;', LIB),
    ('16-bit differencing is read swapped', CODEC,
     'let current = order.u16([row[at], row[at + 1]]);', 'let current = order.u16([row[at + 1], row[at]]);', LIB),
    ('the floating-point predictor does not accumulate', CODEC,
     'row[index] = row[index].wrapping_add(row[index - stride]);',
     'row[index] = row[index].wrapping_sub(row[index - stride]);', LIB),
    ('a read past the end is not caught', STRUCT, 'if end > self.length {', 'if false {', LIB),
    ('8-bit differencing is added', EXPORT,
     'row[index] = row[index].wrapping_sub(row[index - samples]);',
     'row[index] = row[index].wrapping_add(row[index - samples]);', LIB),
    ('16-bit differencing is added', EXPORT,
     'let bytes = current.wrapping_sub(earlier).to_le_bytes();',
     'let bytes = current.wrapping_add(earlier).to_le_bytes();', LIB),
    ('alpha is said to be associated', EXPORT,
     'ifd0.push(Item::short(TAG_EXTRA_SAMPLES, 2));', 'ifd0.push(Item::short(TAG_EXTRA_SAMPLES, 1));', LIB),
    ('the profile is not embedded', EXPORT,
     'ifd0.push(Item::new(TAG_ICC, 7, profile.len() as u32, profile.to_vec()));', '', LIB),
    ('the maker note is not reported', EXPORT,
     'result.maker_note_dropped = split.maker_note;', 'result.maker_note_dropped = false;', LIB),
    ('the camera block is not carried', EXPORT, 'result.placed.exif = true;', '', LIB),
    ('a big-endian block is not swapped', MTIFF,
     '                if order == Order::Big {\n                    field.value = field.value.map(|value| swap_value(field.kind, value));\n                }',
     '', LIB),
    ('shorts are swapped as bytes', MTIFF, '3 | 8 => 2,\n        4 | 9 | 11 | 13 | 5 | 10 => 4,',
     '3 | 8 => 1,\n        4 | 9 | 11 | 13 | 5 | 10 => 4,', LIB),
    ('a TIFF is not an export format', IMAGE_IO,
     'Some("tif" | "tiff") => Ok(ImageFormat::Tiff),', '', IPC),
    ('16 bits is for PNG only', COLOR_IO,
     '&& !matches!(format, image::ImageFormat::Png | image::ImageFormat::Tiff)',
     '&& !matches!(format, image::ImageFormat::Png)', IPC),
]


def run(args):
    proc = subprocess.run(
        ['bash', str(CARGO_SH)] + args,
        capture_output=True, text=True, encoding='utf-8', errors='replace', timeout=1500)
    return proc.returncode, proc.stdout + proc.stderr


def main():
    args = sys.argv[1:]
    dry = '--dry' in args
    only = {int(a) for a in args if a.isdigit()}
    results = []
    for index, (name, path, old, new, command) in enumerate(MUTANTS):
        if only and index not in only:
            continue
        original = open(path, encoding='utf-8', newline='').read()
        crlf = '\r\n' in original
        pattern = old.replace('\n', '\r\n') if crlf else old
        replacement = new.replace('\n', '\r\n') if crlf else new
        count = original.count(pattern)
        if dry or count != 1:
            print(index, name, 'matches', count, flush=True)
            if count != 1:
                results.append((index, name, 'BAD PATTERN (%d matches)' % count))
            continue
        try:
            open(path, 'w', encoding='utf-8', newline='').write(original.replace(pattern, replacement))
            code, output = run(command)
        finally:
            open(path, 'w', encoding='utf-8', newline='').write(original)
        assert open(path, encoding='utf-8', newline='').read() == original, 'restore failed'
        if 'error[' in output or 'could not compile' in output:
            verdict = 'COMPILE ERROR'
        elif code != 0 and 'test result: FAILED' in output:
            failed = [line.strip() for line in output.splitlines()
                      if line.strip().endswith('FAILED') and line.strip().startswith('test ')]
            verdict = 'caught by %d test(s): %s' % (len(failed), '; '.join(failed[:2]))
        elif code == 0:
            verdict = 'SURVIVED'
        else:
            verdict = 'UNCLEAR rc=%d' % code
        results.append((index, name, verdict))
        print(index, name, '->', verdict, flush=True)
    if not dry:
        print('---- summary')
        for index, name, verdict in results:
            print(index, name, '->', verdict)


main()
