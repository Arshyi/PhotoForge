"""Reads and writes the Windows clipboard from outside PhotoForge, for clipboard_window.ps1.

Standard library plus Pillow (tools/requirements.txt). Windows only. Every command prints one JSON
document and exits 0 when what it checked holds, 1 when it does not, 2 on a usage error.

  formats                    what the clipboard holds: id, name, bytes, in the clipboard's order
  seq                        the clipboard sequence number (it changes whenever the clipboard does)
  wait-change SEQ [SECONDS]  block until the sequence number is not SEQ (default 15 s)
  check-copy SRC.png MODE    what PhotoForge copied: MODE "layer" expects the picture trimmed to
                             where SRC has any alpha, "merged" expects all of SRC. Checks the PNG
                             form and CF_DIBV5 against it pixel for pixel, the sizes, and that the
                             cloud-clipboard opt-out (CanUploadToCloudClipboard = 0) is present
  set-image SRC.png FORMS    what another program does: FORMS is a comma list of png (with alpha)
                             and dib (24-bit CF_DIB, no alpha)
  set-text TEXT              CF_UNICODETEXT only

The clipboard is the person's: clipboard_window.ps1 saves it first and gives it back.
"""
import ctypes
import ctypes.wintypes as wt
import io
import json
import struct
import sys
import time

user32 = ctypes.WinDLL('user32', use_last_error=True)
kernel32 = ctypes.WinDLL('kernel32', use_last_error=True)
user32.OpenClipboard.argtypes = [wt.HWND]
user32.OpenClipboard.restype = wt.BOOL
user32.EmptyClipboard.restype = wt.BOOL
user32.CloseClipboard.restype = wt.BOOL
user32.EnumClipboardFormats.argtypes = [wt.UINT]
user32.EnumClipboardFormats.restype = wt.UINT
user32.SetClipboardData.argtypes = [wt.UINT, wt.HANDLE]
user32.SetClipboardData.restype = wt.HANDLE
user32.GetClipboardData.argtypes = [wt.UINT]
user32.GetClipboardData.restype = wt.HANDLE
user32.GetClipboardFormatNameW.argtypes = [wt.UINT, wt.LPWSTR, ctypes.c_int]
user32.GetClipboardFormatNameW.restype = ctypes.c_int
user32.RegisterClipboardFormatW.argtypes = [wt.LPCWSTR]
user32.RegisterClipboardFormatW.restype = wt.UINT
user32.GetClipboardSequenceNumber.restype = wt.DWORD
kernel32.GlobalAlloc.argtypes = [wt.UINT, ctypes.c_size_t]
kernel32.GlobalAlloc.restype = wt.HANDLE
kernel32.GlobalLock.argtypes = [wt.HANDLE]
kernel32.GlobalLock.restype = ctypes.c_void_p
kernel32.GlobalUnlock.argtypes = [wt.HANDLE]
kernel32.GlobalSize.argtypes = [wt.HANDLE]
kernel32.GlobalSize.restype = ctypes.c_size_t

CF_UNICODETEXT, CF_DIB, CF_DIBV5 = 13, 8, 17
GMEM_MOVEABLE = 0x0002
NAMES = {1: 'CF_TEXT', 2: 'CF_BITMAP', 3: 'CF_METAFILEPICT', 7: 'CF_OEMTEXT', 8: 'CF_DIB',
         9: 'CF_PALETTE', 13: 'CF_UNICODETEXT', 14: 'CF_ENHMETAFILE', 15: 'CF_HDROP',
         16: 'CF_LOCALE', 17: 'CF_DIBV5'}
HANDLE_FORMATS = {2, 3, 9, 14, 0x80, 0x82, 0x83, 0x8E}


def register(name):
    return user32.RegisterClipboardFormatW(name)


def open_clipboard():
    for _ in range(40):
        if user32.OpenClipboard(None):
            return
        time.sleep(0.05)
    raise RuntimeError('the clipboard is held by another program')


