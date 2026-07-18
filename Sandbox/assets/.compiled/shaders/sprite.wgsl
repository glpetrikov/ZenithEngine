@group(0) @binding(0)
var sprite_texture: texture_2d<f32>;

@group(0) @binding(1)
var sprite_sampler: sampler;

@group(1) @binding(0)
var<uniform> view_projection: mat4x4<f32>;

struct Vertex {
    @location(0)
    position: vec3<f32>,
    @location(1)
    color: vec4<f32>
}

struct Instance {
    @location(2)
    model_0: vec4<f32>,
    @location(3)
    model_1: vec4<f32>,
    @location(4)
    model_2: vec4<f32>,
    @location(5)
    model_3: vec4<f32>,
    @location(6)
    uv_rect: vec4<f32>,
    @location(7)
    tint: vec4<f32>,
    @location(8)
    params: vec4<f32>,
    @location(9)
    emissive: vec4<f32>
}

struct VertexOutput {
    @builtin(position)
    position: vec4<f32>,
    @location(0)
    color: vec4<f32>,
    @location(1)
    tex_coord: vec2<f32>,
    @location(2)
    uv_rect: vec4<f32>,
    @location(3)
    tint: vec4<f32>,
    @location(4)
    params: vec4<f32>,
    @location(5)
    emissive: vec4<f32>
}

@vertex
fn vs_main(vertex: Vertex, instance: Instance) -> VertexOutput {
    let model = mat4x4<f32>(instance.model_0, instance.model_1, instance.model_2, instance.model_3);
    var out: VertexOutput;
    out.position = view_projection * model * vec4<f32>(vertex.position, 1.0);
    out.color = vertex.color;
    out.tex_coord = vec2<f32>(vertex.position.x + 0.5, -vertex.position.y + 0.5);
    out.uv_rect = instance.uv_rect;
    out.tint = instance.tint;
    out.params = instance.params;
    out.emissive = instance.emissive;
    return out;
}

fn rotate_tex_coord(tex_coord: vec2<f32>, angle: f32) -> vec2<f32> {
    let centered = tex_coord - vec2<f32>(0.5, 0.5);
    let sine = sin(angle);
    let cosine = cos(angle);
    return vec2<f32>(centered.x * cosine - centered.y * sine, centered.x * sine + centered.y * cosine) + vec2<f32>(0.5, 0.5);
}

struct FragmentOutput {
    @location(0)
    albedo: vec4<f32>,
    @location(1)
    emissive: vec4<f32>,
    @location(2)
    normal: vec4<f32>
}

@fragment
fn fs_main(in: VertexOutput) -> FragmentOutput {
    let mode = in.params.x;
    let strength = in.params.y;
    let saturation_threshold = in.params.z;
    let texture_rotation = in.params.w;
    let rotated = rotate_tex_coord(in.tex_coord, texture_rotation);
    let uv_offset = in.uv_rect.xy;
    let uv_scale = in.uv_rect.zw;
    let inset = uv_scale * 0.001;
    let tex_coord = clamp(uv_offset + rotated * uv_scale, uv_offset + inset, uv_offset + uv_scale - inset);
    let sampled = textureSample(sprite_texture, sprite_sampler, tex_coord) * in.color;
    var color: vec4<f32>;
    if mode < 0.5 {
        color = sampled;
    }
    else if mode < 1.5 {
        color = sampled * in.tint;
    }
    else {
        let max_channel = max(sampled.r, max(sampled.g, sampled.b));
        let min_channel = min(sampled.r, min(sampled.g, sampled.b));
        let saturation = max_channel - min_channel;
        let grayscale_factor = 1.0 - smoothstep(0.0, saturation_threshold, saturation);
        let gray = dot(sampled.rgb, vec3<f32>(0.299, 0.587, 0.114));
        let tinted_gray = gray * in.tint.rgb;
        let mix_factor = clamp(grayscale_factor * strength, 0.0, 1.0);
        color = vec4<f32>(mix(sampled.rgb, tinted_gray, mix_factor), sampled.a * in.tint.a);
    }
    var out: FragmentOutput;
    out.albedo = color;
    out.emissive = vec4<f32>(color.rgb * in.emissive.x, color.a);
    out.normal = vec4<f32>(0.5, 0.5, 1.0, 1.0);
    return out;
}
