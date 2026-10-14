// Two small pictures for the clipboard scenario (clipboard_window.ps1), written to
// <work>/validate/inputs. No dependencies: a PNG is written with node's zlib.
//
//   clip-source.png   96 x 64, RGBA. A transparent 8-pixel border all round (so a copy of the layer
//                     is trimmed to 80 x 48), a colour that varies with x and y, and a
//                     half-transparent block, so that straight alpha and the trim are both visible.
//   clip-other.png    40 x 30, opaque: what "another program" puts on the clipboard to be pasted.
//
// The pixel rule is in `source(x, y)` and `other(x, y)`; clipboard_probe.py reads the files back
// with Pillow rather than trusting this rule.
import fs from 'node:fs';
import path from 'node:path';
import zlib from 'node:zlib';
import { fileURLToPath } from 'node:url';

const here = path.dirname(fileURLToPath(import.meta.url));
const repo = path.resolve(here, '..', '..');
const work = process.env.PF_WORK || path.join(repo, '.work');
const directory = path.join(work, 'validate', 'inputs');
fs.mkdirSync(directory, { recursive: true });

const table = new Uint32Array(256).map((_, n) => {
  let c = n;
  for (let k = 0; k < 8; k++) c = c & 1 ? 0xedb88320 ^ (c >>> 1) : c >>> 1;
  return c >>> 0;
});
const crc32 = (buffer) => {
  let c = 0xffffffff;
  for (const byte of buffer) c = table[(c ^ byte) & 0xff] ^ (c >>> 8);
  return (c ^ 0xffffffff) >>> 0;
};
const chunk = (type, data) => {
  const body = Buffer.concat([Buffer.from(type, 'latin1'), data]);
  const length = Buffer.alloc(4);
  length.writeUInt32BE(data.length);
  const crc = Buffer.alloc(4);
  crc.writeUInt32BE(crc32(body));
  return Buffer.concat([length, body, crc]);
};
const png = (width, height, pixel) => {
  const header = Buffer.alloc(13);
  header.writeUInt32BE(width, 0);
  header.writeUInt32BE(height, 4);
  header[8] = 8; // bit depth
  header[9] = 6; // RGBA
  const rows = Buffer.alloc((width * 4 + 1) * height);
  for (let y = 0; y < height; y++) {
    const at = y * (width * 4 + 1);
    rows[at] = 0; // no filter
    for (let x = 0; x < width; x++) {
      const [r, g, b, a] = pixel(x, y);
      rows.set([r, g, b, a], at + 1 + x * 4);
    }
  }
  return Buffer.concat([
    Buffer.from([0x89, 0x50, 0x4e, 0x47, 0x0d, 0x0a, 0x1a, 0x0a]),
    chunk('IHDR', header),
    chunk('IDAT', zlib.deflateSync(rows)),
    chunk('IEND', Buffer.alloc(0))
  ]);
};

const source = (x, y) => {
  if (x < 8 || y < 8 || x >= 88 || y >= 56) return [0, 0, 0, 0];
  const alpha = x >= 60 && x < 80 && y >= 20 && y < 40 ? 128 : 255;
  return [(x * 5) & 255, (y * 9) & 255, (x * 3 + y * 7) & 255, alpha];
};
const other = (x, y) => [(x * 6) & 255, 255 - ((y * 8) & 255), ((x ^ y) * 9) & 255, 255];

fs.writeFileSync(path.join(directory, 'clip-source.png'), png(96, 64, source));
fs.writeFileSync(path.join(directory, 'clip-other.png'), png(40, 30, other));
console.log('wrote clip-source.png (96 x 64) and clip-other.png (40 x 30) in', directory);
