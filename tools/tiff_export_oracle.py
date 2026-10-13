"""Reads the TIFFs PhotoForge wrote with two independent readers (tifffile and Pillow) and checks
the pixels, the directory and the metadata against what was meant.

Usage: python -I tiff_export_oracle.py DUMP_DIR
"""
import os
import sys
sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
from pfenv import CARGO_SH, CRATE_PATH, REPO_PATH, WORK_PATH  # noqa: E402
import json
import os
import sys

import numpy as np
import tifffile
from PIL import Image

d = sys.argv[1]
problems = []
checked = 0


def expect(label, got, want):
    if got != want:
        problems.append('%s: got %r want %r' % (label, got, want))


for name in sorted(f[:-5] for f in os.listdir(d) if f.endswith('.json')):
    info = json.load(open(os.path.join(d, name + '.json')))
    w, h, bits, samples = info['width'], info['height'], info['bits'], info['samples']
    dtype = np.uint8 if bits == 8 else np.dtype('<u2')
    want = np.frombuffer(open(os.path.join(d, name + '.raw'), 'rb').read(), dtype=dtype).reshape(h, w, samples)
    path = os.path.join(d, name + '.tif')

    with tifffile.TiffFile(path) as tf:
        page = tf.pages[0]
        got = page.asarray()
        if got.ndim == 2:
            got = got[..., None]
        if not np.array_equal(got, want):
            problems.append('%s: tifffile pixels differ' % name)
        expect(name + ' compression', int(page.compression), 8 if info['deflate'] else 1)
        expect(name + ' photometric', int(page.photometric), 2)
        expect(name + ' planar', int(page.planarconfig), 1)
        expect(name + ' samples', page.samplesperpixel, samples)
        expect(name + ' bits', page.bitspersample, bits)
        expect(name + ' bits tag', tuple(page.tags['BitsPerSample'].value), (bits,) * samples)
        expect(name + ' predictor', int(page.predictor), 2 if info['deflate'] else 1)
        if info['alpha']:
            expect(name + ' extrasamples', tuple(int(e) for e in page.extrasamples), (2,))
        else:
            expect(name + ' extrasamples', tuple(page.extrasamples), ())
        if info['icc']:
            icc = page.tags['InterColorProfile'].value
            expect(name + ' icc', bytes(icc), open(os.path.join(d, 'p3.icc'), 'rb').read())
        else:
            expect(name + ' no icc', 'InterColorProfile' in page.tags, False)
        if name == 'wide_strips':
            expect(name + ' strips', len(page.dataoffsets), 3)
            expect(name + ' rows per strip', page.rowsperstrip, 97)
        if info['metadata']:
            expect(name + ' make', page.tags['Make'].value, 'Acme')
            expect(name + ' model', page.tags['Model'].value, 'Cam-One')
            xmp = bytes(page.tags['XMP'].value)
            if b'Harbour at dusk' not in xmp:
                problems.append('%s: xmp not carried' % name)
            iptc = page.tags['IPTCNAA'].value if 'IPTCNAA' in page.tags else None
            if iptc is None or bytes(iptc if not isinstance(iptc, tuple) else bytes(iptc))[:5] != bytes([0x1C, 2, 120, 0, 6]):
                problems.append('%s: iptc not carried (%r)' % (name, iptc))

    if samples in (3, 4) and bits == 8:
        with Image.open(path) as im:
            arr = np.asarray(im)
            if not np.array_equal(arr, want):
                problems.append('%s: Pillow pixels differ' % name)
            if info['metadata']:
                ex = im.getexif()
                expect(name + ' pillow make', ex.get(0x010F), 'Acme')
                sub = ex.get_ifd(0x8769)
                expect(name + ' pillow iso', sub.get(0x8827), 400)
                expect(name + ' pillow date', sub.get(0x9003), '2026:10:10 14:03:09')
                expect(name + ' pillow serial', sub.get(0xA431), 'SERIAL-0042')
                expect(name + ' pillow lens', sub.get(0xA434), 'Lens 35mm f/2')
                expect(name + ' pillow exposure', float(sub.get(0x829A)), 1 / 250)
                gps = ex.get_ifd(0x8825)
                expect(name + ' pillow gps ref', gps.get(1), 'N')
                expect(name + ' pillow gps lat', tuple(float(v) for v in gps.get(2)), (48.0, 51.0, 7.0))
    checked += 1

print('checked', checked, 'files; problems:', problems or 'none')
sys.exit(1 if problems else 0)
