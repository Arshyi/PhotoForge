"""Strict, independent structural check of PSD/PSB files (read-only; not part of PhotoForge).

usage: python -I psd_check.py [--loose] FILE...   prints one line per file: OK or the first violation.
Checks: header, section lengths nest exactly, every layer record's tagged blocks end exactly where
the record's extra data does, every block has a signature and (unless --loose) a length that is a
multiple of 4, channel data fills the layer-information section, folder/divider records balance,
every layer has the Unicode-name and layer-id blocks (unless --loose), and the merged image's
compression code is valid.
"""
import os
import sys
sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
from pfenv import CARGO_SH, CRATE_PATH, REPO_PATH, WORK_PATH  # noqa: E402
import struct
import sys

LIMIT = 1 << 30
WIDE = {b'LMsk', b'Lr16', b'Lr32', b'Layr', b'Mt16', b'Mt32', b'Mtrn', b'Alph', b'FMsk',
        b'lnk2', b'FEid', b'FXid', b'PxSD'}


def check(path, strict):
    data = open(path, 'rb').read(LIMIT)
    assert data[:4] == b'8BPS', 'not 8BPS'
    version = struct.unpack('>H', data[4:6])[0]
    assert version in (1, 2), f'version {version}'
    big = version == 2
    ln = 8 if big else 4
    q = '>Q' if big else '>I'
    channels, height, width, depth, mode = struct.unpack('>HIIHH', data[12:26])
    pos = 26
    n = struct.unpack('>I', data[pos:pos + 4])[0]
    pos += 4 + n
    n = struct.unpack('>I', data[pos:pos + 4])[0]
    pos += 4 + n
    lm_len = struct.unpack(q, data[pos:pos + ln])[0]
    pos += ln
    lm_end = pos + lm_len
    assert lm_end <= len(data), 'layer and mask section runs past the file'
    li_len = struct.unpack(q, data[pos:pos + ln])[0]
    pos += ln
    layers = 0
    if li_len:
        li_end = pos + li_len
        assert li_end <= lm_end, 'layer info runs past the section'
        layers = parse_layers(data, pos, li_end, big, strict)
        pos = li_end
    gl = struct.unpack('>I', data[pos:pos + 4])[0]
    pos += 4 + gl
    while pos + 12 <= lm_end:
        assert data[pos:pos + 4] in (b'8BIM', b'8B64'), f'additional block signature at {pos}'
        key = data[pos + 4:pos + 8]
        wide = big and key in WIDE
        sl = 8 if wide else 4
        size = struct.unpack('>Q' if wide else '>I', data[pos + 8:pos + 8 + sl])[0]
        body = pos + 8 + sl
        if key in (b'Lr16', b'Lr32'):
            layers = parse_layers(data, body, body + size, big, strict)
        pos = body + size
        assert pos <= lm_end + 3, f'additional block {key!r} runs past the section'
    comp = struct.unpack('>H', data[lm_end:lm_end + 2])[0]
    assert comp in (0, 1, 2, 3), f'merged image compression {comp}'
    return f'v{version} {width}x{height} d{depth} layers={layers}'


def parse_layers(data, pos, end, big, strict):
    ln = 8 if big else 4
    q = '>Q' if big else '>I'
    count = abs(struct.unpack('>h', data[pos:pos + 2])[0])
    pos += 2
    records = []
    channel_bytes = 0
    for index in range(count):
        top, left, bottom, right, nch = struct.unpack('>iiiiH', data[pos:pos + 18])
        pos += 18
        assert bottom >= top and right >= left, f'record {index}: inverted rectangle'
        for _ in range(nch):
            size = struct.unpack(q, data[pos + 2:pos + 2 + ln])[0]
            channel_bytes += size
            pos += 2 + ln
        assert data[pos:pos + 4] == b'8BIM', f'record {index}: blend signature'
        blend = data[pos + 4:pos + 8]
        pos += 8
        pos += 4
        extra = struct.unpack('>I', data[pos:pos + 4])[0]
        pos += 4
        extra_end = pos + extra
        assert extra_end <= end, f'record {index}: extra data runs past the section'
        mlen = struct.unpack('>I', data[pos:pos + 4])[0]
        p = pos + 4 + mlen
        assert mlen in (0, 20, 36), f'record {index}: mask data length {mlen}'
        blen = struct.unpack('>I', data[p:p + 4])[0]
        p += 4 + blen
        nlen = data[p]
        p += 1 + nlen
        p += (4 - (1 + nlen) % 4) % 4
        keys = {}
        while p < extra_end:
            assert data[p:p + 4] in (b'8BIM', b'8B64'), f'record {index}: block signature at {p}'
            key = data[p + 4:p + 8]
            wide = big and key in WIDE
            sl = 8 if wide else 4
            size = struct.unpack('>Q' if wide else '>I', data[p + 8:p + 8 + sl])[0]
            if strict:
                assert size % 4 == 0, f'record {index}: {key!r} holds {size} bytes'
            keys[key] = data[p + 8 + sl:p + 8 + sl + min(size, 16)]
            p += 8 + sl + size
        assert p == extra_end, f'record {index}: blocks end at {p}, record at {extra_end}'
        records.append((blend, keys))
        pos = extra_end
    depth = 0
    for index, (blend, keys) in reversed(list(enumerate(records))):
        section = keys.get(b'lsct')
        if section is not None:
            kind = struct.unpack('>I', section[:4])[0]
            # Read from the top of the stack down: a folder opens a group, its divider closes it.
            if kind in (1, 2):
                depth += 1
            elif kind == 3:
                depth -= 1
                assert depth >= 0, f'record {index}: divider with no folder'
        if strict:
            assert b'luni' in keys and b'lyid' in keys, f'record {index}: no name or id block'
    assert depth == 0, f'{depth} dividers without folders'
    assert pos + channel_bytes <= end, 'channel data runs past the layer information'
    left = end - (pos + channel_bytes)
    assert left < 4, f'{left} bytes after the channels'
    return count


if __name__ == '__main__':
    strict = '--loose' not in sys.argv
    ok = bad = 0
    for path in [a for a in sys.argv[1:] if not a.startswith('--')]:
        name = path.replace('\\', '/').rsplit('/', 1)[-1]
        try:
            print('OK  ', name, check(path, strict))
            ok += 1
        except Exception as error:  # noqa: BLE001
            print('FAIL', name, type(error).__name__, error)
            bad += 1
    print(f'{ok} ok, {bad} failed')
