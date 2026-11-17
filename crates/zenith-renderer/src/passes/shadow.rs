//! `zenith.shadow`: the shadow map of the view, from the first directional
//! light that casts shadows.
//!
//! The map covers a box of `distance` around the camera, seen from the light.
//! Everything is relative to the camera, so the box is around the origin. The
//! matrices go to the passes that read the map as a note on the texture
//! ([`ShadowInfo`]).

use glam::{Mat4, Vec3};
use zenith_error::{ZResult, ZenithError};

use super::inputs::{CameraData, LightsData, MeshesData};
use crate::{
	cache::{GpuMesh, LayoutId, PipelineId, PipelineKey},
	graph::{
		ExecContext, InitContext, InputDesc, OutputDesc, ParamDesc, ParamMap, PassAction, PassCategory, PassDesc,
		PrepareContext, RenderPass, SizeRule, SlotKind, TextureDesc,
		pass::{FormatRule, data},
	},
	prepare::InstanceData,
	shaders,
};

const DEPTH_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Depth32Float;

/// What the passes that read the shadow map need to read it.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ShadowInfo {
	/// Camera-relative world space to the clip space of the light.
	pub view_proj: [[f32; 4]; 4],
	/// How far the surface is moved along its normal, in world units.
	pub bias: f32,
	/// The side of the map in texels.
	pub size: u32,
}

/// The matrix of a light that travels along `direction`, for a box of
/// `distance` around the origin.
pub fn light_matrix(direction: Vec3, distance: f32) -> Mat4 {
	let direction = direction.normalize_or_zero();
	let direction = if direction == Vec3::ZERO {
		Vec3::NEG_Y
	} else {
		direction
	};
	let up = if direction.y.abs() > 0.99 { Vec3::Z } else { Vec3::Y };
	let view = glam::camera::rh::view::look_at_mat4(-direction * distance, Vec3::ZERO, up);
	let proj =
		glam::camera::rh::proj::directx::orthographic(-distance, distance, -distance, distance, 0.0, distance * 2.0);
	proj * view
}

struct Item {
	mesh: crate::assets::AssetId,
	instances: std::ops::Range<u32>,
}

/// What a view keeps between frames: the buffer of its light matrix.
#[derive(Default)]
struct ViewLight(Option<wgpu::Buffer>);

pub struct Shadow {
	size: u32,
	distance: f32,
	bias: f32,
	layout: Option<LayoutId>,
	// The pass of one view is prepared and recorded before the next view's.
	pipeline: Option<PipelineId>,
	group: Option<wgpu::BindGroup>,
	items: Vec<Item>,
}

impl Default for Shadow {
	fn default() -> Self {
		Self {
			size: 2048,
			distance: 40.0,
			bias: 0.03,
			layout: None,
			pipeline: None,
			group: None,
			items: Vec::new(),
		}
	}
}

const fn layout_entries() -> [wgpu::BindGroupLayoutEntry; 1] {
	[wgpu::BindGroupLayoutEntry {
		binding: 0,
		visibility: wgpu::ShaderStages::VERTEX,
		ty: wgpu::BindingType::Buffer {
			ty: wgpu::BufferBindingType::Uniform,
			has_dynamic_offset: false,
			min_binding_size: wgpu::BufferSize::new(size_of::<[[f32; 4]; 4]>() as u64),
		},
		count: None,
	}]
}

impl RenderPass for Shadow {
	fn describe() -> PassDesc {
		PassDesc::new("zenith.shadow", "Shadow", PassCategory::Lighting)
			.input(InputDesc::read("camera", SlotKind::data(data::CAMERA)))
			.input(InputDesc::read("lights", SlotKind::data(data::LIGHTS)))
			.input(InputDesc::read("meshes", SlotKind::data(data::MESHES)))
			.output(OutputDesc::create(
				"shadow_map",
				SlotKind::Depth,
				TextureDesc {
					format: FormatRule::Depth,
					size: SizeRule::FromParam("size".into()),
				},
			))
			.param(
				ParamDesc::int("size", 2048)
					.range(256.0, 8192.0)
					.tooltip("The side of the map in texels"),
			)
			.param(
				ParamDesc::float("distance", 40.0)
					.range(1.0, 2000.0)
					.tooltip("Half the side of the box around the camera that is shadowed"),
			)
			.param(
				ParamDesc::float("bias", 0.03)
					.range(0.0, 1.0)
					.tooltip("How far a surface is moved along its normal before it is looked up"),
			)
	}

	fn set_params(&mut self, params: &ParamMap) -> ZResult<()> {
		self.size = params
			.int("size")
			.and_then(|size| u32::try_from(size).ok())
			.unwrap_or(2048)
			.clamp(256, 8192);
		self.distance = params.float("distance").unwrap_or(40.0).max(1.0);
		self.bias = params.float("bias").unwrap_or(0.03).max(0.0);
		Ok(())
	}

	fn init(&mut self, ctx: &mut InitContext<'_>) -> ZResult<()> {
		self.layout = Some(ctx.cache.layout(&layout_entries()));
		if ctx.cache.named_shader(shaders::SHADOW).is_none() {
			return Err(ZenithError::Other(format!("no shader {}", shaders::SHADOW)));
		}
		Ok(())
	}

