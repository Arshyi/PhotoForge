"""Check PhotoForge's clipboard code against Pillow and the Win32 API, driven from outside.

Each case leaves the clipboard in a known state (by this script with ctypes, or by one of the
`step_*` tests of tests/clipboard_real.rs) and reads it with the other side:

  * PhotoForge writes the sample (PNG + CF_DIBV5, then CF_DIBV5 alone); Pillow's ImageGrab and
    a raw read of each form say what is there.
  * This script writes a picture as CF_DIB (24-bit and 32-bit), CF_DIBV5 and PNG; PhotoForge reads
    it and reports what it found.

The clipboard is the person's: back it up before and restore it after (clipboard_backup.ps1).
"""
import os
import sys
sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
from pfenv import CARGO_SH, CRATE_PATH, REPO_PATH, WORK_PATH  # noqa: E402
import ctypes
import ctypes.wintypes as wt
import io
import json
import os
import struct
import subprocess
import sys

from PIL import Image, ImageGrab

CARGO = str(CARGO_SH)
ROOT = str(CRATE_PATH)
REPORT = str(WORK_PATH / 'clipboard-report.json')

user32 = ctypes.WinDLL('user32', use_last_error=True)
kernel32 = ctypes.WinDLL('kernel32', use_last_error=True)
user32.OpenClipboard.argtypes = [wt.HWND]
user32.OpenClipboard.restype = wt.BOOL
user32.EmptyClipboard.restype = wt.BOOL
user32.CloseClipboard.restype = wt.BOOL
user32.SetClipboardData.argtypes = [wt.UINT, wt.HANDLE]
user32.SetClipboardData.restype = wt.HANDLE
user32.GetClipboardData.argtypes = [wt.UINT]
user32.GetClipboardData.restype = wt.HANDLE
user32.RegisterClipboardFormatW.argtypes = [wt.LPCWSTR]
user32.RegisterClipboardFormatW.restype = wt.UINT
user32.IsClipboardFormatAvailable.argtypes = [wt.UINT]
kernel32.GlobalAlloc.argtypes = [wt.UINT, ctypes.c_size_t]
kernel32.GlobalAlloc.restype = wt.HANDLE
kernel32.GlobalLock.argtypes = [wt.HANDLE]
kernel32.GlobalLock.restype = ctypes.c_void_p
kernel32.GlobalUnlock.argtypes = [wt.HANDLE]
kernel32.GlobalSize.argtypes = [wt.HANDLE]
kernel32.GlobalSize.restype = ctypes.c_size_t

CF_DIB, CF_DIBV5 = 8, 17
GMEM_MOVEABLE = 0x0002


def set_formats(formats):
    """Replace the clipboard with {format number: bytes}."""
    assert user32.OpenClipboard(None)
    try:
        user32.EmptyClipboard()
        for number, data in formats.items():
            handle = kernel32.GlobalAlloc(GMEM_MOVEABLE, len(data))
            pointer = kernel32.GlobalLock(handle)
            ctypes.memmove(pointer, data, len(data))
            kernel32.GlobalUnlock(handle)
            assert user32.SetClipboardData(number, handle)
    finally:
        user32.CloseClipboard()


def get_format(number):
    assert user32.OpenClipboard(None)
    try:
        if not user32.IsClipboardFormatAvailable(number):
            return None
        handle = user32.GetClipboardData(number)
        if not handle:
            return None
        size = kernel32.GlobalSize(handle)
        pointer = kernel32.GlobalLock(handle)
        data = ctypes.string_at(pointer, size)
        kernel32.GlobalUnlock(handle)
        return data
    finally:
        user32.CloseClipboard()


def png_format():
    return user32.RegisterClipboardFormatW('PNG')


def sample():
    """The 6 x 4 picture of tests/clipboard_real.rs, with alpha."""
    image = Image.new('RGBA', (6, 4))
    for y in range(4):
        for x in range(6):
            a = 255 if (x + y) % 3 == 0 else 60 + x * 30
            image.putpixel((x, y), (x * 40, y * 60, 255 - x * 30 - y * 20, a))
    return image


def run_step(step, extra_env=None):
    env = dict(os.environ)
    env['PHOTOFORGE_CLIPBOARD_TESTS'] = '1'
    env['PHOTOFORGE_CLIPBOARD_STEP'] = step
    env.update(extra_env or {})
    result = subprocess.run(
        ['bash', CARGO, 'test', '--test', 'clipboard_real', f'step_', '--', '--test-threads=1'],
        capture_output=True, text=True, env=env, cwd=ROOT)
    assert result.returncode == 0, result.stdout[-2000:] + result.stderr[-2000:]


