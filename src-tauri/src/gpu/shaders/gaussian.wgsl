// One pass of a separable Gaussian blur, matching high_precision::gaussian.
//
// The CPU reference is the oracle. This kernel reproduces its arithmetic
// deliberately, in the same order: weights are pre-normalised on the host,
// each tap is weighted by the source alpha so invisible colour cannot leak
// into visible pixels, and the result is divided by the accumulated alpha
// weight rather than by the kernel sum.
//
// This file ships with the application. No shader is ever downloaded, and no
// project or user input can reach a shader compiler.

struct Params {
    width: u32,
    height: u32,
    radius: i32,
    horizontal: u32,
};

@group(0) @binding(0) var<storage, read> src: array<vec4<f32>>;
@group(0) @binding(1) var<storage, read_write> dst: array<vec4<f32>>;
@group(0) @binding(2) var<uniform> params: Params;
@group(0) @binding(3) var<storage, read> weights: array<f32>;

@compute @workgroup_size(64)
fn main(@builtin(global_invocation_id) id: vec3<u32>) {
    if (id.x >= params.width || id.y >= params.height) {
        return;
    }
    var sums = vec4<f32>(0.0, 0.0, 0.0, 0.0);
    for (var k: i32 = -params.radius; k <= params.radius; k = k + 1) {
        var sx = i32(id.x);
        var sy = i32(id.y);
        if (params.horizontal == 1u) {
            sx = sx + k;
        } else {
            sy = sy + k;
        }
        // Clamp to edge, as the CPU reference does, so the border does not
        // darken towards transparent.
        sx = clamp(sx, 0, i32(params.width) - 1);
        sy = clamp(sy, 0, i32(params.height) - 1);
        let p = src[u32(sy) * params.width + u32(sx)];
        let weight = weights[u32(k + params.radius)] * p.a;
        sums = sums + vec4<f32>(p.r * weight, p.g * weight, p.b * weight, weight);
    }
    let index = id.y * params.width + id.x;
    if (sums.a > 0.0) {
        dst[index] = vec4<f32>(
            sums.r / sums.a,
            sums.g / sums.a,
            sums.b / sums.a,
            clamp(sums.a, 0.0, 1.0),
        );
    } else {
        dst[index] = vec4<f32>(0.0, 0.0, 0.0, 0.0);
    }
}
