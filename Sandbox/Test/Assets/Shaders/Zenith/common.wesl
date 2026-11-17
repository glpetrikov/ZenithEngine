// zenith-shader: Zenith/common 1
// The structs the passes that draw meshes agree on with the shaders: the
// camera, the lights and the scene (group 0) and the material (group 1). Their
// Rust twins are `CameraUniform`, `LightsUniform`, `SceneUniform` and
// `MaterialUniform`; change both together.

struct Camera {
	view: mat4x4<f32>,
	proj: mat4x4<f32>,
	view_proj: mat4x4<f32>,
	inv_view_proj: mat4x4<f32>,
	// The size of the viewport in pixels, then near and far.
	viewport: vec4<f32>,
}

struct DirectionalLight {
	// The way the light travels.
	direction: vec4<f32>,
	// Color times illuminance; w is 1 for the light that casts the shadow.
	color: vec4<f32>,
}

struct PointLight {
	// w is the range.
	position: vec4<f32>,
	color: vec4<f32>,
}

struct Lights {
	// rgb: the ambient light that does not come from the environment.
	ambient: vec4<f32>,
	// x: directional lights, y: point lights.
	counts: vec4<u32>,
	directional: array<DirectionalLight, 4>,
	point: array<PointLight, 16>,
}

struct Scene {
	shadow_matrix: mat4x4<f32>,
	// x: the intensity of the environment, y: 1 if there is one, z: 1 if
	// there is a shadow map, w: the last mip level of the environment.
	params: vec4<f32>,
	// x: the bias, y: the size of a texel of the shadow map in 0..1.
	shadow: vec4<f32>,
}

struct Material {
	base_color: vec4<f32>,
	roughness: f32,
	metallic: f32,
	alpha_cutoff: f32,
	flags: u32,
}
