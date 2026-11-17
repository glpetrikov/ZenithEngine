//! What the passes that draw meshes share: group 0 (the camera, the lights,
//! the environment, the shadow map) and the way a list of batches is drawn.
//!
//! WebGPU allows four bind groups, so what a graph gives a pass is put
//! together here, in one: a pass that is not given the lights, the
//! environment or the shadow map gets a neutral stand-in for it, and the one
//! shader serves every case.

use std::ops::Range;

use bytemuck::{Pod, Zeroable};
use zenith_error::{ZResult, ZenithError};

use super::{
	inputs::{CameraData, EnvironmentData, LightsData, MeshesData},
	shadow::ShadowInfo,
};
use crate::{
	assets::{AssetId, json::BlendMode},
	cache::{GpuMesh, LayoutId, PipelineId, PipelineKey, SamplerKey},
	graph::{ExecContext, InitContext, PassAction, PrepareContext},
	prepare::{CameraUniform, InstanceData, LightsUniform},
	shaders,
};

const DEPTH_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Depth32Float;

/// The layout of group 0 of the passes that draw meshes.
pub fn scene_layout_entries() -> [wgpu::BindGroupLayoutEntry; 7] {
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
		uniform(0, size_of::<CameraUniform>()),
		uniform(1, size_of::<LightsUniform>()),
		uniform(2, size_of::<SceneUniform>()),
		wgpu::BindGroupLayoutEntry {
			binding: 3,
			visibility: wgpu::ShaderStages::FRAGMENT,
			ty: wgpu::BindingType::Texture {
				sample_type: wgpu::TextureSampleType::Float { filterable: true },
				view_dimension: wgpu::TextureViewDimension::D2,
				multisampled: false,
			},
			count: None,
		},
		wgpu::BindGroupLayoutEntry {
			binding: 4,
			visibility: wgpu::ShaderStages::FRAGMENT,
			ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
			count: None,
		},
		wgpu::BindGroupLayoutEntry {
			binding: 5,
			visibility: wgpu::ShaderStages::FRAGMENT,
			ty: wgpu::BindingType::Texture {
				sample_type: wgpu::TextureSampleType::Depth,
				view_dimension: wgpu::TextureViewDimension::D2,
				multisampled: false,
			},
			count: None,
		},
		wgpu::BindGroupLayoutEntry {
			binding: 6,
			visibility: wgpu::ShaderStages::FRAGMENT,
			ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Comparison),
			count: None,
		},
	]
}

/// `Scene` of `standard.wgsl`.
#[repr(C)]
#[derive(Clone, Copy, Debug, Default, PartialEq, Pod, Zeroable)]
pub struct SceneUniform {
	pub shadow_matrix: [[f32; 4]; 4],
	/// The intensity of the environment, 1 if there is one, 1 if there is a
	/// shadow map, the last mip level of the environment.
	pub params: [f32; 4],
	/// The bias, the size of a texel of the shadow map in 0..1.
	pub shadow: [f32; 4],
}

/// The stand-ins for what a graph does not give.
struct Defaults {
	environment: wgpu::TextureView,
	shadow: wgpu::TextureView,
	lights: wgpu::Buffer,
}

/// What a view keeps between frames: its own scene buffer.
#[derive(Default)]
struct ViewScene(Option<wgpu::Buffer>);

/// Group 0, made again every frame for every view.
#[derive(Default)]
pub struct SceneBindings {
	layout: Option<LayoutId>,
	defaults: Option<Defaults>,
	pub group: Option<wgpu::BindGroup>,
}

const fn one_pixel(format: wgpu::TextureFormat, usage: wgpu::TextureUsages) -> wgpu::TextureDescriptor<'static> {
	wgpu::TextureDescriptor {
		label: Some("Zenith Stand-in"),
		size: wgpu::Extent3d {
			width: 1,
			height: 1,
			depth_or_array_layers: 1,
		},
		mip_level_count: 1,
		sample_count: 1,
		dimension: wgpu::TextureDimension::D2,
		format,
		usage,
		view_formats: &[],
	}
}

impl SceneBindings {
	pub const fn layout(&self) -> Option<LayoutId> {
		self.layout
	}

