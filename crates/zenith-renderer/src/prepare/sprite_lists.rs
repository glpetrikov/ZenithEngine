//! `prepare_sprite_lists`: the sprites of each view, in the order they are
//! drawn.
//!
//! By `order` first, then from far to near, then by texture, so neighbours
//! that share a texture are one batch. The instances of every view go into one
//! buffer, and a batch names its range in it.

use std::{collections::HashMap, ops::Range};

use bevy_ecs::{
	resource::Resource,
	system::{Res, ResMut},
};
use bytemuck::{Pod, Zeroable};
use glam::{DVec3, Quat, Vec3};

use crate::{
	assets::AssetId,
	extract::{ExtractedSprite, ExtractedSprites, ExtractedViews},
	graph::ViewKey,
	prepare::views::PreparedViews,
	renderer::Renderer,
};

/// A sprite on the GPU (the vertex buffer of `sprite.wgsl`, per instance).
#[repr(C)]
#[derive(Clone, Copy, Debug, PartialEq, Pod, Zeroable)]
pub struct SpriteInstance {
	/// The center, relative to the camera.
	pub position: [f32; 3],
	pub size: [f32; 2],
	/// The rotation, a quaternion (xyzw).
	pub rotation: [f32; 4],
	pub color: [f32; 4],
	/// The part of the texture: x, y, width, height in 0..1.
	pub uv_rect: [f32; 4],
}

impl SpriteInstance {
	pub const LAYOUT: wgpu::VertexBufferLayout<'static> = wgpu::VertexBufferLayout {
		array_stride: size_of::<Self>() as wgpu::BufferAddress,
		step_mode: wgpu::VertexStepMode::Instance,
		attributes: &wgpu::vertex_attr_array![
			0 => Float32x3,
			1 => Float32x2,
			2 => Float32x4,
			3 => Float32x4,
			4 => Float32x4,
		],
	};

	pub fn of(sprite: &ExtractedSprite, camera: glam::DVec3) -> Self {
		Self {
			position: (sprite.position - camera).as_vec3().to_array(),
			size: sprite.size.to_array(),
			rotation: sprite.rotation.to_array(),
			color: sprite.color,
			uv_rect: sprite.uv_rect.unwrap_or([0.0, 0.0, 1.0, 1.0]),
		}
	}
}

/// Instances that share a texture.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SpriteBatch {
	pub texture: AssetId,
	/// The range in the instance buffer.
	pub instances: Range<u32>,
}

/// The batches of a view, in the order they are drawn.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct SpriteList {
	pub batches: Vec<SpriteBatch>,
}

/// The sprite lists of the frame, by view, and the buffer of the instances.
#[derive(Resource, Default)]
pub struct SpriteLists {
	views: HashMap<ViewKey, SpriteList>,
	instances: Option<wgpu::Buffer>,
	capacity: u64,
}

impl SpriteLists {
	pub fn get(&self, key: ViewKey) -> Option<&SpriteList> {
		self.views.get(&key)
	}

	/// The buffer every batch counts its instances in (a vertex buffer).
	pub const fn instance_buffer(&self) -> Option<&wgpu::Buffer> {
		self.instances.as_ref()
	}
}

/// Sorts and batches the sprites of every prepared view.
#[allow(clippy::needless_pass_by_value)] // a system takes its parameters by value
pub fn prepare_sprite_lists(
	views: Res<ExtractedViews>,
	sprites: Res<ExtractedSprites>,
	prepared: Res<PreparedViews>,
	renderer: Res<Renderer>,
	mut lists: ResMut<SpriteLists>,
) {
	lists.views.clear();
	let mut instances = Vec::new();
	for view in &views.0 {
		let Some(prepared) = prepared.get(view.key()) else {
			continue;
		};
		let list = build(&sprites.0, prepared.position, prepared.rotation, &mut instances);
		lists.views.insert(prepared.key, list);
	}
	if instances.is_empty() {
		return;
	}
	let bytes: &[u8] = bytemuck::cast_slice(&instances);
	let needed = bytes.len() as u64;
	if lists.instances.is_none() || lists.capacity < needed {
		lists.capacity = needed.next_power_of_two().max(4096);
		lists.instances = Some(renderer.device().create_buffer(&wgpu::BufferDescriptor {
			label: Some("Zenith Sprites"),
			size: lists.capacity,
			usage: wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::COPY_DST,
			mapped_at_creation: false,
		}));
	}
	if let Some(buffer) = &lists.instances {
		renderer.queue().write_buffer(buffer, 0, bytes);
	}
}

