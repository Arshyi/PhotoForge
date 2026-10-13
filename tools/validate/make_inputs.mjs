// Inputs for the RAW parts of the validation scripts, written to <work>/validate/inputs:
//   landscape.dng   a copy of a sample DNG, byte for byte
//   portrait.dng    the same file with its orientation tag (274) rewritten to 6, a portrait
//
// Usage:  node tools/validate/make_inputs.mjs [path-to-a-dng]
// With no argument it looks for the Canon sample the RAW tests use, fetched by
// scripts/fetch-raw-fixtures.ps1 into <repo>/raw-fixtures. The tag is rewritten in the copy only; the
// original is never modified.
import fs from 'node:fs';
import path from 'node:path';
import { fileURLToPath } from 'node:url';

const here = path.dirname(fileURLToPath(import.meta.url));
const repo = path.resolve(here, '..', '..');
const work = process.env.PF_WORK || path.join(repo, '.work');
const source =
  process.argv[2] || path.join(repo, 'raw-fixtures', '5G4A9394-compressed-lossless.DNG');
if (!fs.existsSync(source)) {
  console.error(`No DNG at ${source}. Run scripts/fetch-raw-fixtures.ps1, or pass a path to a DNG.`);
  process.exit(1);
}
const directory = path.join(work, 'validate', 'inputs');
fs.mkdirSync(directory, { recursive: true });

const original = fs.readFileSync(source);
fs.writeFileSync(path.join(directory, 'landscape.dng'), original);

const portrait = Buffer.from(original);
if (portrait.toString('latin1', 0, 2) !== 'II') throw new Error('expected a little-endian file');
const directoryOffset = portrait.readUInt32LE(4);
const count = portrait.readUInt16LE(directoryOffset);
let patched = false;
for (let index = 0; index < count; index++) {
  const at = directoryOffset + 2 + index * 12;
  if (portrait.readUInt16LE(at) === 274) {
    portrait.writeUInt16LE(6, at + 8);
    patched = true;
    break;
  }
}
if (!patched) throw new Error('no orientation tag in the first directory');
fs.writeFileSync(path.join(directory, 'portrait.dng'), portrait);
console.log(
  'wrote',
  fs.readdirSync(directory).map((name) => `${name} ${fs.statSync(path.join(directory, name)).size}`)
);
