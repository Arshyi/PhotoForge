# Model format

What PhotoForge accepts as a local inference model, what it records about one,
and what it refuses.

## Accepted formats

**ONNX only.** ONNX is a graph-and-weights container that carries no executable
code, which is the property that matters. Files are recognised by a `.onnx`
extension and a plausible leading protobuf tag; the tag check is weak and is
treated as one — it rejects obvious rubbish early and proves nothing about the
graph inside.

## Refused formats

These extensions are refused by name, with an explanation:

`.pt` `.pth` `.ckpt` `.pkl` `.pickle` `.bin` `.joblib`

They are usually Python pickles. Unpickling runs arbitrary code by design, so
no amount of validation makes loading one safe. PhotoForge does not support the
format rather than attempting to sanitise it.

## Bounds

Checked before a runtime is handed the file, because a model declares its own
tensor shapes and those shapes drive allocation.

| Property | Bound |
| --- | --- |
| File size | 32 bytes to 512 MB |
| Tile size | 32 to 2048 px |
| Tile overlap | Less than half the tile size |
| Scale factor | 1 to 8, and only a super-resolution model may exceed 1 |
| Channels in / out | 1 to 4 |
| Identifier, name, licence, source | 1 to 200 characters |
| Installed models | 64 |

Identifiers may contain only letters, digits, dash, underscore and dot, and may
not begin with a dot, because the identifier becomes a filename and must not
steer anywhere.

## The descriptor

Recorded per model. Everything below the line is read from the file itself, not
from whoever asked to import it.

| Field | Meaning |
| --- | --- |
| `id` | Stable identifier. What a workflow or project refers to. Never a path. |
| `name`, `version` | For display. |
| `architecture` | The family as the publisher describes it. Recorded, never interpreted — PhotoForge special-cases no architecture. |
| `capability` | What the model is for. See the capability list in [local-inference.md](local-inference.md). |
| `colorSpace` | `encodedSrgb` or `linearSrgb`. What the model expects to be fed. |
| `normalization` | `unitRange` for `[0,1]`, `signedUnitRange` for `[-1,1]`. |
| `inputChannels`, `outputChannels` | Three means RGB. Alpha is handled outside the model. |
| `scale` | Integer upscaling factor. One for anything that is not upscaling. |
| `tileSize`, `tileOverlap` | The tile the model is run at, and how much neighbours overlap. |
| `license`, `source` | As supplied by whoever installed it. Recorded, **not verified**. |
| — | — |
| `format` | Determined by inspection. |
| `fileBytes` | Read from the file. |
| `sha256` | Computed from the file's bytes. |

The size and hash are deliberately not accepted from the import request. A
descriptor that disagreed with its file would make the recorded identity a
fiction, and the identity is what ties a saved result to the model that
produced it.

An empty licence is shown as "not stated". A blank field must not read as
permission.

## Where models live

```
%LOCALAPPDATA%\PhotoForge\inference-models\
```

with a `models.json` manifest. The manifest is the authority on what is
installed: a stray `.onnx` file in that directory is not a model until it has
been imported. Removing a model deletes only the file PhotoForge installed,
inside that directory, and never the file you imported from.

The manifest is treated as untrusted. An entry that fails validation is dropped
and the rest survive, so one bad record cannot make every model disappear, and a
corrupt manifest leaves the editor running with no models rather than failing to
start.

## Memory

Before a model is scheduled, PhotoForge estimates weights plus input tensor plus
output tensor plus a fixed multiple of the tile for activations. The estimate is
deliberately high: activations depend on the graph and cannot be read from the
descriptor, and a scheduler that under-estimated would admit work that then
exhausts memory. All of it saturates rather than overflowing, because a hostile
tile size is exactly the input that would otherwise wrap to a small number and
pass an admission check.

## What a model still has to prove at run time

Validation of a descriptor is not validation of a graph. When the model runs:

- the declared tile shape is imposed on the graph, so a model that will not
  accept it fails immediately rather than one tensor into a long render;
- the output shape is compared against what the descriptor promised before any
  value is read;
- the file's size is re-checked, and `resolve_file` re-hashes it, so a model
  swapped since import is refused.
