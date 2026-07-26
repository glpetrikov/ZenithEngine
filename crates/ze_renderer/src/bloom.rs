use crate::{
	backend::pipeline::{self, Pipeline},
	render_target::{RenderTarget, RenderTargetDescriptor},
};

/// Additive blend for the upsample pass: each upsample step adds onto whatever
/// the downsample pass already wrote at that mip, rather than overwriting it --
/// that accumulation across scales is what gives the "sum of multiple blur
/// radii" bloom look.
const ADDITIVE_BLEND: wgpu::BlendState = wgpu::BlendState {
	color: wgpu::BlendComponent {
		src_factor: wgpu::BlendFactor::One,
		dst_factor: wgpu::BlendFactor::One,
		operation: wgpu::BlendOperation::Add,
	},
	alpha: wgpu::BlendComponent {
		src_factor: wgpu::BlendFactor::One,
		dst_factor: wgpu::BlendFactor::One,
		operation: wgpu::BlendOperation::Add,
	},
};

/// The bloom chain starts at half the viewport's resolution (the CoD/Jimenez
/// convention -- there's no full-res mip in the chain itself, since the first
/// downsample pass already halves), then keeps halving until a dimension would
/// drop below ~8px, clamped to a sane maximum.
fn compute_bloom_mip_count(viewport_width: u32, viewport_height: u32) -> u32 {
	let chain_width = (viewport_width / 2).max(1);
	let chain_height = (viewport_height / 2).max(1);
	let smaller_dim = chain_width.min(chain_height);

	let mip_count = if smaller_dim < 8 {
		1
	} else {
		(smaller_dim / 8).ilog2() + 1
	};

	mip_count.clamp(1, 7)
}

/// Owns the mip-chain texture, both directions' pipelines/sampler/layout (built
/// once, independent of viewport size), and the per-mip bind groups (rebuilt on
/// resize, since they reference specific `TextureView` identities that change
/// whenever the chain or the `GBuffer` emissive attachment is recreated).
pub struct BloomChain {
	downsample_pipeline: Pipeline,
	upsample_pipeline: Pipeline,
	sampler: wgpu::Sampler,
	bind_group_layout: wgpu::BindGroupLayout,
	chain: RenderTarget,
	mip_count: u32,
	/// `[0]` sources the external `GBuffer` emissive view; `[i]` for `i >= 1`
	/// sources `chain.mip_view(i - 1)`.
	downsample_bind_groups: Vec<wgpu::BindGroup>,
	/// `[i]` sources `chain.mip_view(i + 1)` -- the smaller, already-processed
	/// neighbor -- and is used when upsampling *into* `chain.mip_view(i)`.
	upsample_bind_groups: Vec<wgpu::BindGroup>,
}

impl BloomChain {
	pub fn new(
		device: &wgpu::Device,
		size: winit::dpi::PhysicalSize<u32>,
		gbuffer_emissive_view: &wgpu::TextureView,
	) -> ze_core::Result<Self> {
		let bind_group_layout = Self::create_bind_group_layout(device);
		let sampler = Self::create_sampler(device);
		let downsample_pipeline = Self::create_downsample_pipeline(device, &bind_group_layout)?;
		let upsample_pipeline = Self::create_upsample_pipeline(device, &bind_group_layout)?;
		let (chain, mip_count, downsample_bind_groups, upsample_bind_groups) =
			Self::build_chain(device, size, gbuffer_emissive_view, &bind_group_layout, &sampler);

		Ok(Self {
			downsample_pipeline,
			upsample_pipeline,
			sampler,
			bind_group_layout,
			chain,
			mip_count,
			downsample_bind_groups,
			upsample_bind_groups,
		})
	}

	/// Rebuilds the chain texture and both bind-group sets. Pipelines/sampler/
	/// layout are untouched -- their shapes don't depend on viewport size, and
	/// this runs every frame while a user drags the Editor's viewport-panel
	/// divider, so recreating pipelines that often would be wasteful.
	pub fn resize(
		&mut self,
		device: &wgpu::Device,
		size: winit::dpi::PhysicalSize<u32>,
		gbuffer_emissive_view: &wgpu::TextureView,
	) {
		let (chain, mip_count, downsample_bind_groups, upsample_bind_groups) = Self::build_chain(
			device,
			size,
			gbuffer_emissive_view,
			&self.bind_group_layout,
			&self.sampler,
		);
		self.chain = chain;
		self.mip_count = mip_count;
		self.downsample_bind_groups = downsample_bind_groups;
		self.upsample_bind_groups = upsample_bind_groups;
	}

