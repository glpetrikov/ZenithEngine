// The standard material, as drawn by `zenith.opaque`.
//
// Group 0: the camera. Group 1: the material. Everything is in world space
// relative to the camera, so the view matrix has no translation.
// A first version: one fixed light and an ambient term. The real lighting
// comes with the light passes.

struct Camera {
	view: mat4x4<f32>,
	proj: mat4x4<f32>,
	view_proj: mat4x4<f32>,
	inv_view_proj: mat4x4<f32>,
	viewport: vec4<f32>,
}

struct Material {
	base_color: vec4<f32>,
	roughness: f32,
	metallic: f32,
	alpha_cutoff: f32,
	flags: u32,
}

@group(0) @binding(0) var<uniform> camera: Camera;
@group(1) @binding(0) var<uniform> material: Material;
@group(1) @binding(1) var base_texture: texture_2d<f32>;
@group(1) @binding(2) var base_sampler: sampler;

struct Vertex {
	@location(0) position: vec3<f32>,
	@location(1) uv: vec2<f32>,
	@location(2) normal: vec3<f32>,
}

struct Instance {
	@location(5) model_0: vec4<f32>,
	@location(6) model_1: vec4<f32>,
	@location(7) model_2: vec4<f32>,
	@location(8) model_3: vec4<f32>,
	@location(9) normal_0: vec4<f32>,
	@location(10) normal_1: vec4<f32>,
	@location(11) normal_2: vec4<f32>,
}

struct VertexOutput {
	@builtin(position) clip: vec4<f32>,
	@location(0) uv: vec2<f32>,
	@location(1) normal: vec3<f32>,
}

@vertex
fn vs_main(vertex: Vertex, instance: Instance) -> VertexOutput {
	let model = mat4x4<f32>(instance.model_0, instance.model_1, instance.model_2, instance.model_3);
	let normal_matrix = mat3x3<f32>(instance.normal_0.xyz, instance.normal_1.xyz, instance.normal_2.xyz);
	var out: VertexOutput;
	out.clip = camera.proj * camera.view * model * vec4<f32>(vertex.position, 1.0);
	out.uv = vertex.uv;
	out.normal = normalize(normal_matrix * vertex.normal);
	return out;
}

const MASKED: u32 = 1u;

@fragment
fn fs_main(in: VertexOutput) -> @location(0) vec4<f32> {
	let texel = textureSample(base_texture, base_sampler, in.uv);
	let albedo = material.base_color * texel;
	if ((material.flags & MASKED) != 0u && albedo.a < material.alpha_cutoff) {
		discard;
	}
	let sun = normalize(vec3<f32>(0.4, 0.8, 0.5));
	let light = 0.25 + 0.75 * max(dot(normalize(in.normal), sun), 0.0);
	return vec4<f32>(albedo.rgb * light, albedo.a);
}
