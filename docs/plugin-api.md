# Plugin API (host interface version 1)

This is everything a plugin can ask of PhotoForge and everything PhotoForge hands
it. It is small on purpose. If a thing is not on this page, a plugin cannot do it.

A plugin has two halves:

* **Declarations** — data in `manifest.json`: filters, commands, panels, tools,
  parameters. PhotoForge reads these and draws the interface itself. Nothing in a
  declaration is code.
* **One WebAssembly module** — optional, and the only thing that runs. It
  implements the plugin's *filters*: a function from pixels and numbers to pixels.

There is no scripting, no callback into the application, no event hook, and no way
for a plugin to draw its own interface or receive a pointer event.

## The host interface

A module is linked against exactly one import:

```wat
(import "photoforge" "log" (func (param i32 i32 i32)))   ;; level, pointer, length
```

`level` is 0 debug, 1 info, 2 warn, anything else error. The text is read from the
module's own memory, cut to 240 bytes, stripped of control characters, and
dropped silently if the range is outside the memory or if 32 lines have already
been logged in the call. A plugin cannot fail a render by logging badly.

A module that imports anything else — including any `wasi_*` function — is refused
**when it is installed**, naming the import. There is no clock, no randomness, no
environment, no filesystem and no socket to import.

## What a module exports

| Export | Type | Meaning |
| --- | --- | --- |
| `memory` | memory | Its one linear memory. |
| `pf_abi_version` | `() -> i32` | Must return `1`. |
| `pf_alloc` | `(i32) -> i32` | Returns a pointer to that many bytes in `memory`. |
| `pf_filter` | 15 × `i32` → `i32` | Runs one filter on one window. `0` is success. |

A module that lacks any of these, exports them with another signature, or declares
more memories or tables than the limits below, is refused at install. A start
function runs at every instantiation, so one that never returns is stopped by the
same fuel and time limits as any other code (and fails the install self-test).

`pf_filter` takes, in order:

```
filter_index                      which of the manifest's filters (position in "filters")
in_ptr,  in_x, in_y, in_w, in_h   the input window, and where it sits in the image
out_ptr, out_x, out_y, out_w, out_h   the output rectangle, and where it sits
image_w, image_h                  the size of the whole image
params_ptr, params_len            parameters: params_len f64 values, little-endian
```

The host calls `pf_alloc` for the input, the output and the parameters, copies the
input and the parameters in, calls `pf_filter`, and reads the output rectangle back.
The module never receives a pointer into host memory, and the host never trusts a
pointer the module returns: `pf_alloc` returning zero, a negative number, or a range
outside the memory is a trap, not a crash.

A non-zero return from `pf_filter` is reported to the person as the plugin's own
error code. A trap, running out of fuel, running out of time, or exceeding memory is
reported as that, naming the plugin. In every case the document is untouched.

### Pixels

Pixels are **RGBA, four little-endian `f32`s per pixel, linear sRGB, straight (not
premultiplied) alpha**, row-major, with no padding. Values above 1.0 are legal —
documents are linear float and hold highlights — and a filter should not clamp them
unless that is what the filter is for.

The output must be exactly `out_w × out_h × 16` bytes, every value finite, and alpha
within a small tolerance of `[0, 1]`. A NaN, an infinity or an alpha outside range is
**refused, not repaired**: the call fails and says why, so a plugin cannot quietly
write garbage into a document.

### Parameters

`params_ptr` points at `params_len` `f64` values, in the order the filter declares
its parameters. Every kind arrives as a number: a `number` or `integer` as itself, a
`bool` as 0 or 1, a `choice` as the index into its options. The host checks each
against its declaration before the call; a module can rely on the values being in
range and finite.

### Windows, and what `locality` promises

A filter declares how far its output reaches:

| `locality` | Meaning | How it is run |
| --- | --- | --- |
| `{"kind": "pointwise"}` | An output pixel depends on the same input pixel only. | In tiles of 256 × 256. `in` and `out` are the same rectangle. |
| `{"kind": "local", "radius": R}` | An output pixel depends on input pixels within `R` of it (`R` ≤ 256). | In tiles; the input window is the output tile grown by `R` on each side, clipped to the image. |
| `{"kind": "global"}` | An output pixel may depend on any input pixel. | One call, with the whole image as input and output. |

