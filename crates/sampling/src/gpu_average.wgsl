struct Params {
    x0: u32,
    y0: u32,
    x1: u32,
    y1: u32,
}

struct Result {
    r_sum: atomic<u32>,
    g_sum: atomic<u32>,
    b_sum: atomic<u32>,
    count: atomic<u32>,
}

@group(0) @binding(0) var input_texture: texture_2d<f32>;
@group(0) @binding(1) var<uniform> params: Params;
@group(0) @binding(2) var<storage, read_write> result: Result;

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

    atomicAdd(&result.r_sum, r);
    atomicAdd(&result.g_sum, g);
    atomicAdd(&result.b_sum, b);
    atomicAdd(&result.count, 1u);
}
