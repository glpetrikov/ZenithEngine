//! `zenith.sky`: the environment picture behind everything the passes before
//! drew. It is drawn where the depth is still the far plane.

use zenith_error::{ZResult, ZenithError};

use super::inputs::{CameraData, EnvironmentData};
use crate::{
	cache::{LayoutId, PipelineId, PipelineKey, SamplerKey},
	graph::{
		ExecContext, InitContext, InputDesc, OutputDesc, ParamDesc, ParamMap, PassAction, PassCategory, PassDesc,
		PrepareContext, RenderPass, SlotKind, pass::data,
	},
	shaders,
};

const DEPTH_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Depth32Float;

/// What the shader reads (`Params` of `sky.wgsl`).
#[repr(C)]
#[derive(Clone, Copy, Debug, bytemuck::Pod, bytemuck::Zeroable)]
struct Params {
	intensity: f32,
	_pad: [f32; 3],
}

fn layout_entries() -> [wgpu::BindGroupLayoutEntry; 4] {
	let uniform = |binding, size: usize| wgpu::BindGroupLayoutEntry {
		binding,
		visibility: wgpu::ShaderStages::VERTEX_FRAGMENT,
		ty: wgpu::BindingType::Buffer {
			ty: wgpu::BufferBindingType::Uniform,
			has_dynamic_offset: false,
			min_binding_size: wgpu::BufferSize::new(size as u64),
		},
		count: None,
	};
	[
		uniform(0, size_of::<crate::prepare::CameraUniform>()),
		uniform(1, size_of::<Params>()),
		wgpu::BindGroupLayoutEntry {
			binding: 2,
			visibility: wgpu::ShaderStages::FRAGMENT,
			ty: wgpu::BindingType::Texture {
				sample_type: wgpu::TextureSampleType::Float { filterable: true },
				view_dimension: wgpu::TextureViewDimension::D2,
				multisampled: false,
			},
			count: None,
		},
		wgpu::BindGroupLayoutEntry {
			binding: 3,
			visibility: wgpu::ShaderStages::FRAGMENT,
			ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
			count: None,
		},
	]
}

/// What a view keeps between frames: its own parameter buffer.
#[derive(Default)]
struct ViewParams(Option<wgpu::Buffer>);

pub struct Sky {
	intensity: f32,
	layout: Option<LayoutId>,
	// The pass of one view is prepared and recorded before the next view's.
	pipeline: Option<PipelineId>,
	group: Option<wgpu::BindGroup>,
}

impl Default for Sky {
	fn default() -> Self {
		Self {
			intensity: 1.0,
			layout: None,
			pipeline: None,
			group: None,
		}
	}
}

impl RenderPass for Sky {
	fn describe() -> PassDesc {
		PassDesc::new("zenith.sky", "Sky", PassCategory::Geometry)
			.input(InputDesc::read("camera", SlotKind::data(data::CAMERA)))
			.input(InputDesc::read("environment", SlotKind::data(data::ENVIRONMENT)))
			.input(InputDesc::write("color", SlotKind::Color))
			.input(InputDesc::read("depth", SlotKind::Depth))
			.output(OutputDesc::pass_through("color", SlotKind::Color, "color"))
			.param(
				ParamDesc::float("intensity", 1.0)
					.range(0.0, 64.0)
					.tooltip("A multiplier of the sky"),
			)
	}

	fn set_params(&mut self, params: &ParamMap) -> ZResult<()> {
		self.intensity = params.float("intensity").unwrap_or(1.0);
		Ok(())
	}

	fn init(&mut self, ctx: &mut InitContext<'_>) -> ZResult<()> {
		self.layout = Some(ctx.cache.layout(&layout_entries()));
		if ctx.cache.named_shader(shaders::SKY).is_none() {
			return Err(ZenithError::Other(format!("no shader {}", shaders::SKY)));
		}
		Ok(())
	}

	fn prepare(&mut self, ctx: &mut PrepareContext<'_>) -> PassAction {
		self.pipeline = None;
		self.group = None;
		let (Some(camera), Some(environment), Some(layout_id), Some(shader), Some(format)) = (
			ctx.data::<CameraData>("camera").cloned(),
			ctx.data::<EnvironmentData>("environment").cloned(),
			self.layout,
			ctx.cache.named_shader(shaders::SKY),
			ctx.texture_format("color"),
		) else {
			return PassAction::Skip;
		};
		// The sky is not loaded yet: nothing to draw.
		let Some(texture) = environment.texture else {
			return PassAction::Skip;
		};
		let Some(layout) = ctx.cache.get_layout(layout_id).cloned() else {
			return PassAction::Skip;
		};
		let params = Params {
			intensity: self.intensity * environment.intensity,
			_pad: [0.0; 3],
		};
		let device = ctx.device;
		let buffer = ctx
			.view_state::<ViewParams>()
			.0
			.get_or_insert_with(|| {
				device.create_buffer(&wgpu::BufferDescriptor {
					label: Some("Zenith Sky"),
					size: size_of::<Params>() as u64,
					usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
					mapped_at_creation: false,
				})
			})
			.clone();
		ctx.queue.write_buffer(&buffer, 0, bytemuck::bytes_of(&params));
		let sampler = ctx.cache.sampler(&SamplerKey {
			address_mode: wgpu::AddressMode::Repeat,
			..SamplerKey::LINEAR_CLAMP
		});
		self.group = Some(ctx.bind_group(
			&layout,
			&[
				wgpu::BindGroupEntry {
					binding: 0,
					resource: camera.buffer.as_entire_binding(),
				},
				wgpu::BindGroupEntry {
					binding: 1,
					resource: buffer.as_entire_binding(),
				},
				wgpu::BindGroupEntry {
					binding: 2,
					resource: wgpu::BindingResource::TextureView(&texture),
				},
				wgpu::BindGroupEntry {
					binding: 3,
					resource: wgpu::BindingResource::Sampler(&sampler),
				},
			],
		));
		let key = PipelineKey {
			layouts: vec![layout_id],
			depth_stencil: Some(wgpu::DepthStencilState {
				format: DEPTH_FORMAT,
				depth_write_enabled: Some(false),
				depth_compare: Some(wgpu::CompareFunction::LessEqual),
				stencil: wgpu::StencilState::default(),
				bias: wgpu::DepthBiasState::default(),
			}),
			..PipelineKey::new(shader, format)
		};
		self.pipeline = Some(ctx.cache.pipeline(&key));
		PassAction::Run
	}

	fn exec(&self, ctx: &mut ExecContext<'_>) -> ZResult<()> {
		let cache = ctx.cache;
		let mut pass = ctx
			.begin_render_pass("sky", &[("color", wgpu::Color::BLACK)], Some("depth"))
			.ok_or_else(|| ZenithError::Other("sky: the color or the depth is not bound".into()))?;
		let (Some(pipeline), Some(group)) = (self.pipeline.and_then(|id| cache.get_pipeline(id)), &self.group) else {
			return Ok(());
		};
		pass.set_pipeline(pipeline);
		pass.set_bind_group(0, group, &[]);
		pass.draw(0..3, 0..1);
		Ok(())
	}
}