def format_name(number):
    if number in NAMES:
        return NAMES[number]
    buffer = ctypes.create_unicode_buffer(256)
    return buffer.value if user32.GetClipboardFormatNameW(number, buffer, 256) else f'format {number}'


def read_format(number):
    """The bytes of one format, or None."""
    open_clipboard()
    try:
        handle = user32.GetClipboardData(number)
        if not handle:
            return None
        size = kernel32.GlobalSize(handle)
        pointer = kernel32.GlobalLock(handle)
        try:
            return ctypes.string_at(pointer, size)
        finally:
            kernel32.GlobalUnlock(handle)
    finally:
        user32.CloseClipboard()


def list_formats():
    open_clipboard()
    try:
        out = []
        number = 0
        while True:
            number = user32.EnumClipboardFormats(number)
            if not number:
                break
            size = None
            if number not in HANDLE_FORMATS and not 0x200 <= number <= 0x3FF:
                handle = user32.GetClipboardData(number)
                size = kernel32.GlobalSize(handle) if handle else None
            out.append({'id': number, 'name': format_name(number), 'bytes': size})
        return out
    finally:
        user32.CloseClipboard()


def set_formats(items):
    """Replace the clipboard with [(format number, bytes)]."""
    blocks = []
    for number, data in items:
        handle = kernel32.GlobalAlloc(GMEM_MOVEABLE, len(data))
        pointer = kernel32.GlobalLock(handle)
        ctypes.memmove(pointer, data, len(data))
        kernel32.GlobalUnlock(handle)
        blocks.append((number, handle))
    open_clipboard()
    try:
        user32.EmptyClipboard()
        for number, handle in blocks:
            if not user32.SetClipboardData(number, handle):
                raise RuntimeError(f'SetClipboardData({number}) failed')
    finally:
        user32.CloseClipboard()


def decode_dibv5(data):
    """(width, height, [(r, g, b, a)...]) of a 32-bit CF_DIBV5 with colour masks, top row first."""
    (size, width, height, planes, bits, compression, _image, _xp, _yp, _used, _important,
     rmask, gmask, bmask, amask) = struct.unpack('<IiiHHIIiiIIIIII', data[:56])
    if size < 108 or bits != 32 or compression not in (0, 3):
        raise ValueError(f'unexpected CF_DIBV5 header: size {size}, {bits} bits, compression {compression}')
    if compression == 0:
        rmask, gmask, bmask, amask = 0x00FF0000, 0x0000FF00, 0x000000FF, 0xFF000000
    bottom_up = height > 0
    rows = abs(height)
    offset = size
    shifts = {}
    for label, mask in (('r', rmask), ('g', gmask), ('b', bmask), ('a', amask)):
        shifts[label] = (mask & -mask).bit_length() - 1 if mask else None
        if mask and mask >> shifts[label] != 0xFF:
            raise ValueError(f'mask {label} is not 8 bits: {mask:#x}')
    pixels = []
    stride = width * 4
    for row in range(rows):
        source = (rows - 1 - row) if bottom_up else row
        start = offset + source * stride
        for x in range(width):
            value = struct.unpack_from('<I', data, start + x * 4)[0]
            r = (value & rmask) >> shifts['r']
            g = (value & gmask) >> shifts['g']
            b = (value & bmask) >> shifts['b']
            a = (value & amask) >> shifts['a'] if amask else 255
            pixels.append((r, g, b, a))
    return width, rows, pixels


def pixels_of(image):
    """The RGBA pixels of a Pillow image, row by row (getdata is deprecated in newer Pillow)."""
    getter = getattr(image, 'get_flattened_data', None) or image.getdata
    return list(getter())


def expected_pixels(image, mode):
    """The RGBA picture a copy should hold, and its size."""
    rgba = image.convert('RGBA')
    if mode == 'layer':
        box = rgba.getchannel('A').point(lambda a: 255 if a else 0).getbbox()
        if box is None:
            raise ValueError('the source has no visible pixel')
        rgba = rgba.crop(box)
    return rgba


