// One mip level of a texture from the one above it: a triangle that covers
// the target, sampling the bigger level linearly (four texels into one).

struct Out {
    @builtin(position) position: vec4f,
    @location(0) uv: vec2f,
}

@vertex
fn vs_main(@builtin(vertex_index) index: u32) -> Out {
    // (0, 0), (2, 0), (0, 2): a triangle twice the size of the target.
    let uv = vec2f(f32((index << 1u) & 2u), f32(index & 2u));
    var out: Out;
    out.position = vec4f(uv.x * 2.0 - 1.0, 1.0 - uv.y * 2.0, 0.0, 1.0);
    out.uv = uv;
    return out;
}

@group(0) @binding(0) var source: texture_2d<f32>;
@group(0) @binding(1) var linear_sampler: sampler;

@fragment
fn fs_main(in: Out) -> @location(0) vec4f {
    return textureSample(source, linear_sampler, in.uv);
}
