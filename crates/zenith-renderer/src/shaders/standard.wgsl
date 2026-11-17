// The standard material, as drawn by `zenith.opaque` and `zenith.transparent`.
//
// Group 0: what the pass was given (camera, lights, scene, environment,
// shadow map). Group 1: the material. Everything is in world space relative
// to the camera, so the view matrix has no translation.
//
// The lighting is Lambert for the diffuse part and Blinn-Phong for the
// specular one, with the shininess taken from the roughness. The environment
// is an equirectangular picture: a blurry look at it (a high mip level) is
// the ambient light, a look along the reflection (a level that grows with the
// roughness) is the reflection.

struct Camera {
	view: mat4x4<f32>,
	proj: mat4x4<f32>,
	view_proj: mat4x4<f32>,
	inv_view_proj: mat4x4<f32>,
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

@group(0) @binding(0) var<uniform> camera: Camera;
@group(0) @binding(1) var<uniform> lights: Lights;
@group(0) @binding(2) var<uniform> scene: Scene;
@group(0) @binding(3) var env_texture: texture_2d<f32>;
@group(0) @binding(4) var env_sampler: sampler;
@group(0) @binding(5) var shadow_map: texture_depth_2d;
@group(0) @binding(6) var shadow_sampler: sampler_comparison;

@group(1) @binding(0) var<uniform> material: Material;
@group(1) @binding(1) var base_texture: texture_2d<f32>;
@group(1) @binding(2) var base_sampler: sampler;
@group(1) @binding(3) var normal_texture: texture_2d<f32>;
@group(1) @binding(4) var normal_sampler: sampler;

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
	@location(2) world: vec3<f32>,
}

@vertex
fn vs_main(vertex: Vertex, instance: Instance) -> VertexOutput {
	let model = mat4x4<f32>(instance.model_0, instance.model_1, instance.model_2, instance.model_3);
	let normal_matrix = mat3x3<f32>(instance.normal_0.xyz, instance.normal_1.xyz, instance.normal_2.xyz);
	let world = model * vec4<f32>(vertex.position, 1.0);
	var out: VertexOutput;
	out.clip = camera.proj * camera.view * world;
	out.uv = vertex.uv;
	out.normal = normalize(normal_matrix * vertex.normal);
	out.world = world.xyz;
	return out;
}

const MASKED: u32 = 1u;
const PI: f32 = 3.14159265;

// The direction as a place in an equirectangular picture.
fn equirect(direction: vec3<f32>) -> vec2<f32> {
	let u = atan2(direction.z, direction.x) / (2.0 * PI) + 0.5;
	let v = acos(clamp(direction.y, -1.0, 1.0)) / PI;
	return vec2<f32>(u, v);
}

// The normal of the surface turned by the normal map, with the frame of the
// surface taken from the derivatives of the position and the uv.
fn perturbed_normal(normal: vec3<f32>, world: vec3<f32>, uv: vec2<f32>) -> vec3<f32> {
	let tangent_space = textureSample(normal_texture, normal_sampler, uv).xyz * 2.0 - 1.0;
	let dp1 = dpdx(world);
	let dp2 = dpdy(world);
	let duv1 = dpdx(uv);
	let duv2 = dpdy(uv);
	let dp2perp = cross(dp2, normal);
	let dp1perp = cross(normal, dp1);
	let tangent = dp2perp * duv1.x + dp1perp * duv2.x;
	let bitangent = dp2perp * duv1.y + dp1perp * duv2.y;
	let inv_max = inverseSqrt(max(max(dot(tangent, tangent), dot(bitangent, bitangent)), 1e-12));
	let frame = mat3x3<f32>(tangent * inv_max, bitangent * inv_max, normal);
	return normalize(frame * tangent_space);
}

