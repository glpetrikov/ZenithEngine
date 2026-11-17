// Sprites: a quad per instance, facing +Z of its own, tinted and cut out of a
// texture. Everything is relative to the camera.

struct Camera {
	view: mat4x4<f32>,
	proj: mat4x4<f32>,
	view_proj: mat4x4<f32>,
	inv_view_proj: mat4x4<f32>,
	viewport: vec4<f32>,
}

@group(0) @binding(0) var<uniform> camera: Camera;
@group(1) @binding(0) var sprite_texture: texture_2d<f32>;
@group(1) @binding(1) var sprite_sampler: sampler;

struct Instance {
	@location(0) position: vec3<f32>,
	@location(1) size: vec2<f32>,
	@location(2) rotation: vec4<f32>,
	@location(3) color: vec4<f32>,
	@location(4) uv_rect: vec4<f32>,
}

struct VertexOutput {
	@builtin(position) clip: vec4<f32>,
	@location(0) uv: vec2<f32>,
	@location(1) color: vec4<f32>,
}

fn rotate(q: vec4<f32>, v: vec3<f32>) -> vec3<f32> {
	return v + 2.0 * cross(q.xyz, cross(q.xyz, v) + q.w * v);
}

@vertex
fn vs_main(@builtin(vertex_index) index: u32, instance: Instance) -> VertexOutput {
	var corners = array<vec2<f32>, 6>(
		vec2<f32>(0.0, 0.0),
		vec2<f32>(1.0, 0.0),
		vec2<f32>(1.0, 1.0),
		vec2<f32>(0.0, 0.0),
		vec2<f32>(1.0, 1.0),
		vec2<f32>(0.0, 1.0),
	);
	let corner = corners[index];
	// The top of the texture is the top of the sprite.
	let local = vec3<f32>((corner.x - 0.5) * instance.size.x, (0.5 - corner.y) * instance.size.y, 0.0);
	let world = instance.position + rotate(instance.rotation, local);
	var out: VertexOutput;
	out.clip = camera.proj * camera.view * vec4<f32>(world, 1.0);
	out.uv = instance.uv_rect.xy + corner * instance.uv_rect.zw;
	out.color = instance.color;
	return out;
}

@fragment
fn fs_main(in: VertexOutput) -> @location(0) vec4<f32> {
	return textureSample(sprite_texture, sprite_sampler, in.uv) * in.color;
}
