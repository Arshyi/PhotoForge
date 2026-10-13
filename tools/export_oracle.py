"""Reads the JPEGs PhotoForge wrote from a real DNG with Pillow and reports what each carries.
Usage: python -I export_oracle.py DUMP_DIR
"""
import os
import sys
sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
from pfenv import CARGO_SH, CRATE_PATH, REPO_PATH, WORK_PATH  # noqa: E402
import os
import sys
from PIL import Image

problems = []
d = sys.argv[1]
for policy in ("keep_all", "remove_private", "remove_all"):
    path = os.path.join(d, "export-%s.jpg" % policy)
    with Image.open(path) as im:
        ex = im.getexif()
        sub = ex.get_ifd(0x8769)
        xmp = im.info.get("xmp") or im.info.get("XML:com.adobe.xmp")
        tags = {k: ex.get(k) for k in (0x010F, 0x0110, 0x0112, 0x013B)}
        print("%-15s %dx%d  Make=%r Model=%r Orientation=%r Artist=%r  ExifIFD tags=%d  Lens=%r Serial=%r  XMP=%s" % (
            policy, im.width, im.height, tags[0x010F], tags[0x0110], tags[0x0112], tags[0x013B],
            len(sub), sub.get(0xA434), sub.get(0xA431), "%d B" % len(xmp) if xmp else "none"))
        if policy == "keep_all":
            if tags[0x0110] != "Canon EOS 5D Mark III" or sub.get(0xA431) != "063024001964" or not xmp:
                problems.append("keep_all lost something")
        elif policy == "remove_private":
            if tags[0x0110] != "Canon EOS 5D Mark III" or sub.get(0xA431) is not None or xmp:
                problems.append("remove_private is wrong")
            if sub.get(0xA434) != "EF70-200mm f/2.8L IS II USM":
                problems.append("remove_private lost the lens")
        else:
            if tags[0x010F] or len(sub) or xmp:
                problems.append("remove_all wrote metadata")
    raw = open(path, "rb").read()
    for needle in (b"063024001964", b"0000413819"):
        present = needle in raw
        if policy != "keep_all" and present:
            problems.append("%s: %r is in the file" % (policy, needle))
if problems:
    print("PROBLEMS", problems)
    sys.exit(1)
print("no problems")
