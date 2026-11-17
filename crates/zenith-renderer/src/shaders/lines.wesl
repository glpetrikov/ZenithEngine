// zenith-shader: Zenith/lines 1
// Lines with a color each (the grid, the gizmos and the outline of the
// editor). The positions are relative to the camera.

import package::Zenith::common::Camera;

@group(0) @binding(0) var<uniform> camera: Camera;

struct Vertex {
	@location(0) position: vec3<f32>,
	@location(1) color: vec4<f32>,
}

struct VertexOutput {
	@builtin(position) clip: vec4<f32>,
	@location(0) color: vec4<f32>,
}

@vertex
fn vs_main(vertex: Vertex) -> VertexOutput {
	var out: VertexOutput;
	out.clip = camera.proj * camera.view * vec4<f32>(vertex.position, 1.0);
	out.color = vertex.color;
	return out;
}

@fragment
fn fs_main(in: VertexOutput) -> @location(0) vec4<f32> {
	return in.color;
}