/// The list of one view; its instances are added to `instances`.
#[allow(clippy::cast_possible_truncation)] // a frame has far fewer than 2^32 sprites
fn build(
	sprites: &[ExtractedSprite],
	position: DVec3,
	rotation: Quat,
	instances: &mut Vec<SpriteInstance>,
) -> SpriteList {
	let forward = (rotation * Vec3::NEG_Z).as_dvec3();
	let mut sorted: Vec<(f64, &ExtractedSprite)> = sprites
		.iter()
		.map(|sprite| ((sprite.position - position).dot(forward), sprite))
		.collect();
	// The lowest order first; inside an order, far to near.
	sorted.sort_by(|a, b| {
		a.1.order
			.cmp(&b.1.order)
			.then(b.0.total_cmp(&a.0))
			.then(a.1.texture.cmp(&b.1.texture))
			.then(a.1.entity.cmp(&b.1.entity))
	});
	let mut list = SpriteList::default();
	for (_, sprite) in sorted {
		let at = instances.len() as u32;
		instances.push(SpriteInstance::of(sprite, position));
		match list.batches.last_mut() {
			Some(last) if last.texture == sprite.texture => last.instances.end = at + 1,
			_ => list.batches.push(SpriteBatch {
				texture: sprite.texture,
				instances: at..at + 1,
			}),
		}
	}
	list
}

#[cfg(test)]
mod tests {
	use bevy_ecs::entity::Entity;
	use glam::Vec2;

	use super::*;

	fn sprite(texture: u8, order: i32, z: f64) -> ExtractedSprite {
		ExtractedSprite {
			entity: Entity::PLACEHOLDER,
			texture: AssetId::from_bytes([texture; 16]),
			position: DVec3::new(0.0, 0.0, z),
			rotation: Quat::IDENTITY,
			size: Vec2::ONE,
			color: [1.0; 4],
			uv_rect: None,
			order,
		}
	}

	fn list(sprites: &[ExtractedSprite]) -> (SpriteList, Vec<SpriteInstance>) {
		let mut instances = Vec::new();
		// A camera at z = 10, looking down -Z.
		let list = build(sprites, DVec3::new(0.0, 0.0, 10.0), Quat::IDENTITY, &mut instances);
		(list, instances)
	}

	#[test]
	fn an_instance_is_relative_to_the_camera_and_has_a_whole_texture_by_default() {
		let instance = SpriteInstance::of(&sprite(1, 0, 5.0), DVec3::new(0.0, 0.0, 3.0));
		assert_eq!(instance.position, [0.0, 0.0, 2.0]);
		assert_eq!(instance.uv_rect, [0.0, 0.0, 1.0, 1.0]);
	}

	#[test]
	fn the_order_goes_first_then_far_to_near() {
		let (_, instances) = list(&[sprite(1, 1, 0.0), sprite(1, 0, 5.0), sprite(1, 0, -5.0)]);
		// Relative to the camera: z - 10.
		let drawn: Vec<f32> = instances.iter().map(|instance| instance.position[2]).collect();
		assert_eq!(drawn, [-15.0, -5.0, -10.0]);
	}

	#[test]
	fn neighbours_with_one_texture_are_one_batch() {
		let (list, _) = list(&[
			sprite(1, 0, 0.0),
			sprite(2, 0, 1.0),
			sprite(2, 0, 2.0),
			sprite(1, 0, 3.0),
		]);
		let batches: Vec<(u8, usize)> = list
			.batches
			.iter()
			.map(|batch| (batch.texture.as_bytes()[0], batch.instances.len()))
			.collect();
		assert_eq!(batches, [(1, 1), (2, 2), (1, 1)]);
	}
}