	fn prepare(&mut self, ctx: &mut PrepareContext<'_>) -> PassAction {
		self.pipeline = None;
		self.group = None;
		self.items.clear();
		let shader = ctx.cache.named_shader(shaders::SHADOW);
		let (Some(_), Some(lights), Some(meshes), Some(layout_id), Some(shader)) = (
			ctx.data::<CameraData>("camera"),
			ctx.data::<LightsData>("lights").cloned(),
			ctx.data::<MeshesData>("meshes").cloned(),
			self.layout,
			shader,
		) else {
			return PassAction::Skip;
		};
		// No light that casts shadows: no map, and the passes that read it
		// light without shadows.
		let Some(direction) = lights.uniform.shadow_direction() else {
			return PassAction::Skip;
		};
		let Some(layout) = ctx.cache.get_layout(layout_id).cloned() else {
			return PassAction::Skip;
		};
		let matrix = light_matrix(direction, self.distance);
		let device = ctx.device;
		let buffer = ctx
			.view_state::<ViewLight>()
			.0
			.get_or_insert_with(|| {
				device.create_buffer(&wgpu::BufferDescriptor {
					label: Some("Zenith Shadow"),
					size: size_of::<[[f32; 4]; 4]>() as u64,
					usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
					mapped_at_creation: false,
				})
			})
			.clone();
		ctx.queue
			.write_buffer(&buffer, 0, bytemuck::bytes_of(&matrix.to_cols_array_2d()));
		self.group = Some(ctx.bind_group(
			&layout,
			&[wgpu::BindGroupEntry {
				binding: 0,
				resource: buffer.as_entire_binding(),
			}],
		));
		let key = PipelineKey {
			layouts: vec![layout_id],
			vertex_buffers: vec![GpuMesh::LAYOUT, InstanceData::LAYOUT],
			fragment_entry: None,
			targets: Vec::new(),
			depth_stencil: Some(wgpu::DepthStencilState {
				format: DEPTH_FORMAT,
				depth_write_enabled: Some(true),
				depth_compare: Some(wgpu::CompareFunction::Less),
				stencil: wgpu::StencilState::default(),
				bias: wgpu::DepthBiasState {
					constant: 2,
					slope_scale: 2.0,
					clamp: 0.0,
				},
			}),
			..PipelineKey::new(shader, DEPTH_FORMAT)
		};
		self.pipeline = Some(ctx.cache.pipeline(&key));
		for batch in &meshes.list.batches {
			if ctx.cache.mesh(batch.mesh).is_some() {
				self.items.push(Item {
					mesh: batch.mesh,
					instances: batch.instances.clone(),
				});
			}
		}
		ctx.set_note(
			"shadow_map",
			ShadowInfo {
				view_proj: matrix.to_cols_array_2d(),
				bias: self.bias,
				size: self.size,
			},
		);
		PassAction::Run
	}

	fn exec(&self, ctx: &mut ExecContext<'_>) -> ZResult<()> {
		let cache = ctx.cache;
		let instances = ctx
			.data::<MeshesData>("meshes")
			.and_then(|meshes| meshes.instances.as_ref());
		let mut pass = ctx
			.begin_render_pass("shadow", &[], Some("shadow_map"))
			.ok_or_else(|| ZenithError::Other("shadow: the map is not bound".into()))?;
		let (Some(pipeline), Some(group), Some(instances)) = (
			self.pipeline.and_then(|id| cache.get_pipeline(id)),
			&self.group,
			instances,
		) else {
			return Ok(());
		};
		pass.set_pipeline(pipeline);
		pass.set_bind_group(0, group, &[]);
		pass.set_vertex_buffer(1, instances.slice(..));
		for item in &self.items {
			let Some(mesh) = cache.mesh(item.mesh) else {
				continue;
			};
			mesh.bind(&mut pass, 0);
			for submesh in &mesh.submeshes {
				pass.draw_indexed(submesh.indices.clone(), submesh.base_vertex, item.instances.clone());
			}
		}
		Ok(())
	}
}

#[cfg(test)]
mod tests {
	use super::*;

	#[test]
	fn a_point_under_the_light_is_inside_the_map_and_one_far_away_is_not() {
		let matrix = light_matrix(Vec3::NEG_Y, 10.0);
		let near = matrix.project_point3(Vec3::new(1.0, 0.0, 2.0));
		assert!(
			near.x.abs() < 1.0 && near.y.abs() < 1.0 && (0.0..1.0).contains(&near.z),
			"{near}"
		);
		let far = matrix.project_point3(Vec3::new(50.0, 0.0, 0.0));
		assert!(far.x.abs() > 1.0, "{far}");
	}

	#[test]
	fn a_point_nearer_to_the_light_has_less_depth() {
		let matrix = light_matrix(Vec3::new(0.3, -1.0, 0.2), 10.0);
		let high = matrix.project_point3(Vec3::new(0.0, 3.0, 0.0)).z;
		let low = matrix.project_point3(Vec3::new(0.0, -3.0, 0.0)).z;
		assert!(high < low, "{high} {low}");
	}

	#[test]
	fn a_light_straight_down_does_not_make_a_broken_frame() {
		let matrix = light_matrix(Vec3::NEG_Y, 5.0);
		assert!(matrix.to_cols_array().iter().all(|value| value.is_finite()));
	}
}
