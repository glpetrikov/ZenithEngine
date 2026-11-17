//! `zenith.sprites`: sprites over the color the passes before drew, by
//! `order`, then far to near. They blend and do not write depth; with
//! `depth_test` they are hidden by what is nearer.

use zenith_error::{ZResult, ZenithError};

use super::inputs::{CameraData, SpritesData};
use crate::{
	cache::{LayoutId, PipelineId, PipelineKey},
	graph::{
		ExecContext, InitContext, InputDesc, OutputDesc, ParamDesc, ParamMap, PassAction, PassCategory, PassDesc,
		PrepareContext, RenderPass, SlotKind, pass::data,
	},
	prepare::{CameraUniform, SpriteInstance},
	shaders,
};

const DEPTH_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Depth32Float;

const fn camera_entries() -> [wgpu::BindGroupLayoutEntry; 1] {
	[wgpu::BindGroupLayoutEntry {
		binding: 0,
		visibility: wgpu::ShaderStages::VERTEX,
		ty: wgpu::BindingType::Buffer {
			ty: wgpu::BufferBindingType::Uniform,
			has_dynamic_offset: false,
			min_binding_size: wgpu::BufferSize::new(size_of::<CameraUniform>() as u64),
		},
		count: None,
	}]
}

const fn texture_entries() -> [wgpu::BindGroupLayoutEntry; 2] {
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
	]
}

/// A batch `prepare` decided to draw, for `exec`.
struct Draw {
	group: wgpu::BindGroup,
	instances: std::ops::Range<u32>,
}

pub struct Sprites {
	depth_test: bool,
	camera_layout: Option<LayoutId>,
	texture_layout: Option<LayoutId>,
	// The pass of one view is prepared and recorded before the next view's.
	pipeline: Option<PipelineId>,
	camera_group: Option<wgpu::BindGroup>,
	draws: Vec<Draw>,
}

impl Default for Sprites {
	fn default() -> Self {
		Self {
			depth_test: true,
			camera_layout: None,
			texture_layout: None,
			pipeline: None,
			camera_group: None,
			draws: Vec::new(),
		}
	}
}

impl RenderPass for Sprites {
	fn describe() -> PassDesc {
		PassDesc::new("zenith.sprites", "Sprites", PassCategory::Geometry)
			.input(InputDesc::read("camera", SlotKind::data(data::CAMERA)))
			.input(InputDesc::read("sprites", SlotKind::data(data::SPRITES)))
			.input(InputDesc::write("color", SlotKind::Color))
			.input(InputDesc::write("depth", SlotKind::Depth))
			.output(OutputDesc::pass_through("color", SlotKind::Color, "color"))
			.output(OutputDesc::pass_through("depth", SlotKind::Depth, "depth"))
			.param(ParamDesc::bool("depth_test", true).tooltip("Hide what is behind the meshes drawn before"))
	}

	fn set_params(&mut self, params: &ParamMap) -> ZResult<()> {
		self.depth_test = params.bool("depth_test").unwrap_or(true);
		Ok(())
	}

	fn init(&mut self, ctx: &mut InitContext<'_>) -> ZResult<()> {
		self.camera_layout = Some(ctx.cache.layout(&camera_entries()));
		self.texture_layout = Some(ctx.cache.layout(&texture_entries()));
		if ctx.cache.named_shader(shaders::SPRITE).is_none() {
			return Err(ZenithError::Other(format!("no shader {}", shaders::SPRITE)));
		}
		Ok(())
	}

	fn prepare(&mut self, ctx: &mut PrepareContext<'_>) -> PassAction {
		self.pipeline = None;
		self.camera_group = None;
		self.draws.clear();
		let shader = ctx.cache.named_shader(shaders::SPRITE);
		let (Some(camera), Some(sprites), Some(camera_id), Some(texture_id), Some(shader), Some(format)) = (
			ctx.data::<CameraData>("camera").cloned(),
			ctx.data::<SpritesData>("sprites").cloned(),
			self.camera_layout,
			self.texture_layout,
			shader,
			ctx.texture_format("color"),
		) else {
			return PassAction::Skip;
		};
		let (Some(camera_layout), Some(texture_layout)) = (
			ctx.cache.get_layout(camera_id).cloned(),
			ctx.cache.get_layout(texture_id).cloned(),
		) else {
			return PassAction::Skip;
		};
		self.camera_group = Some(ctx.bind_group(
			&camera_layout,
			&[wgpu::BindGroupEntry {
				binding: 0,
				resource: camera.buffer.as_entire_binding(),
			}],
		));
		for batch in &sprites.list.batches {
			// Still loading: not drawn yet.
			let Some(texture) = ctx.cache.texture(batch.texture) else {
				continue;
			};
			let (view, sampler) = (texture.view.clone(), texture.sampler.clone());
			let group = ctx.bind_group(
				&texture_layout,
				&[
					wgpu::BindGroupEntry {
						binding: 0,
						resource: wgpu::BindingResource::TextureView(&view),
					},
					wgpu::BindGroupEntry {
						binding: 1,
						resource: wgpu::BindingResource::Sampler(&sampler),
					},
				],
			);
			self.draws.push(Draw {
				group,
				instances: batch.instances.clone(),
			});
		}
		let key = PipelineKey {
			layouts: vec![camera_id, texture_id],
			vertex_buffers: vec![SpriteInstance::LAYOUT],
			targets: vec![Some(wgpu::ColorTargetState {
				format,
				blend: Some(wgpu::BlendState::ALPHA_BLENDING),
				write_mask: wgpu::ColorWrites::ALL,
			})],
			depth_stencil: Some(wgpu::DepthStencilState {
				format: DEPTH_FORMAT,
				depth_write_enabled: Some(false),
				depth_compare: Some(if self.depth_test {
					wgpu::CompareFunction::LessEqual
				} else {
					wgpu::CompareFunction::Always
				}),
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
		let instances = ctx
			.data::<SpritesData>("sprites")
			.and_then(|sprites| sprites.instances.as_ref());
		let mut pass = ctx
			.begin_render_pass("sprites", &[("color", wgpu::Color::BLACK)], Some("depth"))
			.ok_or_else(|| ZenithError::Other("sprites: the color or the depth is not bound".into()))?;
		let (Some(pipeline), Some(camera_group), Some(instances)) = (
			self.pipeline.and_then(|id| cache.get_pipeline(id)),
			&self.camera_group,
			instances,
		) else {
			return Ok(());
		};
		pass.set_pipeline(pipeline);
		pass.set_bind_group(0, camera_group, &[]);
		pass.set_vertex_buffer(0, instances.slice(..));
		for draw in &self.draws {
			pass.set_bind_group(1, &draw.group, &[]);
			pass.draw(0..6, draw.instances.clone());
		}
		Ok(())
	}
}
