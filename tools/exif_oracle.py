"""Independent check of the EXIF blocks PhotoForge built from real DNGs.

Reads each <stem>.exif.bin with Pillow's own EXIF reader and compares against the values read from
the DNG itself by tiff_dump.py's reader (a different parser written separately). Usage:
python -I exif_oracle.py DUMP_DIR DNG_DIR
"""
import os
import sys
sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
from pfenv import CARGO_SH, CRATE_PATH, REPO_PATH, WORK_PATH  # noqa: E402
import io
import os
import struct
import sys

from PIL import Image

dump_dir, dng_dir = sys.argv[1], sys.argv[2]
problems = []

def read_ascii_tags(path):
    """Tag -> value for the first directory and the Exif directory, straight from the DNG."""
    with open(path, "rb") as f:
        data = f.read()
    end = "<" if data[:2] == b"II" else ">"
    def u16(a): return struct.unpack_from(end + "H", data, a)[0]
    def u32(a): return struct.unpack_from(end + "I", data, a)[0]
    def directory(at):
        out = {}
        for i in range(u16(at)):
            e = at + 2 + 12 * i
            tag, typ, count = u16(e), u16(e + 2), u32(e + 4)
            width = {1: 1, 2: 1, 3: 2, 4: 4, 5: 8, 7: 1, 9: 4, 10: 8}.get(typ, 1)
            size = width * count
            off = e + 8 if size <= 4 else u32(e + 8)
            out[tag] = (typ, count, data[off:off + size], off)
        return out
    ifd0 = directory(u32(4))
    exif = directory(struct.unpack_from(end + "I", ifd0[0x8769][2])[0])
    return ifd0, exif, end

for name in sorted(os.listdir(dump_dir)):
    if not name.endswith(".exif.bin"):
        continue
    stem = name[: -len(".exif.bin")]
    block = open(os.path.join(dump_dir, name), "rb").read()
    ifd0, exif, end = read_ascii_tags(os.path.join(dng_dir, stem + ".DNG"))
    ex = Image.Exif()
    ex.load(b"Exif\x00\x00" + block)

    def expect(label, got, want):
        if got != want:
            problems.append("%s: %s: got %r want %r" % (stem, label, got, want))

    def text(raw):
        return raw.rstrip(b"\0").decode("latin-1")

    expect("Make", ex.get(0x010F), text(ifd0[0x010F][2]))
    expect("Model", ex.get(0x0110), text(ifd0[0x0110][2]))
    expect("Software", ex.get(0x0131), text(ifd0[0x0131][2]))
    expect("DateTime", ex.get(0x0132), text(ifd0[0x0132][2]))
    expect("Artist", ex.get(0x013B), text(ifd0[0x013B][2]))
    expect("Orientation", ex.get(0x0112), struct.unpack(end + "H", ifd0[0x0112][2])[0])
    sub = ex.get_ifd(0x8769)
    expect("DateTimeOriginal", sub.get(0x9003), text(exif[0x9003][2]))
    expect("LensModel", sub.get(0xA434), text(exif[0xA434][2]))
    expect("BodySerialNumber", sub.get(0xA431), text(exif[0xA431][2]))
    expect("ISO", sub.get(0x8827), struct.unpack(end + "H", exif[0x8827][2])[0])
    num, den = struct.unpack(end + "II", exif[0x829A][2])
    expect("ExposureTime", float(sub.get(0x829A)), num / den)
    num, den = struct.unpack(end + "II", exif[0x829D][2])
    expect("FNumber", float(sub.get(0x829D)), num / den)
    # Every Exif-directory tag the source had (but the maker note and pointers) is in the block.
    for tag in exif:
        if tag in (0x927C, 0xA005, 0x8769, 0x8825):
            continue
        if tag not in sub:
            problems.append("%s: Exif tag 0x%04X missing from the block" % (stem, tag))
    # And nothing of the sensor is.
    for tag in (0xC612, 0xC621, 0xC634, 0xC62F, 0x0111, 0x0117):
        if tag in ex:
            problems.append("%s: foreign tag 0x%04X is in the block" % (stem, tag))
    print("%s: %d bytes block; Make=%r Model=%r Lens=%r exif tags %d/%d" % (
        stem, len(block), ex.get(0x010F), ex.get(0x0110), sub.get(0xA434), len(sub), len(exif)))

    xmp_path = os.path.join(dump_dir, stem + ".xmp.xml")
    xmp = open(xmp_path, "rb").read()
    off = ifd0[0x02BC][3]
    src = open(os.path.join(dng_dir, stem + ".DNG"), "rb").read()
    want = src[off: off + ifd0[0x02BC][1]]
    expect("XMP bytes", xmp, want)

if problems:
    print("PROBLEMS:")
    for p in problems:
        print("  ", p)
    sys.exit(1)
print("no problems")
