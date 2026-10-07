# Plugins

A PhotoForge plugin is a `.photoforge-plugin` file that adds **filters** (pixel
functions), **commands** (named lists of PhotoForge's own operations), **panels**
(a few rows of facts and buttons) and **tools** (menu entries). It is installed by
hand, runs in a sandbox with no authority until it is given some, and leaves a
document in a state PhotoForge can always explain.

This page is the overview. The details are in:

* [plugin-api.md](plugin-api.md) — what a plugin can ask of PhotoForge, the module ABI, limits;
* [plugin-package.md](plugin-package.md) — the file format, installing, updating;
* [plugin-security.md](plugin-security.md) — the threat model, and what is *not* protected;
* [plugin-runtime-decision.md](plugin-runtime-decision.md) — why Wasmtime, with measurements;
* [automation.md](automation.md) — macros, which use the same operations a plugin's commands do.

## What a plugin is not

* **Not a native library.** PhotoForge never loads a plugin's DLL, and a package
  cannot contain one. The only code that runs is a WebAssembly module, and only
  through a filter.
* **Not a script.** There is no Python, JavaScript or shell, no expressions in a
  manifest, no loops, no conditions. Commands are lists of registered operations.
* **Not a way to reach the machine.** There is no filesystem, network, process,
  shell, registry, clipboard or device access, and no capability by those names
  exists to be granted.
* **Not a second editor.** A plugin changes a document only by asking the same
  transaction engine as the Layers panel, a macro and the planner. See below.
* **Not downloaded by PhotoForge.** There is no marketplace, no update check and no
  cloud registry. You bring the file.
* **Not signed.** PhotoForge does not verify who wrote a package and says so before
  you install it. What it limits is what a package can do.

## Using plugins

### Installing

**Plugins** (palette: *Manage plugins…*, or Settings) lists what is installed.
*Install plugin…* asks for a `.photoforge-plugin` file and shows a review before
anything is written:

* the name, version, publisher, description and licence the package states;
* every capability it asks for, **in plain words**, each with its own tick box —
  nothing is granted that you do not tick;
* its size and hashes, and **"Not signed."**;
* whether a version is already installed, and whether this one is older;
* its README, if it has one.

Install is all or nothing. The module is compiled and each filter is **tested**
before the install completes: run twice with identical results, and run whole and
in tiles with identical results. A plugin that fails is not installed and the
message says which filter failed and how. The self-test is what makes the filter's
declared reach (*locality*) something PhotoForge has checked rather than something
it was told.

For each installed plugin the manager offers: turn it on or off; change what it is
allowed to do (which takes effect at once and is checked on every use); **Test** it
again; read its README and licence; remove one version; remove the plugin.
Removing asks twice.

### Running a filter

*Run a plugin filter…* (palette, or the Plugins panel) shows the filters of plugins
that can run, each with its description, **how far it reaches** in words
("Looks only at each pixel on its own", "Looks at pixels up to 4 away", "Looks at the
whole image at once"), and its parameters as ordinary controls. Two buttons:

* **Apply to *layer*** bakes the result into the selected pixel layer's pixels.
  Disabled with a reason if the layer is not a pixel layer or is locked.
* **Add as adjustment layer** adds a layer that keeps the filter *live*, so the
  parameters can be changed later and the filter can be masked and reordered like any
  adjustment. Needs a linear-float document; the dialog says so if the document is not.

Either is one undo entry. The values you chose are remembered for next time.

A filter that *looks at the whole image at once* (`global`) on a very large image may
be **refused before it starts**: it needs room for the whole image in the plugin's
memory, and the message says how many megabytes and that a smaller image or region
would work. A neighbourhood or pointwise filter is run in tiles and does not have
this problem.

### Commands, panels and tools

A plugin's commands appear in the command palette under the plugin's name
(`plugin:<id>:<command>` internally, so a plugin can never take the name of a
built-in command). Choosing one asks for its parameters if it has any and runs it
as a transaction. Its panels appear in the Plugins panel, with live facts about the
open document. Its tools are menu entries that run a command: a plugin cannot draw
on the canvas or receive pointer events.

A command, panel or tool is listed only if the plugin is available *and* was granted
what that needs. Anything short of that is not listed at all, rather than listed and
refused.

## Plugins in a document

A plugin filter in a layer is an ordinary edit in the document model
(`EditOperation::PluginFilter`). It records the plugin's **id, exact version, content
hash and declared locality**, and its parameters. Because it is a normal render node:

* it works in **masked** edits, in **adjustment layers**, in groups, and in the
  **tiled renderer**, which gives a `local` filter the margin it declared so tile
  edges do not show;
* it is part of the **render cache key** (with its hash), so a cached tile is never
  reused for a different build of the plugin or different parameters;
* it participates in **Undo and Redo** like any other change;
* a **legacy 8-bit** document refuses it ("plugin filters require a high-precision
  (linear float) document"), because the filter's contract is linear float pixels and
  quantising to and from them on every render would be a silent loss. Convert the
  document in *Color and precision* first.

### When the plugin is not there

A project is never made unreadable, or quietly altered, because of a plugin.

* The project **opens**, and **saves back with the reference intact**. Nothing is
  dropped or replaced.
* The layer is **hidden in the preview**, the preview reports which plugins are
  missing, the layer shows a badge, and a banner in the Plugins panel names the
  plugin and the reason: not installed, turned off, not allowed to run filters,
  damaged, or *a different version is installed*.
* **Export, merge, flatten and rasterize refuse**, naming the plugin, so a file that
  is missing a layer cannot be produced by accident. They work again once the exact
  plugin is available.
* A different version of the "same" plugin is **never substituted**. Install the
  exact version the project was made with, or remove the layer yourself.

## Writing a plugin

The repository's `plugins/examples/` has six working plugins, each a manifest, an
optional module in WebAssembly text, and a README:

| Example | Shows |
| --- | --- |
| `solarize` | A pointwise filter with one parameter. The smallest useful module. |
| `channel_swap` | A pointwise filter with a `choice` parameter. |
| `border` | A pointwise filter that uses its position in the image (it is told where its window sits). |
| `boxblur` | A **local** filter: it declares a reach of 4 and is checked against it. |
| `shapes` | A command that is two registered operations — add a layer, then run the plugin's own filter on it. |
| `inspector` | A plugin that is only declarations: a panel and a command, no module, no code. |

Build them into packages with:

```text
cargo run --example build_example_plugins -- [output directory]
```

`plugins/adversarial/` holds 25 deliberately hostile modules (infinite loops, memory
hogs, NaN output, a filter that lies about locality, a WASI import…) that the test
suite throws at the sandbox. They are not installable, not shipped, and are useful to
read if you want to see what PhotoForge defends against.

Advice that follows from how the host works:

* A filter is **pure**: the same input and parameters must give the same output. No
  state survives a call; each call is a fresh instance.
* **Declare locality honestly**, and as narrowly as is true. `pointwise` is run in
  tiles with no margin; `local` with a radius gets that margin; `global` needs the
  whole image in memory and may be refused on a large one. The install self-test
  catches a filter that lies on its test image; it cannot catch every lie, and a
  filter that is wrong on other images produces a visible seam in its own layer.
* Pixels are **linear float, straight alpha**, and values above 1.0 are legal.
* A module is **single-threaded** and has no clock or randomness. Ask for the memory
  you need in `limits.memoryMib`; the host may give you less on a small machine.

## Building without plugins

The runtime is behind the `plugins` Cargo feature, on by default. A build without it
(`--no-default-features`, with whichever other features are wanted) carries no
WebAssembly engine at all. It still opens projects that mention plugins, reports
those layers as unavailable with "this build does not include the plugin runtime",
and refuses to export them, exactly as if the plugin were missing.
