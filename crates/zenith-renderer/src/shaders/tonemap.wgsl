// Brings an HDR picture into the range of the screen.

struct Params {
	exposure: f32,
	// 0: aces, 1: reinhard, 2: none.
	mode: u32,
	_pad0: u32,
	_pad1: u32,
}

@group(0) @binding(0) var hdr: texture_2d<f32>;
@group(0) @binding(1) var hdr_sampler: sampler;
@group(0) @binding(2) var<uniform> params: Params;

struct VertexOutput {
	@builtin(position) clip: vec4<f32>,
	@location(0) uv: vec2<f32>,
}

// One triangle that covers the screen.
@vertex
fn vs_main(@builtin(vertex_index) index: u32) -> VertexOutput {
	let uv = vec2<f32>(f32((index << 1u) & 2u), f32(index & 2u));
	var out: VertexOutput;
	out.clip = vec4<f32>(uv * 2.0 - 1.0, 0.0, 1.0);
	out.uv = vec2<f32>(uv.x, 1.0 - uv.y);
	return out;
}

// Narkowicz's fit of the ACES curve.
fn aces(x: vec3<f32>) -> vec3<f32> {
	return clamp((x * (2.51 * x + 0.03)) / (x * (2.43 * x + 0.59) + 0.14), vec3<f32>(0.0), vec3<f32>(1.0));
}

@fragment
fn fs_main(in: VertexOutput) -> @location(0) vec4<f32> {
	let texel = textureSample(hdr, hdr_sampler, in.uv);
	let color = texel.rgb * params.exposure;
	var mapped = color;
	if (params.mode == 0u) {
		mapped = aces(color);
	} else if (params.mode == 1u) {
		mapped = color / (color + vec3<f32>(1.0));
	}
	return vec4<f32>(mapped, texel.a);
}