Clipping matters: at the image edge the input window is *smaller* than the output
tile plus margin, and the module must treat pixels outside `[0, image_w) × [0,
image_h)` according to what the filter means (the examples clamp to the edge). The
module is told where its window sits so it can tell.

The declaration is **checked, not trusted**. When a plugin is installed — and
whenever it is tested from the manager — the host runs each filter on a test image
once as a whole and once tiled, and refuses the plugin if the two results are not
bit-identical. A filter that says `pointwise` and reads its neighbour fails this. A
filter that depends on something it should not (its position in the image, in a way
that differs between whole and tiled) fails it too.

A `global` filter on a large image needs room for the whole image in the module's
memory. If the image does not fit in the memory the plugin may use (below), the
filter is **refused before any pixel is copied**, with the number of megabytes it
would need, and the person is told a smaller image or region would work. It is never
run on a part of the image as if that were the whole.

### Determinism

A filter is a **pure function of its input pixels and its parameters**. Each call is
a fresh instance: nothing survives from one tile to the next, or from one render to
the next, and the module cannot tell which tile it is, how many there are, or what
time it is. Floating-point NaN bit patterns are canonicalised and relaxed SIMD is
off, so the same input gives the same output. This is what lets PhotoForge cache and
tile a plugin's result and reproduce it on another machine.

## Limits a module cannot change

| Limit | Value | Notes |
| --- | --- | --- |
| Memory per call | what the manifest asks (`limits.memoryMib`), default 256 MiB, at most 2048 MiB, at most **a quarter of the memory budget** | A plugin cannot talk its way past a small machine. The initial size counts, so a module that declares gigabytes cannot start. |
| Work (fuel) per call | 10,000,000 + 20,000 per pixel processed, capped at 500,000,000,000 | About one unit per WebAssembly instruction. Not requestable. |
| Time per call | 20 s + 2.5 s per megapixel, capped at 15 min | An epoch interrupt at loop back-edges; resolution 10 ms. Cancelling a render uses the same mechanism. |
| Instances | 1 per call | |
| Memories / tables | 1 / 4; 10,000 table elements | |
| Log | 32 lines per call, 240 bytes each | |

The limits are *constants a plugin cannot raise*: a limit the plugin sets is a limit
the plugin can remove, so the manifest may ask for memory (and gets no more than the
ceiling) and may not ask for time or work at all. The memory a call may use follows
the machine through the resource budget (`docs/resource-management.md`); the time
and work follow the size of the image. Nothing is described as dynamic beyond that.

## Declarations

### Parameters

```json
{ "id": "radius", "title": "Radius (pixels)", "type": "integer", "min": 1, "max": 4, "default": 1 }
```

Types: `number` (`min`, `max`, `default`, optional `step`), `integer`, `bool`
(`default`), `choice` (`options`, 2–32 of them, and an index `default`). A field that
does not belong to the type is refused — a misspelt `"maxx"` is an error, not an
ignored word. Ids are lowercase letters, digits and underscores; titles are 1–60
plain characters. At most 32 parameters per filter or command.

### Filters

```json
{ "id": "box_blur", "title": "Box blur", "locality": { "kind": "local", "radius": 4 },
  "parameters": [ ... ] }
```

At most 32 filters; titles 1–60 characters, descriptions up to 300. Filters need the
`filter.pixels` capability *and* an entry module, and an entry module with no filters
is refused (it would never run). `deterministic` defaults to true; a filter that
declares `false` is **refused**, because interface 1 supports only pure functions of
pixels and parameters. A `local` radius must be 1–256: use `pointwise` for none and
`global` for more.

### Commands

A command is a **list of registered operations** — the same operations the Layers
panel, the macro editor and the planner use (`docs/automation.md`):

```json
{ "id": "add_inspected_group", "title": "Add a group called Inspected",
  "parameters": [ { "id": "count", "title": "How many", "type": "integer", "min": 1, "max": 5, "default": 1 } ],
  "steps": [ { "op": "core.layer.add_group", "params": { "name": "Inspected" } } ] }
```

