//! `prepare_draw_lists`: what each view draws, sorted and batched.
//!
//! Opaque meshes are sorted so what shares a material and a mesh is one
//! batch; transparent ones from far to near, so they blend right. Every
//! instance of the frame (of every view) goes into one buffer, and a batch
//! names its range in it: a buffer written once is no trap, as it is for the
//! uniform buffers of the views.

use std::{collections::HashMap, ops::Range};

use bevy_ecs::{
	resource::Resource,
	system::{Res, ResMut},
};
use bytemuck::{Pod, Zeroable};
use glam::{DVec3, Mat3, Mat4, Vec3, Vec4};

use crate::{
	assets::{AssetId, json::BlendMode},
	cache::GpuCache,
	extract::{ExtractedMesh, ExtractedMeshes, ExtractedViews},
	graph::ViewKey,
	prepare::views::{PreparedView, PreparedViews},
	renderer::Renderer,
};

/// What differs between the instances of a mesh: where it stands, relative to
/// the camera.
#[repr(C)]
#[derive(Clone, Copy, Debug, PartialEq, Pod, Zeroable)]
pub struct InstanceData {
	/// Model space to world space, relative to the camera.
	pub model: [[f32; 4]; 4],
	/// Normals to world space: the three columns of a 3x3 matrix, each padded
	/// to a `vec4`.
	pub normal: [[f32; 4]; 3],
}

impl InstanceData {
	pub fn of(mesh: &ExtractedMesh, camera: DVec3) -> Self {
		let model =
			Mat4::from_scale_rotation_translation(mesh.scale, mesh.rotation, (mesh.position - camera).as_vec3());
		let linear = Mat3::from_mat4(model);
		// A scale of zero has no inverse: the mesh is a flat thing anyway.
		let normal = if linear.determinant().abs() > f32::EPSILON {
			linear.inverse().transpose()
		} else {
			Mat3::IDENTITY
		};
		let column = |column: Vec3| Vec4::from((column, 0.0)).to_array();
		Self {
			model: model.to_cols_array_2d(),
			normal: [column(normal.x_axis), column(normal.y_axis), column(normal.z_axis)],
		}
	}
}

/// Instances of one mesh with one material.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct MeshBatch {
	pub material: AssetId,
	pub mesh: AssetId,
	/// The range in the instance buffer.
	pub instances: Range<u32>,
}

/// The batches of one phase, in the order they are drawn.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct MeshList {
	pub batches: Vec<MeshBatch>,
}

impl MeshList {
	#[allow(clippy::cast_possible_truncation)] // a batch has far fewer than 2^32 instances
	pub fn instance_count(&self) -> u32 {
		self.batches.iter().map(|batch| batch.instances.len() as u32).sum()
	}
}

/// What one view draws.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ViewDrawLists {
	pub opaque: MeshList,
	pub transparent: MeshList,
}

/// The draw lists of the frame, by view, and the buffer of the instances.
#[derive(Resource, Default)]
pub struct DrawLists {
	views: HashMap<ViewKey, ViewDrawLists>,
	instances: Option<wgpu::Buffer>,
	capacity: u64,
}

impl DrawLists {
	pub fn get(&self, key: ViewKey) -> Option<&ViewDrawLists> {
		self.views.get(&key)
	}

	/// The buffer every batch counts its instances in (a vertex buffer).
	pub const fn instance_buffer(&self) -> Option<&wgpu::Buffer> {
		self.instances.as_ref()
	}
}

/// Sorts and batches the meshes of every prepared view.
#[allow(clippy::needless_pass_by_value)] // a system takes its parameters by value
pub fn prepare_draw_lists(
	views: Res<ExtractedViews>,
	meshes: Res<ExtractedMeshes>,
	prepared: Res<PreparedViews>,
	cache: Res<GpuCache>,
	renderer: Res<Renderer>,
	mut lists: ResMut<DrawLists>,
) {
	lists.views.clear();
	let mut instances = Vec::new();
	for view in &views.0 {
		let Some(prepared) = prepared.get(view.key()) else {
			continue;
		};
		let view_lists = build(&meshes.0, prepared, &cache, &mut instances);
		lists.views.insert(prepared.key, view_lists);
	}
	if instances.is_empty() {
		return;
	}
	let bytes: &[u8] = bytemuck::cast_slice(&instances);
	let needed = bytes.len() as u64;
	if lists.instances.is_none() || lists.capacity < needed {
		lists.capacity = needed.next_power_of_two().max(4096);
		lists.instances = Some(renderer.device().create_buffer(&wgpu::BufferDescriptor {
			label: Some("Zenith Instances"),
			size: lists.capacity,
			usage: wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::COPY_DST,
			mapped_at_creation: false,
		}));
	}
	if let Some(buffer) = &lists.instances {
		renderer.queue().write_buffer(buffer, 0, bytes);
	}
}

