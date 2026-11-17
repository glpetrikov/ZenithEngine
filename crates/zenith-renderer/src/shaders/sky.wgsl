// The sky: an equirectangular picture, seen in the direction each pixel looks.

struct Camera {
	view: mat4x4<f32>,
	proj: mat4x4<f32>,
	view_proj: mat4x4<f32>,
	inv_view_proj: mat4x4<f32>,
	viewport: vec4<f32>,
}

struct Params {
	intensity: f32,
	pad0: f32,
	pad1: f32,
	pad2: f32,
}

@group(0) @binding(0) var<uniform> camera: Camera;
@group(0) @binding(1) var<uniform> params: Params;
@group(0) @binding(2) var env_texture: texture_2d<f32>;
@group(0) @binding(3) var env_sampler: sampler;

struct VertexOutput {
	@builtin(position) position: vec4<f32>,
	@location(0) clip: vec2<f32>,
}

const PI: f32 = 3.14159265;

@vertex
fn vs_main(@builtin(vertex_index) id: u32) -> VertexOutput {
	// One triangle over the screen, on the far plane: what was drawn before
	// is nearer and keeps its pixels.
	let uv = vec2<f32>(vec2<u32>((id << 1u) & 2u, id & 2u));
	var out: VertexOutput;
	out.position = vec4<f32>(uv * 2.0 - 1.0, 1.0, 1.0);
	out.clip = uv * 2.0 - 1.0;
	return out;
}

@fragment
fn fs_main(in: VertexOutput) -> @location(0) vec4<f32> {
	// The view matrix has no translation, so the point on the far plane is
	// the direction itself.
	let far = camera.inv_view_proj * vec4<f32>(in.clip, 1.0, 1.0);
	let direction = normalize(far.xyz / far.w);
	let u = atan2(direction.z, direction.x) / (2.0 * PI) + 0.5;
	let v = acos(clamp(direction.y, -1.0, 1.0)) / PI;
	let color = textureSampleLevel(env_texture, env_sampler, vec2<f32>(u, v), 0.0).rgb;
	return vec4<f32>(color * params.intensity, 1.0);
}