// How much of the light reaches the point: 1 in the light, 0 in the shadow.
fn shadow_factor(world: vec3<f32>, normal: vec3<f32>, to_light: vec3<f32>) -> f32 {
	if (scene.params.z < 0.5) {
		return 1.0;
	}
	// Moved along the normal, more where the surface is steep to the light.
	let slope = 1.0 - clamp(dot(normal, to_light), 0.0, 1.0);
	let offset = normal * (scene.shadow.x * (1.0 + 2.0 * slope));
	let light_clip = scene.shadow_matrix * vec4<f32>(world + offset, 1.0);
	let ndc = light_clip.xyz / light_clip.w;
	let uv = vec2<f32>(ndc.x * 0.5 + 0.5, 0.5 - ndc.y * 0.5);
	if (ndc.z > 1.0 || uv.x < 0.0 || uv.x > 1.0 || uv.y < 0.0 || uv.y > 1.0) {
		return 1.0;
	}
	var lit = 0.0;
	for (var x = -1; x <= 1; x++) {
		for (var y = -1; y <= 1; y++) {
			let shift = vec2<f32>(f32(x), f32(y)) * scene.shadow.y;
			lit += textureSampleCompareLevel(shadow_map, shadow_sampler, uv + shift, ndc.z);
		}
	}
	return lit / 9.0;
}

@fragment
fn fs_main(in: VertexOutput) -> @location(0) vec4<f32> {
	let texel = textureSample(base_texture, base_sampler, in.uv);
	let albedo = material.base_color * texel;
	let normal = perturbed_normal(normalize(in.normal), in.world, in.uv);
	if ((material.flags & MASKED) != 0u && albedo.a < material.alpha_cutoff) {
		discard;
	}

	let roughness = clamp(material.roughness, 0.04, 1.0);
	let metallic = clamp(material.metallic, 0.0, 1.0);
	let diffuse_color = albedo.rgb * (1.0 - metallic);
	let specular_color = mix(vec3<f32>(0.04), albedo.rgb, metallic);
	let view_dir = normalize(-in.world);
	let shininess = clamp(2.0 / (roughness * roughness * roughness * roughness) - 2.0, 1.0, 4096.0);
	let normalization = (shininess + 8.0) / (8.0 * PI);

	var color = vec3<f32>(0.0);

	for (var i = 0u; i < lights.counts.x; i++) {
		let light = lights.directional[i];
		let to_light = -normalize(light.direction.xyz);
		let n_dot_l = max(dot(normal, to_light), 0.0);
		var visibility = 1.0;
		if (light.color.w > 0.5) {
			visibility = shadow_factor(in.world, normalize(in.normal), to_light);
		}
		let half_vector = normalize(to_light + view_dir);
		let specular = specular_color * normalization * pow(max(dot(normal, half_vector), 0.0), shininess);
		color += (diffuse_color / PI + specular) * light.color.rgb * n_dot_l * visibility;
	}

	for (var i = 0u; i < lights.counts.y; i++) {
		let light = lights.point[i];
		let offset = light.position.xyz - in.world;
		let distance = length(offset);
		let to_light = offset / max(distance, 1e-4);
		let range = max(light.position.w, 1e-3);
		let window = clamp(1.0 - pow(distance / range, 4.0), 0.0, 1.0);
		let attenuation = window * window / (distance * distance + 1.0);
		let n_dot_l = max(dot(normal, to_light), 0.0);
		let half_vector = normalize(to_light + view_dir);
		let specular = specular_color * normalization * pow(max(dot(normal, half_vector), 0.0), shininess);
		color += (diffuse_color / PI + specular) * light.color.rgb * n_dot_l * attenuation;
	}

	var ambient = lights.ambient.rgb * diffuse_color;
	if (scene.params.y > 0.5) {
		let max_lod = scene.params.w;
		let irradiance = textureSampleLevel(env_texture, env_sampler, equirect(normal), max_lod).rgb;
		let reflected = reflect(-view_dir, normal);
		let reflection = textureSampleLevel(env_texture, env_sampler, equirect(reflected), roughness * max_lod).rgb;
		let facing = 1.0 - max(dot(normal, view_dir), 0.0);
		let fresnel = specular_color + (vec3<f32>(1.0) - specular_color) * pow(facing, 5.0) * (1.0 - roughness);
		ambient += scene.params.x * (irradiance * diffuse_color + reflection * fresnel);
	}
	color += ambient;

	return vec4<f32>(color, albedo.a);
}