def photoforge_reads():
    if os.path.exists(REPORT):
        os.remove(REPORT)
    run_step('report', {'PHOTOFORGE_CLIPBOARD_REPORT': REPORT})
    return json.load(open(REPORT))


def to_dib_bytes(image, mode):
    """A packed DIB (no BITMAPFILEHEADER) of `image` as Pillow writes a BMP."""
    buffer = io.BytesIO()
    image.convert(mode).save(buffer, 'BMP')
    return buffer.getvalue()[14:]


def compare(label, expected, got_pixels, got_size, tolerance=0):
    width, height = expected.size
    if tuple(got_size) != (width, height):
        return f'{label}: size {got_size} against {(width, height)}'
    worst = 0
    for index, pixel in enumerate(got_pixels):
        x, y = index % width, index // width
        want = expected.getpixel((x, y))
        worst = max(worst, max(abs(a - b) for a, b in zip(pixel, want)))
    return None if worst <= tolerance else f'{label}: largest difference {worst} (tolerance {tolerance})'


def main():
    problems = []
    expected = sample()

    # 1. PhotoForge writes both forms; read each raw, and through Pillow.
    run_step('both')
    png = get_format(png_format())
    dib = get_format(CF_DIBV5)
    if png is None or dib is None:
        problems.append(f'forms present: PNG={png is not None} CF_DIBV5={dib is not None}')
    else:
        decoded = Image.open(io.BytesIO(png)).convert('RGBA')
        issue = compare('PNG form', expected, list(decoded.getdata()), decoded.size)
        if issue:
            problems.append(issue)
        header = struct.unpack('<IiiHHI', dib[:20])
        print('CF_DIBV5 header: size %d, %dx%d, planes %d, bits %d, compression %d' % header)
    grabbed = ImageGrab.grabclipboard()
    print('ImageGrab on PNG + DIBV5:', getattr(grabbed, 'mode', type(grabbed)), getattr(grabbed, 'size', ''))
    if isinstance(grabbed, Image.Image):
        issue = compare('ImageGrab (both forms)', expected, list(grabbed.convert('RGBA').getdata()), grabbed.size)
        if issue:
            problems.append(issue)

    # 2. PhotoForge writes CF_DIBV5 alone, with alpha: another reader's view of that layout.
    run_step('dibv5_alpha')
    grabbed = ImageGrab.grabclipboard()
    print('ImageGrab on DIBV5 alone:', getattr(grabbed, 'mode', type(grabbed)), getattr(grabbed, 'size', ''))
    if isinstance(grabbed, Image.Image):
        issue = compare('ImageGrab (CF_DIBV5 alone)', expected, list(grabbed.convert('RGBA').getdata()), grabbed.size)
        if issue:
            problems.append(issue)
    else:
        problems.append(f'ImageGrab found no picture on a CF_DIBV5-only clipboard: {grabbed!r}')

    # 3. This script writes; PhotoForge reads.
    opaque = expected.convert('RGB')
    cases = {
        'CF_DIB 24-bit (Pillow BMP)': ({CF_DIB: to_dib_bytes(expected, 'RGB')}, opaque, 0),
        'CF_DIB 32-bit (Pillow BMP, alpha only if the header says it has a mask)': (
            {CF_DIB: to_dib_bytes(expected, 'RGBA')},
            expected if struct.unpack('<I', to_dib_bytes(expected, 'RGBA')[:4])[0] >= 108 or
            struct.unpack('<I', to_dib_bytes(expected, 'RGBA')[16:20])[0] == 3 else opaque,
            0),
        'PNG form with alpha': ({png_format(): (lambda b: (expected.save(b, 'PNG'), b.getvalue())[1])(io.BytesIO())}, expected, 0),
        'PNG preferred over a DIB': ({
            CF_DIB: to_dib_bytes(Image.new('RGB', (3, 3), (9, 9, 9)), 'RGB'),
            png_format(): (lambda b: (expected.save(b, 'PNG'), b.getvalue())[1])(io.BytesIO()),
        }, expected, 0),
    }
    for label, (formats, want, tolerance) in cases.items():
        set_formats(formats)
        report = photoforge_reads()
        if 'error' in report:
            problems.append(f'{label}: PhotoForge said: {report["error"]}')
            continue
        want_rgba = want.convert('RGBA')
        pixels = [tuple(p) for p in report['pixels']]
        issue = compare(label + f' (read as {report["format"]})', want_rgba, pixels, (report['width'], report['height']), tolerance)
        print(f'{label}: PhotoForge read {report["width"]}x{report["height"]} as {report["format"]}',
              'OK' if issue is None else issue)
        if issue:
            problems.append(issue)

    print('problems:', len(problems))
    for problem in problems:
        print('  ', problem)
    return 1 if problems else 0


if __name__ == '__main__':
    sys.exit(main())