def compare(label, expected, size, pixels):
    """None if equal (colour only where there is alpha), else a sentence."""
    width, height = expected.size
    if tuple(size) != (width, height):
        return f'{label}: {size[0]} x {size[1]} against the expected {width} x {height}'
    want = pixels_of(expected)
    worst = 0
    wrong = 0
    for index, (got, wanted) in enumerate(zip(pixels, want)):
        difference = abs(got[3] - wanted[3])
        if wanted[3]:
            difference = max(difference, *(abs(got[c] - wanted[c]) for c in range(3)))
        if difference:
            wrong += 1
            worst = max(worst, difference)
    return None if wrong == 0 else f'{label}: {wrong} of {len(want)} pixels differ, by up to {worst}'


def main(argv):
    if len(argv) < 2:
        print(__doc__)
        return 2
    command = argv[1]
    if command == 'formats':
        print(json.dumps(list_formats(), indent=1))
        return 0
    if command == 'seq':
        print(user32.GetClipboardSequenceNumber())
        return 0
    if command == 'wait-change':
        start = int(argv[2])
        limit = time.time() + (float(argv[3]) if len(argv) > 3 else 15.0)
        while time.time() < limit:
            if user32.GetClipboardSequenceNumber() != start:
                print(json.dumps({'changed': True, 'seq': user32.GetClipboardSequenceNumber()}))
                return 0
            time.sleep(0.05)
        print(json.dumps({'changed': False}))
        return 1
    if command == 'set-text':
        set_formats([(CF_UNICODETEXT, (argv[2] + '\0').encode('utf-16-le'))])
        print(json.dumps({'set': 'text'}))
        return 0
    if command == 'set-image':
        from PIL import Image
        image = Image.open(argv[2]).convert('RGBA')
        forms = argv[3].split(',')
        items = []
        if 'png' in forms:
            buffer = io.BytesIO()
            image.save(buffer, 'PNG')
            items.append((register('PNG'), buffer.getvalue()))
        if 'dib' in forms:
            buffer = io.BytesIO()
            image.convert('RGB').save(buffer, 'BMP')
            items.append((CF_DIB, buffer.getvalue()[14:]))
        set_formats(items)
        print(json.dumps({'set': forms, 'size': list(image.size)}))
        return 0
    if command == 'check-copy':
        from PIL import Image
        source = Image.open(argv[2])
        mode = argv[3]
        expected = expected_pixels(source, mode)
        problems = []
        report = {'expected': list(expected.size)}
        formats = {item['name']: item for item in list_formats()}
        png = read_format(register('PNG'))
        dib = read_format(CF_DIBV5)
        report['forms'] = [name for name in formats if name in ('PNG', 'CF_DIBV5', 'CF_DIB')]
        if png is None:
            problems.append('no PNG form on the clipboard')
        else:
            decoded = Image.open(io.BytesIO(png)).convert('RGBA')
            report['png'] = list(decoded.size)
            issue = compare('PNG form', expected, decoded.size, pixels_of(decoded))
            if issue:
                problems.append(issue)
        if dib is None:
            problems.append('no CF_DIBV5 form on the clipboard')
        else:
            try:
                width, height, pixels = decode_dibv5(dib)
            except ValueError as error:
                problems.append(str(error))
            else:
                report['dibv5'] = [width, height]
                issue = compare('CF_DIBV5', expected, (width, height), pixels)
                if issue:
                    problems.append(issue)
        cloud = read_format(register('CanUploadToCloudClipboard'))
        report['cloud_opt_out'] = None if cloud is None else int.from_bytes(cloud[:4], 'little')
        if cloud is None:
            problems.append('CanUploadToCloudClipboard is not on the clipboard')
        elif report['cloud_opt_out'] != 0:
            problems.append(f'CanUploadToCloudClipboard is {report["cloud_opt_out"]}, not 0')
        report['problems'] = problems
        print(json.dumps(report))
        return 1 if problems else 0
    print(__doc__)
    return 2


if __name__ == '__main__':
    sys.exit(main(sys.argv))
