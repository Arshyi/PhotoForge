# PhotoForge Phase 13 results — 0.13.0 verification

Phase 13 adds semantic text, vector shape, and smart-object layers to the
non-destructive editor. The source registry, nested-source renderer, explicit
local-link checks, blank-document workflow, project persistence, recovery,
thumbnails, masks, transforms, undo/redo, merge/flatten, and export paths are
implemented in this checkout.

## Verified in this checkout

- `cargo fmt --all -- --check` passes.
- `cargo check --manifest-path src-tauri/Cargo.toml --offline` passes.
- `cargo clippy --manifest-path src-tauri/Cargo.toml --offline --all-targets --all-features -- -D warnings` passes.
- The complete offline Rust suite passes: 1,068 library tests plus the layer,
  raw, precision, restoration, shape, text, tiled, and command integration
  suites; zero failures.
- `npx tsc --noEmit --pretty false` passes.
- `npm run check` (svelte-check) passes: 443 files, 0 errors, 0 warnings.
- `npm test` (Vitest) passes: 899 tests across 59 files, zero failures.
- `npm run build` (Vite production build) succeeds: 206 modules transformed.
- Smart-source tests cover missing references, duplicate IDs, cycles, depth and
  memory limits, hostile link paths, nested sources, shared instances, masks,
  transforms, cache invalidation, CPU/tiled/stream pixel parity, and document
  immutability. Project tests cover a source registry, link metadata, source
  pixels, and source masks surviving a version-2 round trip.

## Rendering boundary

The full-frame float renderer is the semantic oracle. Tiled, cached-tiled, and
streamed output lower each required smart source to one immutable native-size
float composite and then apply normal instance transforms. This gives exact
parity and bounded memory, but the source composite itself is a full-frame CPU
fallback. It is reported in `TiledStats`; Phase 13 does not claim arbitrary
out-of-core smart-source rendering or GPU compositing.

## Packaged artifacts

`npm run tauri build` completed and produced both Windows bundles for 0.13.0:

| Artifact | Size |
| --- | --- |
| `nsis/PhotoForge_0.13.0_x64-setup.exe` | 10,171,365 bytes |
| `msi/PhotoForge_0.13.0_x64_en-US.msi` | 21,004,288 bytes |
| `photoforge.exe` | 45,270,528 bytes |

`SHA256SUMS.txt` records the SHA-256 of all three and verifies against the files
on disk. The build was checked by its output files rather than by its exit
status, because `npm run tauri build` has previously reported success on this
machine while cargo was unreachable and nothing was produced.

All three report `NotSigned` from `Get-AuthenticodeSignature`. That is recorded
rather than remedied: no production signing identity exists here, and a
self-generated certificate would be a fabricated one.

## Not verified here

- The packaged Windows GUI hands-on matrix, including native 125%, 150%, and
  200% display scaling, was not completed. Browser zoom is not a substitute.
- Elevated all-users MSI install/launch/uninstall was not completed because UAC
  consent was cancelled; administrative extraction/launch was not treated as
  MSI lifecycle evidence.
- Production Authenticode signing was unavailable because no trusted
  certificate or signing service was supplied. Binaries remain unsigned.
- WebView2 made Microsoft TLS connections during observation. The PhotoForge
  Rust process opened no observed socket, but the complete process tree cannot
  be described as zero-network.

The installers above exist and their bytes are recorded. No claim is made that
they have been *installed*, launched from an installed location, or exercised
through a GUI; producing a bundle and accepting one are different things.

## A note on the frontend gates

An earlier pass recorded `npm run check`, `npm test` and `npm run build` as
blocked by an esbuild directory-access denial. That was a property of the
sandbox that pass ran in, not of the project: all three run clean on an ordinary
Windows checkout, and the counts above are from this one. The distinction
matters because "blocked" and "failing" read the same in a results table and
mean opposite things.

Running svelte-check here also surfaced one warning that the TypeScript gate
alone could not see: `SemanticCanvas` declared an `oneditselected` prop that
`App.svelte` passed and the component never called. Editing the selected text
layer was already reachable from the Layers panel, its context button and the
canvas panel, so the dead plumbing was removed rather than given an invented
gesture.
