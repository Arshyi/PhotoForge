# Metadata: EXIF, XMP and IPTC

Until 15.3 an exported file carried none of a photograph's metadata and the interface said so
("Camera EXIF/GPS is not exported"). That is safe for privacy and destructive for an archive, and
it was the same for every export whether the person wanted it or not. Now the metadata is read
when a file is opened, kept with the document, and written on export under a **policy the person
chooses**, and every export says what it kept, what it removed and what it could not store.

## The three policies

| Policy | What is written | Default |
| --- | --- | --- |
| **Remove location and serial numbers** | Everything that describes the picture: camera make and model, lens, exposure, date, title, keywords, rating, caption, credit, copyright. Not the place, the owner's name, serial numbers, the camera maker's private notes, an XMP packet that holds a preview | **yes** |
| **Keep everything** | What the file carried, corrected for the edit (below) | |
| **Remove everything** | No EXIF, XMP or IPTC | |

The default is the private one because this is a private tool and a photograph is usually
exported to be shared. The choice is remembered and is shown, with what the file carries, in the
**Metadata** card of the Color panel, in the batch panel, and in the Photoshop and image export
paths alike.

Under every policy, because they are about being correct and not about privacy:

* **The orientation is turned.** A phone or camera stores the picture as the sensor saw it with
  a tag saying how to turn it. Every other program applies the tag; PhotoForge now does, when a
  file is opened whole, and the exported tag says "upright". (Before this, a portrait photograph
  opened on its side.) A region or reduced copy of a very large file is in the file's own
  orientation, so the tag is carried with it and the picture comes out right in other programs.
  `Remove everything` still writes the orientation in that one case, because a sideways picture
  is a defect and not a privacy choice.
* **The embedded thumbnail is removed.** It is a small copy of the picture as it was, and after a
  crop it shows the part that was cropped away.
* **The size is the new size** (EXIF PixelXDimension and PixelYDimension), and the colour space
  tag says "uncalibrated" when the file carries a profile other than sRGB.

## Where each block can be written

| | EXIF | XMP | IPTC |
| --- | --- | --- | --- |
| JPEG | APP1 | APP1 | APP13 (Photoshop resource 0x0404) |
| PNG | `eXIf` | `iTXt` "XML:com.adobe.xmp" | no standard place; the report says so |
| WebP | `EXIF` | `XMP ` | no place; the report says so |
| PSD / PSB | image resource 1058 | 1060 | 1028 |
| TIFF | the descriptive tags of the first directory, and the Exif and GPS directories (pointers 34665, 34853) | tag 700 | tag 33723 |
| OpenEXR | no place; the report says so | no place | no place |
| `.photoforge` project | entries beside the layers; older versions ignore them | | |

A JPEG segment holds 64 KiB, so a larger EXIF block or XMP packet is reported as not stored.
Extended XMP (a packet split over several segments) is not carried.

## What "remove" means

* **EXIF** is edited **in place**. The block keeps its layout, so offsets inside vendor data that
  PhotoForge does not understand stay valid, and what is removed is **overwritten with zeros
  where it lies**, not merely unlinked. A test searches the exported bytes for the removed
  values.
* **XMP** is not rewritten. It is XML under prefixes the writer chose, and an edit that missed a
  spelling would leave behind what it meant to remove. The packet is asked only whether it names
  a place (by local name, so a renamed prefix does not hide it), an owner or a serial number, or
  carries a preview; if so it is dropped whole, and the report says why. Otherwise it is kept as
  it was.
* **IPTC** is rebuilt without the location datasets (content location, city, sub-location,
  state, country code and name); the caption, keywords and copyright stay.
* **A block that cannot be walked safely** (a pointer past the end, a directory that loops) is
  dropped whole and the report says so. A block that cannot be understood cannot be promised to
  be free of anything.

## DNG sources

A DNG is a TIFF, so its metadata is not a block to cut out: the camera's tags sit in the first
directory among the picture's own (strip offsets, colour matrices, the CFA description), the
exposure in the Exif directory it points at, the place in the GPS directory, and XMP and IPTC are
tags of the first directory. When a DNG is opened, whole or as a region, its EXIF is therefore
**built**: a new, small TIFF holding

* the descriptive tags of the first directory: description, make, model, orientation,
  resolution, software, date, artist and copyright;
* every field of the Exif directory (exposure, the date taken, the lens, the body and lens serial
  numbers, ...), of the GPS directory, and of the Interoperability directory, each copied byte
  for byte under its own type and count, in the file's byte order.

XMP (tag 700) and IPTC (tag 33723, or the Photoshop resources of tag 34377) are read as the
blocks they are. From there the three policies apply exactly as they do to a JPEG, and the report
says what each did.

What is **not** carried, and is said in the report under every policy that writes EXIF:

* **The maker note** (Exif tag 0x927C) and a DNG's **private data** (tag 0xC634, where Adobe's
  converter keeps the camera's own maker note). Their values hold offsets into the file they came
  from; moved into a block of their own they would point at nothing, and a block that points at
  nothing is worse than no block. The Metadata card still lists them as present.
* A field larger than 16 KiB, or one that does not fit the block's 60,000-byte budget (which keeps
  it inside what one JPEG segment holds).

And silently, because they describe the sensor and not the photograph: the picture's own
structure and the DNG-specific tags (colour matrices, CFA pattern, linearisation tables, the
DNG's own camera serial number tag, ...). The serial number in the Exif directory *is* carried,
and removed under the private policy as it is for any file.

A TIFF *picture* (see [tiff.md](tiff.md)) goes through the same code, from its directories read
by seeking: its EXIF, XMP and IPTC are built exactly as a DNG's are, and the orientation is
applied to its pixels (a TIFF is opened upright), so the tag is spent. Written, the same
structure is taken apart the other way: the cleaned block is split into the directories a TIFF
keeps them in, every value converted to the file's little-endian order, and the maker note is
not written (its offsets are into the block it came from; the report says so).

**The orientation of a DNG is read, not applied.** Development leaves the sensor's own way up, so a
portrait DNG still opens on its side, as a region of a very large JPEG does; the tag travels with
the pixels (and under *Remove everything* too, for the same reason), so the exported file comes
out upright in other programs. Turning the developed picture upright is not done.

## What is not carried, under any policy

PNG text chunks, JPEG comments, JFIF and PNG resolution (so the DPI of a print is not written by
export), vendor data other than the maker note, and GPS held *inside* a maker note under
**Keep everything**: a maker note is carried as the block the camera wrote and is not parsed.
A DNG's maker note is not carried at all (above).

## How it was checked

* **Unit tests** for each block and container: a TIFF structure built by hand with a place, a
  maker note, serial numbers and a thumbnail is cleaned and the bytes searched; every truncation
  and every changed byte of it, of an IPTC stream, and of each container, returns or refuses and
  never panics; a declared length of four gigabytes costs nothing.
* **The whole path**: a JPEG with all three blocks is opened, exported under each policy, and
  the files read again; through the real command boundary, with the default and with a bad
  policy name.
* **Real camera files.** 13 JPEGs from the MIT-licensed Piexif test suite (Canon, Casio,
  Olympus, Panasonic, Pentax/Ricoh, Sigma, Sony; three have a GPS directory), each cleaned and
  then read with Pillow, a parser written elsewhere: GPS, maker note, serial numbers and
  thumbnail gone, every other tag with its original value, none refused.
* **Mutation checks**: eight breakages (the place left in the bytes, the thumbnail left, serial
  numbers kept, orientation not applied, a keep-everything default, a place-naming XMP kept,
  IPTC places kept, an unplaceable block reported as kept) are each caught.
* **DNG sources** (15.6a): a synthetic DNG with the structure a converter writes is opened
  through the real command boundary and exported under each policy, and the JPEGs read again
  (7 tests); the hand-built TIFFs behind them, little- and big-endian, are truncated at every
  length and changed at every byte, and whatever block comes out is one the cleaner walks and is
  within budget. Against three real files (a Canon EOS 5D Mark III frame written by Adobe DNG
  Converter 9.8, uncompressed, lossless and lossy): the block built from each was read with
  Pillow, a separate parser, and every one of its 30 Exif tags and the IFD0 tags equal the
  values read out of the DNG by a second reader written for the purpose; the exports under the
  three policies were read with Pillow as well, and the serial numbers are not in the private
  or the bare file's bytes.

## What was not checked

* The result in **Lightroom, Photoshop, ExifTool or a phone's gallery**: the independent reader
  was Pillow, and Pillow is lenient.
* A file with a **maker note that holds a place** under *Remove location*: the maker note is
  dropped whole, which is the safe answer, and no sample had one to prove it.
* **Orientation on a very large file** opened whole: the rotation makes one extra copy of the
  pixels, which the admission planner did not price separately.
* The interface in a real window: the selector and the notice are covered by component tests,
  not driven in the packaged application.
* **DNGs from other makers.** One real converter's file stands for all: a DNG a camera writes
  itself may keep its maker note in the Exif directory (it is left behind and reported, and the
  tests cover that structure only in a synthetic file), carry a GPS directory or IPTC (no real
  sample had either), or be big-endian (only synthetic ones are). IPTC stored as 32-bit
  integers byte-swapped, which some writers do in little-endian files, is not undone; such a
  stream is unreadable and is not carried.

## Not done

Turning a developed DNG upright; XMP rewriting (so a packet that
names a place is dropped whole, including its title and keywords); extended XMP; PNG/WebP IPTC
in a legacy text chunk; writing a resolution; editing metadata in the interface.