	pub fn init(&mut self, ctx: &mut InitContext<'_>) {
		self.layout = Some(ctx.cache.layout(&scene_layout_entries()));
		// A new texture reads as zeros: a black environment.
		let environment = ctx.device.create_texture(&one_pixel(
			wgpu::TextureFormat::Rgba16Float,
			wgpu::TextureUsages::TEXTURE_BINDING,
		));
		let shadow = ctx.device.create_texture(&one_pixel(
			DEPTH_FORMAT,
			wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::RENDER_ATTACHMENT,
		));
		let shadow_view = shadow.create_view(&wgpu::TextureViewDescriptor::default());
		// A depth texture cannot be filled by a copy: a pass clears it.
		let mut encoder = ctx
			.device
			.create_command_encoder(&wgpu::CommandEncoderDescriptor::default());
		encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
			label: Some("Zenith No Shadow"),
			color_attachments: &[],
			depth_stencil_attachment: Some(wgpu::RenderPassDepthStencilAttachment {
				view: &shadow_view,
				depth_ops: Some(wgpu::Operations {
					load: wgpu::LoadOp::Clear(1.0),
					store: wgpu::StoreOp::Store,
				}),
				stencil_ops: None,
			}),
			occlusion_query_set: None,
			timestamp_writes: None,
			multiview_mask: None,
		});
		ctx.queue.submit([encoder.finish()]);
		let lights = ctx.device.create_buffer(&wgpu::BufferDescriptor {
			label: Some("Zenith No Lights"),
			size: size_of::<LightsUniform>() as u64,
			usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
			mapped_at_creation: false,
		});
		ctx.queue
			.write_buffer(&lights, 0, bytemuck::bytes_of(&LightsUniform::default()));
		self.defaults = Some(Defaults {
			environment: environment.create_view(&wgpu::TextureViewDescriptor::default()),
			shadow: shadow_view,
			lights,
		});
	}

	/// Puts group 0 together from the slots `camera`, `lights`,
	/// `environment` and `shadow_map`. `false`: there is no camera.
	#[allow(clippy::cast_precision_loss)] // mip levels and sizes are small
	pub fn prepare(&mut self, ctx: &mut PrepareContext<'_>) -> bool {
		self.group = None;
		let (Some(layout_id), Some(defaults)) = (self.layout, &self.defaults) else {
			return false;
		};
		let Some(camera) = ctx.data::<CameraData>("camera").cloned() else {
			return false;
		};
		let Some(layout) = ctx.cache.get_layout(layout_id).cloned() else {
			return false;
		};
		let lights = ctx.data::<LightsData>("lights").map(|lights| lights.buffer.clone());
		let environment = ctx.data::<EnvironmentData>("environment").cloned();
		let shadow = ctx.texture("shadow_map").cloned();
		let shadow_info = ctx.note::<ShadowInfo>("shadow_map").copied();

		let env_view = environment.as_ref().and_then(|env| env.texture.clone());
		let mut scene = SceneUniform::default();
		if let (Some(environment), Some(_)) = (&environment, &env_view) {
			scene.params[0] = environment.intensity;
			scene.params[1] = 1.0;
			scene.params[3] = environment.mip_levels.saturating_sub(1) as f32;
		}
		let shadow_view = match (&shadow, shadow_info) {
			(Some(view), Some(info)) => {
				scene.shadow_matrix = info.view_proj;
				scene.params[2] = 1.0;
				scene.shadow = [info.bias, 1.0 / info.size.max(1) as f32, 0.0, 0.0];
				view.clone()
			}
			_ => defaults.shadow.clone(),
		};
		let device = ctx.device;
		let scene_buffer = ctx
			.view_state::<ViewScene>()
			.0
			.get_or_insert_with(|| {
				device.create_buffer(&wgpu::BufferDescriptor {
					label: Some("Zenith Scene"),
					size: size_of::<SceneUniform>() as u64,
					usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
					mapped_at_creation: false,
				})
			})
			.clone();
		ctx.queue.write_buffer(&scene_buffer, 0, bytemuck::bytes_of(&scene));

		let env_sampler = ctx.cache.sampler(&SamplerKey {
			address_mode: wgpu::AddressMode::Repeat,
			..SamplerKey::LINEAR_CLAMP
		});
		let shadow_sampler = ctx.cache.sampler(&SamplerKey {
			mipmap_filter: wgpu::MipmapFilterMode::Nearest,
			compare: Some(wgpu::CompareFunction::LessEqual),
			..SamplerKey::LINEAR_CLAMP
		});
		let lights = lights.as_ref().unwrap_or(&defaults.lights);
		let env_view = env_view.as_ref().unwrap_or(&defaults.environment);
		self.group = Some(ctx.bind_group(
			&layout,
			&[
				wgpu::BindGroupEntry {
					binding: 0,
					resource: camera.buffer.as_entire_binding(),
				},
				wgpu::BindGroupEntry {
					binding: 1,
					resource: lights.as_entire_binding(),
				},
				wgpu::BindGroupEntry {
					binding: 2,
					resource: scene_buffer.as_entire_binding(),
				},
				wgpu::BindGroupEntry {
					binding: 3,
					resource: wgpu::BindingResource::TextureView(env_view),
				},
				wgpu::BindGroupEntry {
					binding: 4,
					resource: wgpu::BindingResource::Sampler(&env_sampler),
				},
				wgpu::BindGroupEntry {
					binding: 5,
					resource: wgpu::BindingResource::TextureView(&shadow_view),
				},
				wgpu::BindGroupEntry {
					binding: 6,
					resource: wgpu::BindingResource::Sampler(&shadow_sampler),
				},
			],
		));
		true
	}
}

/// What `prepare` decided to draw, for `exec`.
struct DrawItem {
	pipeline: PipelineId,
	mesh: AssetId,
	material: AssetId,
	instances: Range<u32>,
}

