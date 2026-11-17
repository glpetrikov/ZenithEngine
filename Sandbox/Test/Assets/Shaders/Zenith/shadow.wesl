// zenith-shader: Zenith/shadow 1
// The shadow pass: the depth of the meshes seen from the light.

struct Light {
	view_proj: mat4x4<f32>,
}

@group(0) @binding(0) var<uniform> light: Light;

struct Vertex {
	@location(0) position: vec3<f32>,
}

struct Instance {
	@location(5) model_0: vec4<f32>,
	@location(6) model_1: vec4<f32>,
	@location(7) model_2: vec4<f32>,
	@location(8) model_3: vec4<f32>,
}

@vertex
fn vs_main(vertex: Vertex, instance: Instance) -> @builtin(position) vec4<f32> {
	let model = mat4x4<f32>(instance.model_0, instance.model_1, instance.model_2, instance.model_3);
	return light.view_proj * model * vec4<f32>(vertex.position, 1.0);
}
