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

## What is not carried, under any policy

PNG text chunks, JPEG comments, JFIF and PNG resolution (so the DPI of a print is not written by
export), vendor data other than the maker note, and GPS held *inside* a maker note under
**Keep everything**: a maker note is carried as the block the camera wrote and is not parsed.
RAW (DNG) sources: their EXIF is not read yet.

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

## What was not checked

* The result in **Lightroom, Photoshop, ExifTool or a phone's gallery**: the independent reader
  was Pillow, and Pillow is lenient.
* A file with a **maker note that holds a place** under *Remove location*: the maker note is
  dropped whole, which is the safe answer, and no sample had one to prove it.
* **Orientation on a very large file** opened whole: the rotation makes one extra copy of the
  pixels, which the admission planner did not price separately.
* The interface in a real window: the selector and the notice are covered by component tests,
  not driven in the packaged application.

## Not done

TIFF; metadata of DNG sources; XMP rewriting (so a packet that names a place is dropped whole,
including its title and keywords); extended XMP; PNG/WebP IPTC in a legacy text chunk; writing a
resolution; editing metadata in the interface.
