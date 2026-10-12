# TIFF

TIFF is the format photographs are handed from one program to another in: Lightroom exports it,
Photoshop and scanners write it, print workflows ask for it. PhotoForge opens a TIFF as a new,
unsaved document with a report of what the file held that it has no place for, and writes the
document, composited, as a TIFF through the same export path as PNG, JPEG and WebP.

Nothing here depends on a TIFF library. The format is read in `src-tauri/src/tiff/`, bounded, as
every other stranger's file is; Deflate is `miniz_oxide`, which the plugin-package reader already
used, and LZW and PackBits are written here. No package is added to the build.

## What is read

| | |
| --- | --- |
| Container | Classic TIFF (32-bit offsets), either byte order. BigTIFF is named and refused |
| Picture | The first full-resolution image of the file; reduced-resolution copies, further pages and masks are counted and reported, not imported |
| Layout | Strips and tiles; chunky and planar (separate) samples |
| Colour | Grey (black or white is zero), RGB, palette, each with alpha. CMYK, YCbCr, CIE L\*a\*b\*, LogLuv, camera mosaics and linear RAW are refused by name |
| Depth | 1, 2, 4 (grey or palette), 8, 16 and 32 bits an integer; 16, 32 and 64 bits a float. Signed integers and other depths are refused |
| Compression | None, PackBits, LZW, Deflate (Adobe and the older code). JPEG, CCITT, JPEG 2000, LZMA, Zstandard and the rest are refused by name |
| Predictor | Horizontal differencing (8, 16, 32 bits) and floating point |
| Alpha | Associated (premultiplied) is made straight; unassociated is kept; an extra channel with no stated meaning is not alpha, and is reported |
| Profile | An embedded RGB profile is applied; a grey picture's profile is reported and not applied |
| Orientation | Turned upright (all eight values), and the tag is then spent |
| Metadata | EXIF (the descriptive tags of the first directory and the Exif, GPS and Interoperability directories), XMP and IPTC, under the same policies as any photograph |

## Colour

The rule is the Photoshop importer's, so one picture gives one result by either door.

| The file | The document |
| --- | --- |
| 8-bit, no profile or an sRGB one | 8-bit sRGB (stays 8-bit; nothing is converted) |
| 8-bit with another RGB profile | High precision, converted through the profile |
| 16- or 32-bit integer, no profile or an sRGB one | High precision, read as sRGB |
| 16- or 32-bit integer with another profile | High precision, converted through the profile |
| Floating point, no profile | High precision, read as **linear light** (what a 32-bit float TIFF is, by convention) |
| Floating point with a profile | High precision, converted through the profile |

Values above 1 and below 0 in a floating-point file are kept. A value that is not a number is
read as zero and infinity as the largest finite value, and the count is reported.

## What is written

One picture, RGB (or RGBA when any pixel is not opaque), 8 or 16 bits a sample, little-endian,
in strips of about a quarter of a megabyte, **Deflate-compressed with horizontal differencing**
(what libtiff and Photoshop write). Alpha is stored unassociated (straight), with
`ExtraSamples = 2`. The colour space chosen for the export is embedded as an ICC profile.

The camera's metadata goes where TIFF puts it: the descriptive tags in the first directory, the
exposure and the place in their own directories, XMP in tag 700 and IPTC in tag 33723, under the
policy chosen for the export. **The maker note is not written**: its offsets are into the block
it came from, and the report says so under every policy.

Not written: layers (the composite is one picture), several pages, tiles, other depths, a
resolution, thumbnails.

## Limits

* The file is admitted from its directory before any pixel is decoded: the picture's size, the
  buffer it becomes, and the largest strip or tile held while it is made are checked against the
  memory budget, and nothing is allocated before that.
* A strip or tile may expand to exactly the rows it holds and no further, whatever its stream
  says; one that ends short is an error and not a band of zeros.
* At most 4,194,304 strips or tiles; 4,096 directory entries; 64 directories walked; a value of
  at most 32 MiB.

## How it was checked

See [phase-15-6b-results.md](phase-15-6b-results.md).
