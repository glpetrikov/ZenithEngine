// The sky: what is behind everything, read from the environment cube map in
// the direction each pixel of the screen looks at.

struct Camera {
    view_pos: vec4<f32>,
    view: mat4x4<f32>,
    view_proj: mat4x4<f32>,
    inv_proj: mat4x4<f32>,
    inv_view: mat4x4<f32>,
}
@group(0) @binding(0)
var<uniform> camera: Camera;

@group(1) @binding(0)
var env_map: texture_cube<f32>;
@group(1) @binding(1)
var env_sampler: sampler;

struct VertexOutput {
    @builtin(position) frag_position: vec4<f32>,
    @location(0) clip_position: vec4<f32>,
}

@vertex
fn vs_main(@builtin(vertex_index) id: u32) -> VertexOutput {
    // One triangle over the whole screen, on the far plane: anything that was
    // drawn before is nearer and keeps its pixels (the depth test is
    // `LessEqual`, and the sky does not write depth).
    let uv = vec2<f32>(vec2<u32>((id << 1u) & 2u, id & 2u));
    let clip = vec4<f32>(uv * 2.0 - 1.0, 1.0, 1.0);

    var out: VertexOutput;
    out.frag_position = clip;
    out.clip_position = clip;
    return out;
}

@fragment
fn fs_main(in: VertexOutput) -> @location(0) vec4<f32> {
    // The way back: from the screen to the view space, and from there (only
    // the direction counts, so w = 0) to the world.
    let view_pos = camera.inv_proj * in.clip_position;
    let view_ray = view_pos.xyz / view_pos.w;
    let ray_direction = normalize((camera.inv_view * vec4<f32>(view_ray, 0.0)).xyz);

    return textureSample(env_map, env_sampler, ray_direction);
}