/// The draw lists of one view; its instances are added to `instances`.
fn build(
	meshes: &[ExtractedMesh],
	view: &PreparedView,
	cache: &GpuCache,
	instances: &mut Vec<InstanceData>,
) -> ViewDrawLists {
	let forward = (view.rotation * Vec3::NEG_Z).as_dvec3();
	let mut opaque: Vec<&ExtractedMesh> = Vec::new();
	let mut transparent: Vec<(f64, &ExtractedMesh)> = Vec::new();
	for mesh in meshes {
		// A material that is not loaded yet is drawn as the stand-in: opaque.
		let blend = cache
			.material(mesh.material)
			.map_or(BlendMode::Opaque, |material| material.blend);
		if blend == BlendMode::Blend {
			transparent.push(((mesh.position - view.position).dot(forward), mesh));
		} else {
			opaque.push(mesh);
		}
	}
	// By material and mesh, so equal ones are next to each other. The entity
	// keeps the order the same every frame.
	opaque.sort_by_key(|mesh| (mesh.material, mesh.mesh, mesh.entity));
	// Far to near.
	transparent.sort_by(|a, b| b.0.total_cmp(&a.0).then(a.1.entity.cmp(&b.1.entity)));
	let transparent: Vec<&ExtractedMesh> = transparent.into_iter().map(|(_, mesh)| mesh).collect();
	ViewDrawLists {
		opaque: batch(&opaque, view.position, instances),
		transparent: batch(&transparent, view.position, instances),
	}
}

/// Neighbours with the same material and mesh become one batch.
#[allow(clippy::cast_possible_truncation)] // a frame has far fewer than 2^32 instances
fn batch(sorted: &[&ExtractedMesh], camera: DVec3, instances: &mut Vec<InstanceData>) -> MeshList {
	let mut list = MeshList::default();
	for mesh in sorted {
		let at = instances.len() as u32;
		instances.push(InstanceData::of(mesh, camera));
		match list.batches.last_mut() {
			Some(last) if last.material == mesh.material && last.mesh == mesh.mesh => last.instances.end = at + 1,
			_ => list.batches.push(MeshBatch {
				material: mesh.material,
				mesh: mesh.mesh,
				instances: at..at + 1,
			}),
		}
	}
	list
}

#[cfg(test)]
mod tests {
	use bevy_ecs::entity::Entity;
	use glam::Quat;

	use super::*;

	fn mesh_at(position: DVec3, scale: Vec3) -> ExtractedMesh {
		ExtractedMesh {
			entity: Entity::PLACEHOLDER,
			mesh: AssetId::from_bytes([1; 16]),
			material: AssetId::from_bytes([2; 16]),
			position,
			rotation: Quat::IDENTITY,
			scale,
		}
	}

	#[test]
	fn an_instance_is_placed_relative_to_the_camera() {
		// Far from the origin, where an f32 position would lose the detail.
		let far = 1.0e7;
		let instance = InstanceData::of(
			&mesh_at(DVec3::new(far + 2.0, 0.0, 0.0), Vec3::ONE),
			DVec3::new(far, 0.0, 0.0),
		);
		assert_eq!(instance.model[3][..3], [2.0, 0.0, 0.0]);
	}

	#[test]
	fn a_scaled_instance_has_the_inverse_transpose_for_its_normals() {
		let instance = InstanceData::of(&mesh_at(DVec3::ZERO, Vec3::new(2.0, 1.0, 1.0)), DVec3::ZERO);
		assert_eq!(instance.normal[0][0], 0.5);
		assert_eq!(instance.normal[1][1], 1.0);
	}

	#[test]
	fn a_flat_scale_does_not_make_nan() {
		let instance = InstanceData::of(&mesh_at(DVec3::ZERO, Vec3::new(0.0, 1.0, 1.0)), DVec3::ZERO);
		assert!(instance.normal.iter().flatten().all(|value| value.is_finite()));
	}
}
