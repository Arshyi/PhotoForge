# Phase 13 smart objects

A smart object is an editable source stack plus one or more placements of that
stack. The placement is a normal layer: it owns the transform, opacity, blend
mode, visibility, lock, collapse state, and mask. The source owns its own native
width and height and an ordinary layer tree, which may include text, shapes,
adjustments, and nested smart objects.

## Source and instance semantics

The document keeps sources in `LayerDocument.smartSources`, keyed by a bounded
identifier. A `smart_object` layer stores only `sourceId`. Two instances can
therefore share one source without sharing their placement. Editing source
contents updates every instance; **Independent copy** clones the referenced
source dependency graph and gives the new instance a new source identity.

Source graphs are validated before they reach a renderer. Identifiers are
restricted, layer IDs remain unique across the root tree and every source tree,
missing references are rejected, cycles are rejected, nesting is capped at four
levels, and source count, native edges, aggregate layer count, and composite
memory are bounded. Pixel IDs inside source stacks are included in project-store
resolution and recovery, so a source cannot silently render blank after reload.

## Rendering

Each required source is composed once at native size in straight-alpha linear
sRGB float pixels. Nested sources are lowered to immutable aliases in dependency
order. The outer layer then applies the instance transform and mask, so repeated
scale changes never bake a previous resample into the source. Full-frame, tiled,
cached-tiled, and streamed rendering all use the same float semantics; tests
compare their pixels exactly and assert that rendering does not mutate the
document.

Native source composites currently use a deliberately bounded full-frame CPU
fallback. The outer result still tiles or streams, and `TiledStats` reports the
source composite bytes and the fallback flag. This is safe and deterministic but
is not a claim that arbitrarily large smart sources are out-of-core; a disk-backed
or source-tiled implementation is a later performance phase.

## Authoring and baking

Semantic authoring requires a linear-float document. Use **New** for a blank
high-precision canvas or convert an existing document under **Color and
precision**. The Layers panel can convert a layer or contiguous sibling
selection into a smart object, edit source contents in an isolated subdocument,
create an independent copy, check links, relink, and explicitly rasterize.
Rasterizing replaces only the selected semantic content with a canvas-sized
pixel layer; placement properties that would otherwise be applied twice remain
editable on the replacement layer. Undo restores the source and instance trees.

## Linked files

Importing a linked image reads one bounded local file and stores an embedded
snapshot plus its absolute local path, byte count, and SHA-256 digest. The
embedded snapshot remains render authority. Opening a project never reads the
recorded path. **Check Links** hashes the file only after an explicit user action
and reports Embedded, Available, Missing, or Changed. **Relink** requires a new
explicit path; replacing Changed content requires an explicit accept choice.
UNC/network paths, URLs, traversal, alternate data streams, device names,
reparse-point traversal, and files over the byte ceiling are rejected.

## Persistence and recovery

The version-2 `.photoforge` manifest serializes the source registry and its
semantic trees alongside the root tree. Masks remain detached into bounded PNG
entries and are reattached by globally unique layer ID. Pixel buffers referenced
only by a source are still stored and restored. Recovery snapshots use the same
typed project container and never dereference links or modify the source project.

## Deliberate limits

There is no PSD import/export, procedural or neural layer, live external-file
replacement, embedded font, GPU smart-source compositor, or arbitrary out-of-core
source cache. These boundaries are explicit so a project stays deterministic,
local, and reviewable.
