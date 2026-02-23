struct Uniforms {
    scale: vec2<f32>,
    offset: vec2<f32>,
};

@group(0) @binding(0) var frame_texture: texture_2d<f32>;
@group(0) @binding(1) var frame_sampler: sampler;
@group(0) @binding(2) var<uniform> uniforms: Uniforms;

struct VertexOutput {
    @builtin(position) position: vec4<f32>,
    @location(0) uv: vec2<f32>,
};

@vertex
fn vs_main(@builtin(vertex_index) vertex_index: u32) -> VertexOutput {
    // Generate a fullscreen triangle from vertex index (0, 1, 2)
    var pos: array<vec2<f32>, 3> = array<vec2<f32>, 3>(
        vec2<f32>(-1.0, -1.0),
        vec2<f32>(3.0, -1.0),
        vec2<f32>(-1.0, 3.0),
    );

    let raw_pos = pos[vertex_index];

    // Map from clip space [-1, 1] to UV space [0, 1]
    let base_uv = vec2<f32>(
        (raw_pos.x + 1.0) * 0.5,
        // Flip Y: top of screen = uv.y=0, bottom = uv.y=1
        (1.0 - raw_pos.y) * 0.5,
    );

    // Apply aspect ratio correction:
    // Map UV through scale/offset so the frame is centered with correct aspect ratio
    let adjusted_uv = (base_uv - uniforms.offset) / uniforms.scale;

    var output: VertexOutput;
    output.position = vec4<f32>(raw_pos, 0.0, 1.0);
    output.uv = adjusted_uv;
    return output;
}

@fragment
fn fs_main(input: VertexOutput) -> @location(0) vec4<f32> {
    // Discard fragments outside the [0, 1] UV range (letterbox/pillarbox area)
    if (input.uv.x < 0.0 || input.uv.x > 1.0 || input.uv.y < 0.0 || input.uv.y > 1.0) {
        return vec4<f32>(0.0, 0.0, 0.0, 1.0); // Black bars
    }

    let color = textureSample(frame_texture, frame_sampler, input.uv);
    // Force alpha to 1.0 for formats like BGRx/RGBx where alpha channel is undefined
    return vec4<f32>(color.rgb, 1.0);
}