	#[allow(clippy::type_complexity)]
	fn build_chain(
		device: &wgpu::Device,
		size: winit::dpi::PhysicalSize<u32>,
		gbuffer_emissive_view: &wgpu::TextureView,
		bind_group_layout: &wgpu::BindGroupLayout,
		sampler: &wgpu::Sampler,
	) -> (RenderTarget, u32, Vec<wgpu::BindGroup>, Vec<wgpu::BindGroup>) {
		let mip_count = compute_bloom_mip_count(size.width, size.height);
		let chain_width = (size.width / 2).max(1);
		let chain_height = (size.height / 2).max(1);

		let chain = RenderTarget::new(
			device,
			&RenderTargetDescriptor {
				label: "Bloom Chain",
				width: chain_width,
				height: chain_height,
				format: wgpu::TextureFormat::Rgba16Float,
				mip_level_count: mip_count,
				usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::TEXTURE_BINDING,
			},
		);

		let downsample_bind_groups = (0..mip_count)
			.map(|level| {
				let source = if level == 0 {
					gbuffer_emissive_view
				} else {
					chain.mip_view(level - 1)
				};
				Self::create_bind_group(
					device,
					bind_group_layout,
					source,
					sampler,
					"Bloom Downsample Bind Group",
				)
			})
			.collect();

		let upsample_bind_groups = (0..mip_count.saturating_sub(1))
			.map(|level| {
				Self::create_bind_group(
					device,
					bind_group_layout,
					chain.mip_view(level + 1),
					sampler,
					"Bloom Upsample Bind Group",
				)
			})
			.collect();

		(chain, mip_count, downsample_bind_groups, upsample_bind_groups)
	}

	/// Records the full downsample + upsample chain into `encoder`. Caller owns
	/// the encoder's lifetime/submission -- this never creates or submits its
	/// own.
	pub fn record(&self, encoder: &mut wgpu::CommandEncoder) {
		self.record_downsample(encoder);
		self.record_upsample(encoder);
	}

	fn record_downsample(&self, encoder: &mut wgpu::CommandEncoder) {
		for level in 0..self.mip_count {
			let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
				label: Some("Bloom Downsample Pass"),
				color_attachments: &[Some(wgpu::RenderPassColorAttachment {
					view: self.chain.mip_view(level),
					resolve_target: None,
					depth_slice: None,
					ops: wgpu::Operations {
						load: wgpu::LoadOp::Clear(wgpu::Color::BLACK),
						store: wgpu::StoreOp::Store,
					},
				})],
				depth_stencil_attachment: None,
				occlusion_query_set: None,
				timestamp_writes: None,
				multiview_mask: None,
			});
			pass.set_pipeline(&self.downsample_pipeline.render_pipeline);
			pass.set_bind_group(0, &self.downsample_bind_groups[level as usize], &[]);
			pass.draw(0..3, 0..1);
		}
	}

	fn record_upsample(&self, encoder: &mut wgpu::CommandEncoder) {
		if self.mip_count < 2 {
			return;
		}
		for level in (0..self.mip_count - 1).rev() {
			let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
				label: Some("Bloom Upsample Pass"),
				color_attachments: &[Some(wgpu::RenderPassColorAttachment {
					view: self.chain.mip_view(level),
					resolve_target: None,
					depth_slice: None,
					ops: wgpu::Operations {
						load: wgpu::LoadOp::Load,
						store: wgpu::StoreOp::Store,
					},
				})],
				depth_stencil_attachment: None,
				occlusion_query_set: None,
				timestamp_writes: None,
				multiview_mask: None,
			});
			pass.set_pipeline(&self.upsample_pipeline.render_pipeline);
			pass.set_bind_group(0, &self.upsample_bind_groups[level as usize], &[]);
			pass.draw(0..3, 0..1);
		}
	}

	/// Mip 0 of the chain, fully accumulated after downsample+upsample. This is
	/// what the composite pass samples.
	pub fn result_view(&self) -> &wgpu::TextureView { self.chain.mip_view(0) }

	fn create_bind_group_layout(device: &wgpu::Device) -> wgpu::BindGroupLayout {
		device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
			label: Some("Bloom Bind Group Layout"),
			entries: &[
				wgpu::BindGroupLayoutEntry {
					binding: 0,
					visibility: wgpu::ShaderStages::FRAGMENT,
					ty: wgpu::BindingType::Texture {
						sample_type: wgpu::TextureSampleType::Float { filterable: true },
						view_dimension: wgpu::TextureViewDimension::D2,
						multisampled: false,
					},
					count: None,
				},
				wgpu::BindGroupLayoutEntry {
					binding: 1,
					visibility: wgpu::ShaderStages::FRAGMENT,
					ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
					count: None,
				},
			],
		})
	}

	fn create_sampler(device: &wgpu::Device) -> wgpu::Sampler {
		device.create_sampler(&wgpu::SamplerDescriptor {
			label: Some("Bloom Sampler"),
			address_mode_u: wgpu::AddressMode::ClampToEdge,
			address_mode_v: wgpu::AddressMode::ClampToEdge,
			address_mode_w: wgpu::AddressMode::ClampToEdge,
			mag_filter: wgpu::FilterMode::Linear,
			min_filter: wgpu::FilterMode::Linear,
			mipmap_filter: wgpu::MipmapFilterMode::Nearest,
			..wgpu::SamplerDescriptor::default()
		})
	}

	fn create_bind_group(
		device: &wgpu::Device,
		layout: &wgpu::BindGroupLayout,
		source_view: &wgpu::TextureView,
		sampler: &wgpu::Sampler,
		label: &str,
	) -> wgpu::BindGroup {
		device.create_bind_group(&wgpu::BindGroupDescriptor {
			label: Some(label),
			layout,
			entries: &[
				wgpu::BindGroupEntry {
					binding: 0,
					resource: wgpu::BindingResource::TextureView(source_view),
				},
				wgpu::BindGroupEntry {
					binding: 1,
					resource: wgpu::BindingResource::Sampler(sampler),
				},
			],
		})
	}

	fn create_downsample_pipeline(device: &wgpu::Device, layout: &wgpu::BindGroupLayout) -> ze_core::Result<Pipeline> {
		pipeline::Builder::new(device)
			.with_name("Bloom Downsample")
			.with_shader_source(BLOOM_DOWNSAMPLE_SHADER)
			.with_pixel_format(wgpu::TextureFormat::Rgba16Float)
			.with_blend_state(None)
			.with_bind_group_layout(layout)
			.with_depth_stencil_enabled(false)
			.build()
	}

	fn create_upsample_pipeline(device: &wgpu::Device, layout: &wgpu::BindGroupLayout) -> ze_core::Result<Pipeline> {
		pipeline::Builder::new(device)
			.with_name("Bloom Upsample")
			.with_shader_source(BLOOM_UPSAMPLE_SHADER)
			.with_pixel_format(wgpu::TextureFormat::Rgba16Float)
			.with_blend_state(Some(ADDITIVE_BLEND))
			.with_bind_group_layout(layout)
			.with_depth_stencil_enabled(false)
			.build()
	}
}