In a step's `params`, the only substitutions are `{"$param": "count"}` (a value the
person gives when running the command) and `"$self"` (this plugin's own id, as the
value of `plugin`). Any other string beginning with `$` is refused. There are no
expressions, no arithmetic, no conditions, no loops. At most 50 steps of at most 4
KiB each, nested at most 8 deep, and every step's operation is checked against the
registry when the manifest is read.

Two further rules keep a command from spending authority it was not given:

* A command may not issue **`core.layer.delete`, `core.document.flatten`,
  `core.layer.merge_down` or `core.layer.set_locked`**. They destroy work or change
  what protects it; a person can still do any of them by hand.
* `core.plugin.apply_filter` in a command must say `"plugin": "$self"` and name a
  filter the plugin declares. A plugin runs its own filters and no one else's.

Running a command sends its steps to the transaction engine **as a plugin** (see
below): one undo entry, all or nothing, layer locks respected, and refused unless the
plugin was granted `document.operations`.

### Panels

```json
{ "id": "facts", "title": "Document facts",
  "rows": [ { "kind": "text", "text": "..." },
            { "kind": "fact", "label": "Canvas width", "fact": "canvas_width" },
            { "kind": "command", "label": "Add group", "command": "add_inspected_group" } ] }
```

A panel is rows over a closed set. `fact` is one of `layer_count`,
`pixel_layer_count`, `canvas_width`, `canvas_height`, `precision`,
`active_layer_name`, `active_layer_kind`, `active_layer_opacity`; a panel cannot ask
for anything else, and cannot read pixels. At most 8 panels of 40 rows. Needs
`ui.panel`, and `document.read` for facts.

### Tools

A tool is a **menu entry that runs a command**. It cannot receive pointer events,
draw on the canvas, or add a gesture. At most 16. Needs `ui.tool`.

## Capabilities

The whole list:

| Capability | Lets the plugin |
| --- | --- |
| `filter.pixels` | Have its filters run on the pixels of a layer the person chose. |
| `document.read` | Show facts about the open document in its panels. |
| `document.operations` | Change the document with its commands, undoably, respecting locks. |
| `ui.panel` | Add panels. |
| `ui.tool` | Add entries to the tools menu. |

**Default authority is none.** A manifest names what it wants; the person installing
it ticks what to grant (each is described in plain words); every use is checked
against the grant, and a grant can be withdrawn later without uninstalling.

A capability in the namespaces `filesystem`, `fs`, `network`, `net`, `process`,
`shell`, `registry`, `env`, `clipboard`, `device` or `native` does not exist. A
manifest that asks for one is not given less; it is **refused outright**, with a
message saying plugins are never given those.

## How a plugin edits a document

A plugin never edits a document. Its command's steps go through the same
transaction engine as every other source of change — typed operation, validation,
resource admission, transaction, document invariants, Undo — as `origin: plugin`
with the plugin's id on the transaction:

* the **grant is re-checked** at run time, so a revoked capability stops the command;
* **locked layers** are respected: an operation whose lock policy is
  `automationOnly` is refused on a locked layer for every origin except a person at
  the keyboard (plugin, macro, planner, batch), and one whose policy is `always` is
  refused for everyone;
* it is **atomic**: if step 4 of 5 fails, nothing — no layer, no pixel buffer — is
  left behind;
* it is **one undo entry**, named for the command;
* the engine's authoritative validator runs after every step, so a plugin cannot
  produce a document that violates an invariant.

`core.plugin.apply_filter` and `core.plugin.add_adjustment` run a *filter* (the
module) through the same path: the first bakes the result into a pixel layer, the
second adds an adjustment layer that keeps the filter live. Either records the
plugin's **exact version, content hash and declared locality** in the document.

## Identity and versions

A plugin's identity is a hash of its manifest and its module. A document that uses
one records that hash, the version, and the locality; the render cache keys on it. A
different build of the "same" plugin is a different plugin: PhotoForge will not
substitute it silently. See `docs/plugin-package.md` for updates and
`docs/plugins.md` for what a project does when a plugin is missing.
