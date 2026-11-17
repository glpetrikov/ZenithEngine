// zenith-shader: Zenith/magenta 1
// What stands in for a shader of a material that is missing or does not
// compile: plain magenta, so it is seen at once. It takes what the passes that
// draw meshes give (the camera, the vertex and the instance).

import package::Zenith::common::Camera;

@group(0) @binding(0) var<uniform> camera: Camera;

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
	return camera.proj * camera.view * model * vec4<f32>(vertex.position, 1.0);
}

@fragment
fn fs_main() -> @location(0) vec4<f32> {
	return vec4<f32>(1.0, 0.0, 1.0, 1.0);
}