/// 13-tap "Next Generation Post Processing in Call of Duty: Advanced Warfare"
/// downsample filter (Jimenez, SIGGRAPH 2014). Four of the taps sit at
/// half-texel offsets so hardware bilinear filtering secretly averages a 2x2
/// box per tap -- this wide filter is what suppresses fireflies/shimmer, far
/// better than a naive 4-tap box average, with no brightness threshold needed.
const BLOOM_DOWNSAMPLE_SHADER: &str = r"
@group(0) @binding(0) var source_texture: texture_2d<f32>;
@group(0) @binding(1) var source_sampler: sampler;

struct VertexOutput {
    @builtin(position) position: vec4<f32>,
    @location(0) uv: vec2<f32>,
}

@vertex
fn vs_main(@builtin(vertex_index) vertex_index: u32) -> VertexOutput {
    var out: VertexOutput;
    let positions = array<vec2<f32>, 3>(
        vec2<f32>(-1.0, -1.0),
        vec2<f32>(3.0, -1.0),
        vec2<f32>(-1.0, 3.0),
    );
    let pos = positions[vertex_index];
    out.position = vec4<f32>(pos, 0.0, 1.0);
    out.uv = vec2<f32>(pos.x * 0.5 + 0.5, 1.0 - (pos.y * 0.5 + 0.5));
    return out;
}

