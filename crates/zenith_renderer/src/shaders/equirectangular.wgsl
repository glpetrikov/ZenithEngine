// Turns an equirectangular picture (the whole sky on one rectangle: the
// longitude goes along, the latitude down) into the six faces of a cube map.

const PI: f32 = 3.14159265358979323846;
// Half floats top out at 65504; the sun of a real photo can be brighter.
const MAX_HALF: f32 = 65000.0;

@group(0) @binding(0)
var src: texture_2d<f32>;
@group(0) @binding(1)
var dst: texture_storage_2d_array<rgba16float, write>;

// The direction a texel of a cube face looks at, `uv` going from -1 to 1 with
// `v` downwards in the image, as the cube map of a GPU is laid out: the same
// rule that turns a direction into a face when the cube is sampled.
fn face_direction(face: u32, uv: vec2<f32>) -> vec3<f32> {
    switch face {
        case 0u: { return vec3<f32>(1.0, -uv.y, -uv.x); }  // +X
        case 1u: { return vec3<f32>(-1.0, -uv.y, uv.x); }  // -X
        case 2u: { return vec3<f32>(uv.x, 1.0, uv.y); }    // +Y
        case 3u: { return vec3<f32>(uv.x, -1.0, -uv.y); }  // -Y
        case 4u: { return vec3<f32>(uv.x, -uv.y, 1.0); }   // +Z
        default: { return vec3<f32>(-uv.x, -uv.y, -1.0); } // -Z
    }
}

// Bilinear by hand: a compute shader cannot `textureSample`, and the source
// is a 32-bit float texture, which is not filterable anyway. The picture wraps
// around in `x` (it is a circle) and stops at the poles.
fn load_bilinear(uv: vec2<f32>) -> vec3<f32> {
    let size = vec2<i32>(textureDimensions(src));
    let p = uv * vec2<f32>(size) - 0.5;
    let base = vec2<i32>(floor(p));
    let f = fract(p);

    let x0 = ((base.x % size.x) + size.x) % size.x;
    let x1 = (x0 + 1) % size.x;
    let y0 = clamp(base.y, 0, size.y - 1);
    let y1 = clamp(base.y + 1, 0, size.y - 1);

    let a = textureLoad(src, vec2<i32>(x0, y0), 0).rgb;
    let b = textureLoad(src, vec2<i32>(x1, y0), 0).rgb;
    let c = textureLoad(src, vec2<i32>(x0, y1), 0).rgb;
    let d = textureLoad(src, vec2<i32>(x1, y1), 0).rgb;
    return mix(mix(a, b, f.x), mix(c, d, f.x), f.y);
}

@compute @workgroup_size(16, 16, 1)
fn compute_equirect_to_cubemap(@builtin(global_invocation_id) gid: vec3<u32>) {
    let size = textureDimensions(dst);
    // The groups are 16 wide; the cube is not always a multiple of that.
    if gid.x >= size.x || gid.y >= size.y {
        return;
    }

    // The middle of the texel, from -1 to 1.
    let uv = (vec2<f32>(gid.xy) + 0.5) / vec2<f32>(size) * 2.0 - 1.0;
    let dir = normalize(face_direction(gid.z, uv));

    // Longitude from `atan2` (0 to 1 along the picture), latitude from `asin`
    // (the zenith is the top row).
    let eq_uv = vec2<f32>(
        atan2(dir.z, dir.x) / (2.0 * PI) + 0.5,
        0.5 - asin(dir.y) / PI,
    );

    let color = min(load_bilinear(eq_uv), vec3<f32>(MAX_HALF));
    textureStore(dst, gid.xy, gid.z, vec4<f32>(color, 1.0));
}
