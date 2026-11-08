// The scene: instanced, lit by one point light, with normal mapping.

struct Camera {
    view_pos: vec4<f32>,
    view: mat4x4<f32>,
    view_proj: mat4x4<f32>,
    inv_proj: mat4x4<f32>,
    inv_view: mat4x4<f32>,
}
@group(1) @binding(0)
var<uniform> camera: Camera;

struct Light {
    position: vec3<f32>,
    color: vec3<f32>,
}
@group(2) @binding(0)
var<uniform> light: Light;

struct VertexInput {
    @location(0) position: vec3<f32>,
    @location(1) tex_coords: vec2<f32>,
    @location(2) normal: vec3<f32>,
    @location(3) tangent: vec3<f32>,
    @location(4) bitangent: vec3<f32>,
}

struct InstanceInput {
    @location(5) model_matrix_0: vec4<f32>,
    @location(6) model_matrix_1: vec4<f32>,
    @location(7) model_matrix_2: vec4<f32>,
    @location(8) model_matrix_3: vec4<f32>,
    @location(9) normal_matrix_0: vec3<f32>,
    @location(10) normal_matrix_1: vec3<f32>,
    @location(11) normal_matrix_2: vec3<f32>,
}

struct VertexOutput {
    @builtin(position) clip_position: vec4<f32>,
    @location(0) tex_coords: vec2<f32>,
    // The position of the fragment, the light and the camera, all in the
    // tangent space of the surface: the normal map is in that space too, so
    // the lighting can be done without turning the normal into the world.
    @location(1) tangent_position: vec3<f32>,
    @location(2) tangent_light_position: vec3<f32>,
    @location(3) tangent_view_position: vec3<f32>,
    // The same in the world, for what is looked up by a world direction.
    @location(4) world_position: vec3<f32>,
    @location(5) world_normal: vec3<f32>,
    @location(6) world_tangent: vec3<f32>,
    @location(7) world_bitangent: vec3<f32>,
}

@vertex
fn vs_main(model: VertexInput, instance: InstanceInput) -> VertexOutput {
    let model_matrix = mat4x4<f32>(
        instance.model_matrix_0,
        instance.model_matrix_1,
        instance.model_matrix_2,
        instance.model_matrix_3,
    );
    let normal_matrix = mat3x3<f32>(
        instance.normal_matrix_0,
        instance.normal_matrix_1,
        instance.normal_matrix_2,
    );

    // Tangent space -> world space is the matrix of the three axes; the
    // other way round is its inverse, and for an orthonormal basis that is
    // the transpose.
    let world_normal = normalize(normal_matrix * model.normal);
    let world_tangent = normalize(normal_matrix * model.tangent);
    let world_bitangent = normalize(normal_matrix * model.bitangent);
    let tangent_matrix = transpose(mat3x3<f32>(
        world_tangent,
        world_bitangent,
        world_normal,
    ));

    let world_position = model_matrix * vec4<f32>(model.position, 1.0);

    var out: VertexOutput;
    out.clip_position = camera.view_proj * world_position;
    out.tex_coords = model.tex_coords;
    out.tangent_position = tangent_matrix * world_position.xyz;
    out.tangent_view_position = tangent_matrix * camera.view_pos.xyz;
    out.tangent_light_position = tangent_matrix * light.position;
    out.world_position = world_position.xyz;
    out.world_normal = world_normal;
    out.world_tangent = world_tangent;
    out.world_bitangent = world_bitangent;
    return out;
}

@group(0) @binding(0)
var t_diffuse: texture_2d<f32>;
@group(0) @binding(1)
var s_diffuse: sampler;
@group(0) @binding(2)
var t_normal: texture_2d<f32>;
@group(0) @binding(3)
var s_normal: sampler;

@group(3) @binding(0)
var env_map: texture_cube<f32>;
@group(3) @binding(1)
var env_sampler: sampler;

// How much of the sky the surface throws back. The bricks are not mirrors.
const REFLECTIVITY: f32 = 0.1;

@fragment
fn fs_main(in: VertexOutput) -> @location(0) vec4<f32> {
    let object_color: vec4<f32> = textureSample(t_diffuse, s_diffuse, in.tex_coords);
    let object_normal: vec4<f32> = textureSample(t_normal, s_normal, in.tex_coords);

    // Light that comes from everywhere, so nothing is pitch black.
    let ambient_strength = 0.1;
    let ambient_color = light.color * ambient_strength;

    // A normal map stores 0..1 where a direction has -1..1.
    let tangent_normal = normalize(object_normal.xyz * 2.0 - 1.0);
    let light_dir = normalize(in.tangent_light_position - in.tangent_position);
    let view_dir = normalize(in.tangent_view_position - in.tangent_position);
    let half_dir = normalize(view_dir + light_dir);

    // The more the surface faces the light, the brighter it is.
    let diffuse_strength = max(dot(tangent_normal, light_dir), 0.0);
    let diffuse_color = light.color * diffuse_strength;

    // Blinn-Phong: the shine is where the half vector lines up with the normal.
    let specular_strength = pow(max(dot(tangent_normal, half_dir), 0.0), 32.0);
    let specular_color = specular_strength * light.color;

    // The reflection: the view ray bounced off the surface (with the normal
    // of the normal map, taken back to the world), looked up in the sky.
    let tbn = mat3x3<f32>(in.world_tangent, in.world_bitangent, in.world_normal);
    let world_normal = normalize(tbn * tangent_normal);
    let world_view_dir = normalize(camera.view_pos.xyz - in.world_position);
    let reflected = reflect(-world_view_dir, world_normal);
    let reflection = textureSample(env_map, env_sampler, reflected).rgb;

    let result = (ambient_color + diffuse_color + specular_color) * object_color.xyz + reflection * REFLECTIVITY;
    return vec4<f32>(result, object_color.a);
}
