//! `zenith.opaque`: draws the opaque and masked meshes into an HDR texture
//! with a depth buffer.
//!
//! A first version of the shading: see `standard.wgsl`. A mesh or a material
//! that is not loaded yet is not drawn.

use std::ops::Range;

use zenith_error::{ZResult, ZenithError};

use super::inputs::{CameraData, MeshesData};
use crate::{
	assets::AssetId,
	cache::{GpuMesh, LayoutId, PipelineId, PipelineKey},
	graph::{
		ExecContext, InitContext, InputDesc, OutputDesc, ParamDesc, ParamMap, PassAction, PassCategory, PassDesc,
		PrepareContext, RenderPass, SlotKind, TextureDesc, pass::data,
	},
	prepare::InstanceData,
	shaders,
};

const DEPTH_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Depth32Float;

/// The layout of group 0 of the passes that draw meshes: the camera.
pub const fn camera_layout_entries() -> [wgpu::BindGroupLayoutEntry; 1] {
	[wgpu::BindGroupLayoutEntry {
		binding: 0,
		visibility: wgpu::ShaderStages::VERTEX_FRAGMENT,
		ty: wgpu::BindingType::Buffer {
			ty: wgpu::BufferBindingType::Uniform,
			has_dynamic_offset: false,
			min_binding_size: wgpu::BufferSize::new(size_of::<crate::prepare::CameraUniform>() as u64),
		},
		count: None,
	}]
}

/// What `prepare` decided to draw, for `exec`.
struct DrawItem {
	pipeline: PipelineId,
	mesh: AssetId,
	material: AssetId,
	instances: Range<u32>,
}

#[derive(Default)]
pub struct Opaque {
	clear: [f32; 4],
	camera_layout: Option<LayoutId>,
	// The pass of one view is prepared and recorded before the next view's.
	camera_group: Option<wgpu::BindGroup>,
	items: Vec<DrawItem>,
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

impl RenderPass for Opaque {
	fn describe() -> PassDesc {
		PassDesc::new("zenith.opaque", "Opaque", PassCategory::Geometry)
			.input(InputDesc::read("camera", SlotKind::data(data::CAMERA)))
			.input(InputDesc::read("meshes", SlotKind::data(data::MESHES)))
			.output(OutputDesc::create("color", SlotKind::Color, TextureDesc::HDR))
			.output(OutputDesc::create("depth", SlotKind::Depth, TextureDesc::DEPTH))
			.param(ParamDesc::color("clear_color", [0.1, 0.2, 0.5, 1.0]).tooltip("What the scene is cleared to"))
	}

	fn set_params(&mut self, params: &ParamMap) -> ZResult<()> {
		self.clear = params.vec4("clear_color").unwrap_or([0.1, 0.2, 0.5, 1.0]);
		Ok(())
	}

	fn init(&mut self, ctx: &mut InitContext<'_>) -> ZResult<()> {
		self.camera_layout = Some(ctx.cache.layout(&camera_layout_entries()));
		if ctx.cache.named_shader(shaders::STANDARD).is_none() {
			return Err(ZenithError::Other(format!("no shader {}", shaders::STANDARD)));
		}
		Ok(())
	}

	fn prepare(&mut self, ctx: &mut PrepareContext<'_>) -> PassAction {
		self.items.clear();
		self.camera_group = None;
		let (Some(camera), Some(meshes), Some(camera_layout)) = (
			ctx.data::<CameraData>("camera").cloned(),
			ctx.data::<MeshesData>("meshes").cloned(),
			self.camera_layout,
		) else {
			return PassAction::Skip;
		};
		let Some(layout) = ctx.cache.get_layout(camera_layout).cloned() else {
			return PassAction::Skip;
		};
		self.camera_group = Some(ctx.bind_group(
			&layout,
			&[wgpu::BindGroupEntry {
				binding: 0,
				resource: camera.buffer.as_entire_binding(),
			}],
		));
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
			let shader = ctx
				.cache
				.named_shader(&name)
				.or_else(|| ctx.cache.named_shader(shaders::STANDARD));
			let Some(shader) = shader else {
				continue;
			};
			let key = PipelineKey {
				layouts: vec![camera_layout, material_layout],
				vertex_buffers: vec![GpuMesh::LAYOUT, InstanceData::LAYOUT],
				depth_stencil: Some(wgpu::DepthStencilState {
					format: DEPTH_FORMAT,
					depth_write_enabled: Some(true),
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

	fn exec(&self, ctx: &mut ExecContext<'_>) -> ZResult<()> {
		let cache = ctx.cache;
		let instances = ctx
			.data::<MeshesData>("meshes")
			.and_then(|meshes| meshes.instances.as_ref());
		let [r, g, b, a] = self.clear.map(f64::from);
		let mut pass = ctx
			.begin_render_pass("opaque", &[("color", wgpu::Color { r, g, b, a })], Some("depth"))
			.ok_or_else(|| ZenithError::Other("opaque: the color or the depth is not bound".into()))?;
		let (Some(camera_group), Some(instances)) = (&self.camera_group, instances) else {
			return Ok(());
		};
		pass.set_bind_group(0, camera_group, &[]);
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
