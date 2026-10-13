"""Independent TIFF/DNG directory dumper (no decoding): prints IFD0, SubIFDs, Exif, GPS, Interop.

Usage: python -I tiff_dump.py FILE [--values]
Reads only the directories; never touches pixel data. Written separately from the Rust reader so
the two can be compared.
"""
import os
import sys
sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
from pfenv import CARGO_SH, CRATE_PATH, REPO_PATH, WORK_PATH  # noqa: E402
import struct
import sys

TYPES = {1: ("BYTE", 1), 2: ("ASCII", 1), 3: ("SHORT", 2), 4: ("LONG", 4), 5: ("RATIONAL", 8),
         6: ("SBYTE", 1), 7: ("UNDEF", 1), 8: ("SSHORT", 2), 9: ("SLONG", 4), 10: ("SRATIONAL", 8),
         11: ("FLOAT", 4), 12: ("DOUBLE", 8), 13: ("IFD", 4), 16: ("LONG8", 8), 17: ("SLONG8", 8),
         18: ("IFD8", 8)}
NAMES = {
    0x00FE: "NewSubfileType", 0x0100: "ImageWidth", 0x0101: "ImageLength", 0x0102: "BitsPerSample",
    0x0103: "Compression", 0x0106: "Photometric", 0x010E: "ImageDescription", 0x010F: "Make",
    0x0110: "Model", 0x0111: "StripOffsets", 0x0112: "Orientation", 0x0115: "SamplesPerPixel",
    0x0116: "RowsPerStrip", 0x0117: "StripByteCounts", 0x011A: "XResolution", 0x011B: "YResolution",
    0x011C: "PlanarConfig", 0x0128: "ResolutionUnit", 0x0131: "Software", 0x0132: "DateTime",
    0x013B: "Artist", 0x014A: "SubIFDs", 0x02BC: "XMP", 0x8298: "Copyright", 0x829A: "ExposureTime",
    0x829D: "FNumber", 0x83BB: "IPTC-NAA", 0x8649: "PhotoshopResources", 0x8769: "ExifIFD",
    0x8773: "ICCProfile", 0x8825: "GPSIFD", 0x8827: "ISO", 0x9000: "ExifVersion",
    0x9003: "DateTimeOriginal", 0x9004: "DateTimeDigitized", 0x927C: "MakerNote",
    0x9286: "UserComment", 0xA005: "InteropIFD", 0xA430: "CameraOwnerName",
    0xA431: "BodySerialNumber", 0xA434: "LensModel", 0xA435: "LensSerialNumber",
    0xA420: "ImageUniqueID", 0xC612: "DNGVersion", 0xC614: "UniqueCameraModel",
    0xC62F: "CameraSerialNumber", 0xC634: "DNGPrivateData", 0xC635: "MakerNoteSafety",
}


def rd(data, end, fmt, at):
    return struct.unpack_from(end + fmt, data, at)[0]


def read_ifd(data, end, at):
    n = rd(data, end, "H", at)
    entries = []
    for i in range(n):
        e = at + 2 + 12 * i
        tag, typ, count = rd(data, end, "H", e), rd(data, end, "H", e + 2), rd(data, end, "I", e + 4)
        name, width = TYPES.get(typ, ("?%d" % typ, 1))
        size = width * count
        if size <= 4:
            off = e + 8
        else:
            off = rd(data, end, "I", e + 8)
        entries.append((tag, typ, count, size, off))
    nxt = rd(data, end, "I", at + 2 + 12 * n)
    return entries, nxt


def show(data, end, at, label, depth, values, seen):
    if at in seen or at == 0 or at + 2 > len(data):
        return
    seen.add(at)
    entries, nxt = read_ifd(data, end, at)
    print("%s%s @%d: %d entries, next=%d" % ("  " * depth, label, at, len(entries), nxt))
    for tag, typ, count, size, off in entries:
        tname = TYPES.get(typ, ("?%d" % typ, 1))[0]
        inline = size <= 4
        line = "%s  0x%04X %-20s %-8s x%-7d %8d B %s" % (
            "  " * depth, tag, NAMES.get(tag, ""), tname, count, size, "inline" if inline else "@%d" % off)
        if values and size <= 80 and off + size <= len(data):
            raw = data[off:off + size]
            if typ == 2:
                line += "  " + repr(raw.rstrip(b"\0").decode("latin-1"))
            elif typ == 3:
                line += "  " + str(struct.unpack(end + "H" * count, raw))
            elif typ in (4, 13):
                line += "  " + str(struct.unpack(end + "I" * count, raw))
            elif typ == 5:
                line += "  " + str(struct.unpack(end + "I" * (2 * count), raw))
            else:
                line += "  " + raw.hex()
        if off + size > len(data) and size > 4:
            line += "  [OUT OF FILE]"
        print(line)
    for tag, typ, count, size, off in entries:
        ptrs = []
        if tag in (0x8769, 0x8825, 0xA005):
            ptrs = [rd(data, end, "I", off)]
        elif tag == 0x014A:
            ptrs = list(struct.unpack_from(end + "I" * count, data, off))
        label2 = {0x8769: "ExifIFD", 0x8825: "GPSIFD", 0xA005: "InteropIFD", 0x014A: "SubIFD"}.get(tag)
        for p in ptrs:
            show(data, end, p, label2, depth + 1, values, seen)
    if nxt:
        show(data, end, nxt, "next IFD", depth, values, seen)


def main():
    path = sys.argv[1]
    values = "--values" in sys.argv
    with open(path, "rb") as f:
        data = f.read()
    end = "<" if data[:2] == b"II" else ">"
    print("%s: %d bytes, %s-endian, magic %d" % (path, len(data), "little" if end == "<" else "big",
                                                 rd(data, end, "H", 2)))
    show(data, end, rd(data, end, "I", 4), "IFD0", 0, values, set())


if __name__ == "__main__":
    main()
