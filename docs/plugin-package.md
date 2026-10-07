# The `.photoforge-plugin` package

A package is a ZIP file with a fixed, tiny shape. It is read as **hostile input**:
it arrives from a stranger, so the reader trusts none of it — not the sizes, the
names, the offsets or the directory.

## Contents

A package holds a closed list of files and nothing else:

| File | Required | Notes |
| --- | --- | --- |
| `manifest.json` | yes | At most 64 KiB. See `docs/plugin-api.md` for every field. |
| the module | if the manifest has `entry` | The file `entry.path` names, whose SHA-256 must be the `entry.sha256` the manifest states. |
| `README.md` | no | Shown in the plugin manager. |
| `LICENSE` or `LICENSE.txt` | no | Shown in the plugin manager. |

A file that is not on this list is **refused, not skipped**. There is no way to ship
a second module, a script, an executable, an icon or data.

```json
{
  "format": "photoforge-plugin", "formatVersion": 1, "apiVersion": 1,
  "id": "com.example.solarize", "name": "Solarize", "version": "1.0.0",
  "publisher": "Example", "license": "MIT",
  "entry": { "path": "plugin.wasm", "sha256": "<64 hex digits>" },
  "capabilities": ["filter.pixels"],
  "limits": { "memoryMib": 64 },
  "filters": [ ... ]
}
```

* `id`: two to eight dot-separated segments of lowercase letters, digits and hyphens
  (each at most 24 characters, 3–64 in all). `core` is reserved for PhotoForge's own
  operations. Because the id names a directory, a segment that is a Windows device
  name (`con`, `prn`, `aux`, `nul`, `com1`–`com9`, `lpt1`–`lpt9`) is refused anywhere.
* `version`: three numeric segments, `1.0.0`.
* `format` / `formatVersion` / `apiVersion` must match what this build reads. A
  package for a newer interface is refused with a message saying so, not guessed at.
* Unknown fields anywhere in the manifest are an error. A manifest that silently
  ignored a misspelt field would be one that did not mean what it said.

PhotoForge also discovers *component* manifests (planners and restoration engines,
`schemaVersion` 1) that it never runs; see `docs/plugin-specification.md`. The two
formats are unrelated and are told apart by their first field: this one has
`format`, that one has `schemaVersion`.

## What the reader accepts

| Rule | Value |
| --- | --- |
| Whole package | at most 48 MiB |
| Entries | at most 16 |
| Module | at most 32 MiB |
| Manifest / README / licence | at most 256 KiB each (manifest: 64 KiB) |
| Total uncompressed | at most 40 MiB |
| Compression ratio | at most 1000:1 |
| Methods | stored and deflate only |

It refuses, with a specific message, anything else a ZIP can do:

* encryption, ZIP64, split or multi-disk archives, an archive comment (the end
  record must be exactly the last 22 bytes, so a comment cannot hide a second one);
* a symbolic link or a directory entry;
* a name that is not a plain relative path of 1–128 characters made of letters,
  digits, `-`, `_`, `.` and spaces: traversal (`..`), an absolute path or drive, a
  Windows device name, an alternate data stream (`:`), a backslash, a segment ending
  in a dot or space, or a name that appears twice (compared case-insensitively, as
  Windows compares them);
* entries whose data **overlap** each other, the directory or the end record (the
  construction behind "zip bomb" archives), and any size or offset that does not fit
  the file, checked in 64-bit arithmetic before it is used;
* an entry whose inflated size is not exactly the size it declared, or whose CRC-32
  is wrong. Inflation is bounded by the declared size, which is itself bounded, so a
  few kilobytes cannot become gigabytes;
* a local header that disagrees with the central directory;
* a module whose SHA-256 is not the one the manifest states, or that does not begin
  like a WebAssembly module, before anything tries to compile it.

The reader is a deliberately small, purpose-built one (on `miniz_oxide` and
`crc32fast`) rather than a general ZIP library, because everything a general library
supports and a plugin does not need is surface a malicious archive can aim at. The
adversarial fixtures that exercise each rule are in `src-tauri/tests/plugin_package.rs`.

## Identity

A plugin version is identified by its **content hash**: SHA-256 over the manifest
exactly as shipped, a separator, and the module's SHA-256. Documents record it, the
render cache keys on it, and the store names the file by it. Changing a filter's
behaviour *or its declaration* changes it. Two packages with the same version string
and different content are different plugins.

## Installing

Installing is a deliberate act with a preview:

1. **Inspect.** The manager reads the package and shows who made it, the version,
   the declared capabilities in plain words, the package size and hashes, and the
   line **"Not signed."** PhotoForge does not verify who made a plugin; what it limits
   is what a plugin can do. If a version is already installed it says so, and says
   whether this one is older.
2. **Choose** which of the declared capabilities to grant. Nothing is pre-ticked
   beyond what the person ticks; granting something the manifest did not ask for is
   refused.
3. **Install** is all or nothing. The module is compiled (its imports and exports
   checked), and each filter must pass a **self-test**: run twice with identical
   results, and run whole and tiled with identical results (this is what makes a
   locality declaration *checked* rather than trusted). Only then is anything
   written. The install is refused if the file changed between inspecting and
   installing.

Plugins live under `%LOCALAPPDATA%\PhotoForge\plugins\`. The store keeps the
**validated package file itself** as `store\<id>\<content hash>.photoforge-plugin`,
not an unpacked copy: each load goes through the same hostile-input reader as the
install, so a file altered on disk afterwards is refused rather than trusted because
it was once checked, and an install is one atomic file write with no half-unpacked
state. What a person decided (enabled, granted capabilities) is in `state.json`,
written atomically. At most 128 plugins and 16 versions of each.

## Updating

An update installs the new version **beside** the old one, and only then moves the
"active" pointer. A document made with the old version keeps finding it until the
person removes it, and is never quietly rendered by a different build. If only a
different version is installed, the document's plugin is reported as unavailable
with the message "This needs X 1.0.0; version 2.0.0 is installed instead. PhotoForge
will not render with a different version unless you install the exact one."

Capabilities are **not inherited** by an update: the grant is whatever the person
chose in that install. Withdrawing a grant, turning a plugin off, or removing one
version or the whole plugin are each separate actions in the manager.

There is no auto-update, no update check, no marketplace and no download. PhotoForge
never fetches a plugin; the person brings the file.

## Building one

`cargo run --example build_example_plugins` builds the packages in `plugins/examples/`
(each holds a `manifest.json`, an optional `plugin.wat` written in WebAssembly text,
and a README). Output is deterministic. The packages are for people writing plugins to
read, install and change; they are not part of the installer.