@fragment
fn fs_main(in: VertexOutput) -> @location(0) vec4<f32> {
    let texel_size = 1.0 / vec2<f32>(textureDimensions(source_texture));
    let uv = in.uv;

    let a = textureSample(source_texture, source_sampler, uv + texel_size * vec2<f32>(-1.0, -1.0)).rgb;
    let b = textureSample(source_texture, source_sampler, uv + texel_size * vec2<f32>( 0.0, -1.0)).rgb;
    let c = textureSample(source_texture, source_sampler, uv + texel_size * vec2<f32>( 1.0, -1.0)).rgb;
    let d = textureSample(source_texture, source_sampler, uv + texel_size * vec2<f32>(-0.5, -0.5)).rgb;
    let e = textureSample(source_texture, source_sampler, uv + texel_size * vec2<f32>( 0.5, -0.5)).rgb;
    let f = textureSample(source_texture, source_sampler, uv + texel_size * vec2<f32>(-1.0,  0.0)).rgb;
    let g = textureSample(source_texture, source_sampler, uv).rgb;
    let h = textureSample(source_texture, source_sampler, uv + texel_size * vec2<f32>( 1.0,  0.0)).rgb;
    let i = textureSample(source_texture, source_sampler, uv + texel_size * vec2<f32>(-0.5,  0.5)).rgb;
    let j = textureSample(source_texture, source_sampler, uv + texel_size * vec2<f32>( 0.5,  0.5)).rgb;
    let k = textureSample(source_texture, source_sampler, uv + texel_size * vec2<f32>(-1.0,  1.0)).rgb;
    let l = textureSample(source_texture, source_sampler, uv + texel_size * vec2<f32>( 0.0,  1.0)).rgb;
    let m = textureSample(source_texture, source_sampler, uv + texel_size * vec2<f32>( 1.0,  1.0)).rgb;

    var result = vec3<f32>(0.0);
    result += (d + e + i + j) * (0.5 / 4.0);   // inner 2x2 box, total weight 0.5
    result += (a + b + f + g) * (0.125 / 4.0); // 4 outer corner-anchored 2x2 boxes,
    result += (b + c + g + h) * (0.125 / 4.0); // each weight 0.125, summing to 0.5.
    result += (f + g + k + l) * (0.125 / 4.0); // Total: 1.0
    result += (g + h + l + m) * (0.125 / 4.0);

    return vec4<f32>(result, 1.0);
}
";

/// 9-tap tent filter (3x3, weights 1/2/4 normalized by 16), sampling the
/// smaller (already-processed) neighbor mip and additively blended by the
/// pipeline onto whatever the downsample pass already wrote at this mip.
const BLOOM_UPSAMPLE_SHADER: &str = r"
@group(0) @binding(0) var source_texture: texture_2d<f32>;
@group(0) @binding(1) var source_sampler: sampler;

struct VertexOutput {
    @builtin(position) position: vec4<f32>,
    @location(0) uv: vec2<f32>,
}

@vertex
fn vs_main(@builtin(vertex_index) vertex_index: u32) -> VertexOutput {
    var out: VertexOutput;
    let positions = array<vec2<f32>, 3>(
        vec2<f32>(-1.0, -1.0),
        vec2<f32>(3.0, -1.0),
        vec2<f32>(-1.0, 3.0),
    );
    let pos = positions[vertex_index];
    out.position = vec4<f32>(pos, 0.0, 1.0);
    out.uv = vec2<f32>(pos.x * 0.5 + 0.5, 1.0 - (pos.y * 0.5 + 0.5));
    return out;
}

@fragment
fn fs_main(in: VertexOutput) -> @location(0) vec4<f32> {
    let texel_size = 1.0 / vec2<f32>(textureDimensions(source_texture));
    let uv = in.uv;

    let top_left     = textureSample(source_texture, source_sampler, uv + texel_size * vec2<f32>(-1.0,  1.0)).rgb;
    let top          = textureSample(source_texture, source_sampler, uv + texel_size * vec2<f32>( 0.0,  1.0)).rgb;
    let top_right    = textureSample(source_texture, source_sampler, uv + texel_size * vec2<f32>( 1.0,  1.0)).rgb;
    let left         = textureSample(source_texture, source_sampler, uv + texel_size * vec2<f32>(-1.0,  0.0)).rgb;
    let center       = textureSample(source_texture, source_sampler, uv).rgb;
    let right        = textureSample(source_texture, source_sampler, uv + texel_size * vec2<f32>( 1.0,  0.0)).rgb;
    let bottom_left  = textureSample(source_texture, source_sampler, uv + texel_size * vec2<f32>(-1.0, -1.0)).rgb;
    let bottom       = textureSample(source_texture, source_sampler, uv + texel_size * vec2<f32>( 0.0, -1.0)).rgb;
    let bottom_right = textureSample(source_texture, source_sampler, uv + texel_size * vec2<f32>( 1.0, -1.0)).rgb;

    var result = center * 4.0;
    result += (top + bottom + left + right) * 2.0;
    result += (top_left + top_right + bottom_left + bottom_right) * 1.0;
    result *= 1.0 / 16.0; // 4 + 4*2 + 4*1 = 16 -- normalized.

    return vec4<f32>(result, 1.0);
}
";
