//! `zenith.tonemap`: brings an HDR texture into the range of the screen, into
//! the target of the view.

use bytemuck::{Pod, Zeroable};
use zenith_error::{ZResult, ZenithError};

use crate::{
	cache::{LayoutId, PipelineId, PipelineKey, SamplerKey},
	graph::{
		ExecContext, InitContext, InputDesc, OutputDesc, ParamDesc, ParamMap, PassAction, PassCategory, PassDesc,
		PrepareContext, RenderPass, SlotKind, TextureDesc,
	},
	shaders,
};

const OPERATORS: [&str; 3] = ["aces", "reinhard", "none"];

/// What the shader reads (`Params` of `tonemap.wgsl`).
#[repr(C)]
#[derive(Clone, Copy, Debug, Pod, Zeroable)]
struct Params {
	exposure: f32,
	operator: u32,
	_pad: [u32; 2],
}

const fn layout_entries() -> [wgpu::BindGroupLayoutEntry; 3] {
	[
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
		wgpu::BindGroupLayoutEntry {
			binding: 2,
			visibility: wgpu::ShaderStages::FRAGMENT,
			ty: wgpu::BindingType::Buffer {
				ty: wgpu::BufferBindingType::Uniform,
				has_dynamic_offset: false,
				min_binding_size: wgpu::BufferSize::new(size_of::<Params>() as u64),
			},
			count: None,
		},
	]
}

/// What a view keeps between frames: its own parameter buffer (a buffer
/// written for one view would be seen by all of them).
#[derive(Default)]
struct ViewBuffer(Option<wgpu::Buffer>);

pub struct Tonemap {
	exposure: f32,
	operator: u32,
	layout: Option<LayoutId>,
	// The pass of one view is prepared and recorded before the next view's.
	pipeline: Option<PipelineId>,
	group: Option<wgpu::BindGroup>,
}

impl Default for Tonemap {
	fn default() -> Self {
		Self {
			exposure: 1.0,
			operator: 0,
			layout: None,
			pipeline: None,
			group: None,
		}
	}
}

impl RenderPass for Tonemap {
	fn describe() -> PassDesc {
		PassDesc::new("zenith.tonemap", "Tonemap", PassCategory::Post)
			.input(InputDesc::read("hdr", SlotKind::Color))
			.output(OutputDesc::create("color", SlotKind::Color, TextureDesc::VIEW_TARGET))
			.param(ParamDesc::enumeration("operator", &OPERATORS, "aces"))
			.param(ParamDesc::float("exposure", 1.0).range(0.0, 64.0))
	}

	fn set_params(&mut self, params: &ParamMap) -> ZResult<()> {
		self.exposure = params.float("exposure").unwrap_or(1.0);
		let operator = params.str("operator").unwrap_or("aces");
		self.operator = OPERATORS
			.iter()
			.position(|&name| name == operator)
			.and_then(|index| u32::try_from(index).ok())
			.unwrap_or(0);
		Ok(())
	}

	fn init(&mut self, ctx: &mut InitContext<'_>) -> ZResult<()> {
		self.layout = Some(ctx.cache.layout(&layout_entries()));
		if ctx.cache.named_shader(shaders::TONEMAP).is_none() {
			return Err(ZenithError::Other(format!("no shader {}", shaders::TONEMAP)));
		}
		Ok(())
	}

	fn prepare(&mut self, ctx: &mut PrepareContext<'_>) -> PassAction {
		self.pipeline = None;
		self.group = None;
		let (Some(layout_id), Some(shader)) = (self.layout, ctx.cache.named_shader(shaders::TONEMAP)) else {
			return PassAction::Skip;
		};
		let Some(layout) = ctx.cache.get_layout(layout_id).cloned() else {
			return PassAction::Skip;
		};
		let Some(hdr) = ctx.texture("hdr").cloned() else {
			return PassAction::Skip;
		};
		let params = Params {
			exposure: self.exposure,
			operator: self.operator,
			_pad: [0; 2],
		};
		let device = ctx.device;
		let buffer = ctx
			.view_state::<ViewBuffer>()
			.0
			.get_or_insert_with(|| {
				device.create_buffer(&wgpu::BufferDescriptor {
					label: Some("Zenith Tonemap"),
					size: size_of::<Params>() as u64,
					usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
					mapped_at_creation: false,
				})
			})
			.clone();
		ctx.queue.write_buffer(&buffer, 0, bytemuck::bytes_of(&params));
		let sampler = ctx.cache.sampler(&SamplerKey::LINEAR_CLAMP);
		self.group = Some(ctx.bind_group(
			&layout,
			&[
				wgpu::BindGroupEntry {
					binding: 0,
					resource: wgpu::BindingResource::TextureView(&hdr),
				},
				wgpu::BindGroupEntry {
					binding: 1,
					resource: wgpu::BindingResource::Sampler(&sampler),
				},
				wgpu::BindGroupEntry {
					binding: 2,
					resource: buffer.as_entire_binding(),
				},
			],
		));
		let key = PipelineKey {
			layouts: vec![layout_id],
			..PipelineKey::new(shader, ctx.view.target_format)
		};
		self.pipeline = Some(ctx.cache.pipeline(&key));
		PassAction::Run
	}

	fn exec(&self, ctx: &mut ExecContext<'_>) -> ZResult<()> {
		let cache = ctx.cache;
		let mut pass = ctx
			.begin_render_pass("tonemap", &[("color", wgpu::Color::BLACK)], None)
			.ok_or_else(|| ZenithError::Other("tonemap: the target is not bound".into()))?;
		let (Some(pipeline), Some(group)) = (self.pipeline.and_then(|id| cache.get_pipeline(id)), &self.group) else {
			return Ok(());
		};
		pass.set_pipeline(pipeline);
		pass.set_bind_group(0, group, &[]);
		pass.draw(0..3, 0..1);
		Ok(())
	}
}