/// The shader of a material: `standard` is `Zenith/standard`. One that the
/// engine does not have is drawn with the standard one.
fn shader_name(material: &str) -> String {
	if material.contains('/') {
		material.to_owned()
	} else {
		format!("Zenith/{material}")
	}
}

/// A pass that draws the batches of the `meshes` slot with the standard
/// lighting: `zenith.opaque` and `zenith.transparent`.
#[derive(Default)]
pub struct MeshDraw {
	scene: SceneBindings,
	/// Blend and do not write depth.
	blended: bool,
	// The pass of one view is prepared and recorded before the next view's.
	items: Vec<DrawItem>,
}

impl MeshDraw {
	pub fn new(blended: bool) -> Self {
		Self {
			blended,
			..Self::default()
		}
	}

	/// # Errors
	/// There is no standard shader.
	pub fn init(&mut self, ctx: &mut InitContext<'_>) -> ZResult<()> {
		self.scene.init(ctx);
		if ctx.cache.named_shader(shaders::STANDARD).is_none() {
			return Err(ZenithError::Other(format!("no shader {}", shaders::STANDARD)));
		}
		Ok(())
	}

	pub fn prepare(&mut self, ctx: &mut PrepareContext<'_>) -> PassAction {
		self.items.clear();
		let (Some(meshes), Some(scene_layout)) = (ctx.data::<MeshesData>("meshes").cloned(), self.scene.layout())
		else {
			return PassAction::Skip;
		};
		if !self.scene.prepare(ctx) {
			return PassAction::Skip;
		}
		let Some(color_format) = ctx.texture_format("color") else {
			return PassAction::Skip;
		};
		let material_layout = ctx.cache.material_layout();
		for batch in &meshes.list.batches {
			let Some(material) = ctx.cache.material(batch.material) else {
				continue;
			};
			if ctx.cache.mesh(batch.mesh).is_none() {
				continue;
			}
			let (double_sided, name) = (material.double_sided, shader_name(&material.shader));
			let blend = self.blended || material.blend == BlendMode::Blend;
			let shader = ctx
				.cache
				.named_shader(&name)
				.or_else(|| ctx.cache.named_shader(shaders::STANDARD));
			let Some(shader) = shader else {
				continue;
			};
			let key = PipelineKey {
				layouts: vec![scene_layout, material_layout],
				vertex_buffers: vec![GpuMesh::LAYOUT, InstanceData::LAYOUT],
				targets: vec![Some(wgpu::ColorTargetState {
					format: color_format,
					blend: blend.then_some(wgpu::BlendState::ALPHA_BLENDING),
					write_mask: wgpu::ColorWrites::ALL,
				})],
				depth_stencil: Some(wgpu::DepthStencilState {
					format: DEPTH_FORMAT,
					depth_write_enabled: Some(!self.blended),
					depth_compare: Some(wgpu::CompareFunction::Less),
					stencil: wgpu::StencilState::default(),
					bias: wgpu::DepthBiasState::default(),
				}),
				primitive: wgpu::PrimitiveState {
					cull_mode: (!double_sided).then_some(wgpu::Face::Back),
					..wgpu::PrimitiveState::default()
				},
				..PipelineKey::new(shader, color_format)
			};
			let pipeline = ctx.cache.pipeline(&key);
			self.items.push(DrawItem {
				pipeline,
				mesh: batch.mesh,
				material: batch.material,
				instances: batch.instances.clone(),
			});
		}
		PassAction::Run
	}

	/// Draws into the slots `color` and `depth`.
	///
	/// # Errors
	/// A slot is not bound.
	pub fn exec(&self, ctx: &mut ExecContext<'_>, label: &str, clear: [f32; 4]) -> ZResult<()> {
		let cache = ctx.cache;
		let instances = ctx
			.data::<MeshesData>("meshes")
			.and_then(|meshes| meshes.instances.as_ref());
		let [r, g, b, a] = clear.map(f64::from);
		let mut pass = ctx
			.begin_render_pass(label, &[("color", wgpu::Color { r, g, b, a })], Some("depth"))
			.ok_or_else(|| ZenithError::Other(format!("{label}: the color or the depth is not bound")))?;
		let (Some(group), Some(instances)) = (&self.scene.group, instances) else {
			return Ok(());
		};
		pass.set_bind_group(0, group, &[]);
		pass.set_vertex_buffer(1, instances.slice(..));
		for item in &self.items {
			let (Some(pipeline), Some(mesh), Some(material)) = (
				cache.get_pipeline(item.pipeline),
				cache.mesh(item.mesh),
				cache.material(item.material),
			) else {
				continue;
			};
			pass.set_pipeline(pipeline);
			pass.set_bind_group(1, &material.bind_group, &[]);
			mesh.bind(&mut pass, 0);
			for submesh in &mesh.submeshes {
				pass.draw_indexed(submesh.indices.clone(), submesh.base_vertex, item.instances.clone());
			}
		}
		Ok(())
	}
}
