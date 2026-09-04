# Phase 11 GPU rendering

GPU acceleration is an optional implementation detail. The CPU tiled
compositor is authoritative and is present in CPU-only builds. A GPU context
being created is not treated as a performance claim: the diagnostics counters
report only completed dispatches, pixels, declines and failures.

## Backend decision

PhotoForge uses `wgpu` 30 with an embedded WGSL compute shader. The default
desktop feature set compiles the Vulkan backend and WGSL locally; no driver,
shader, model or compiler resource is downloaded. The Direct3D 12 path was
evaluated but is not enabled in this dependency graph because its
`gpu-allocator`/Windows-crate resolution conflicts with the Tauri dependency
set. A CPU-only build remains available with:

```powershell
cargo build --no-default-features --features custom-protocol
```

The GPU feature is enabled by default for the packaged desktop build, but
initialisation is lazy and runtime-optional. `PHOTOFORGE_DISABLE_GPU=1` is a
hard, fail-closed switch useful for validation and support.

## Supported work

The current device path accelerates only a full-frame, sufficiently wide
Gaussian blur. The source and intermediate buffers use an RGBA32F storage
representation; the shader performs horizontal and vertical passes with the
same bounded kernel policy as the CPU implementation. The operation is sent to
the device only when it meets the measured automatic threshold (at least
2,000,000 pixels and an effective radius of at least 24 taps, within the
validated 60-tap limit). Small/tiled blur work stays on CPU because transfer
overhead dominates it.

The 24-tap threshold is calibrated **inside a document render**, not on an
isolated operation. Measured at 24 MP on an RTX A5500 over Vulkan, a bare-frame
blur crosses over at about nine taps, but the same operation inside a render —
where the pixel store, the composited canvas and the renderer's intermediates
are already resident, and the device path allocates several more frame-sized
buffers — does not cross over until about twenty-four:

| radius | CPU | GPU | gain, in a render |
| --- | --- | --- | --- |
| 12 | 1211 ms | 1332 ms | 0.91x |
| 24 | 1484 ms | 1361 ms | 1.09x |
| 36 | 1956 ms | 1372 ms | 1.43x |
| 60 | 2816 ms | 1391 ms | 2.02x |

An earlier draft used the isolated figure and would have shipped a default that
made a radius-12 blur about nine per cent slower than CPU-only.

`GPU` preference means “prefer this eligible blur”, not “force every operation
onto the device”.

Blends, masks, opacity, groups, transforms, color management, all other
adjustments, RAW development, histogramming and export encoding remain CPU
operations. No GPU compositing parity is claimed. This keeps the CPU and GPU
boundaries explicit and avoids a host↔device round trip for every small node.

## Policy and fallback

The diagnostics settings expose `Auto`, `CPU only` and `Prefer GPU for eligible
blur`. The policy reports the effective backend, adapter state, hard-disable
state and a fallback reason. Any of these conditions selects the CPU path:

- no usable adapter or unsupported limits;
- disabled environment or CPU policy;
- shader/dispatch/readback failure;
- device loss (the session latches the adapter unhealthy rather than retrying a
  broken device on every operation);
- an operation outside the supported full-frame blur contract.

The device is never a startup requirement. A failed GPU operation returns
`None` to its caller, increments the failure counter and recomputes the same
operation through the CPU implementation. The document is not discarded and
no partial GPU buffer is published.

## Correctness and limits

GPU tests compare deterministic blur output with the full-frame CPU reference
within an explicit float tolerance and exercise missing-device, small-work,
CPU-policy and unhealthy-device paths. The CPU and GPU arithmetic can differ
slightly across drivers; bit-identical cross-adapter output is not promised.
The GPU path does not bypass the linear color pipeline because it is reached
only by the already-linear blur operation.

There is no persistent GPU tile/texture cache or VRAM budget in this phase. A
single dispatch allocates checked buffers for the requested frame and releases
them after readback. Dimensions and byte lengths are checked before allocation;
user projects cannot supply shader source. The optional disk render cache is a
separate CPU tile cache and is not a GPU resource.
