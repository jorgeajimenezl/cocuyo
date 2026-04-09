struct Params {
    x0: u32,
    y0: u32,
    x1: u32,
    y1: u32,
}

@group(0) @binding(0) var input_texture: texture_2d<f32>;
@group(0) @binding(1) var<uniform> params: Params;
@group(0) @binding(2) var<storage, read_write> histogram: array<atomic<u32>, 2048>;

// 512 bins (8×8×8 RGB quantization), each bin stores 4 u32s:
//   [bin*4 + 0] = r_sum
//   [bin*4 + 1] = g_sum
//   [bin*4 + 2] = b_sum
//   [bin*4 + 3] = count

@compute @workgroup_size(16, 16)
fn main(@builtin(global_invocation_id) gid: vec3<u32>) {
    let px = params.x0 + gid.x;
    let py = params.y0 + gid.y;

    if px >= params.x1 || py >= params.y1 {
        return;
    }

    let color = textureLoad(input_texture, vec2<u32>(px, py), 0);

    // Convert from [0,1] float to [0,255] u32.
    // Using a non-sRGB texture view so textureLoad returns raw byte values.
    let r = u32(color.r * 255.0 + 0.5);
    let g = u32(color.g * 255.0 + 0.5);
    let b = u32(color.b * 255.0 + 0.5);

    // Bin index: 8 bins per channel, (r>>5)*64 + (g>>5)*8 + (b>>5)
    let bin = (r >> 5u) * 64u + (g >> 5u) * 8u + (b >> 5u);
    let base = bin * 4u;

    atomicAdd(&histogram[base + 0u], r);
    atomicAdd(&histogram[base + 1u], g);
    atomicAdd(&histogram[base + 2u], b);
    atomicAdd(&histogram[base + 3u], 1u);
}
