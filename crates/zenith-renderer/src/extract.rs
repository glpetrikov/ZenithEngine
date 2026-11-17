//! The `Extracted*` resources and the `extract_*` systems that fill them from
//! the components.
//!
//! The rule: what is extracted holds no wgpu types and no references into the
//! world, so it can be handed to a render thread as it is. Every system clears
//! its `Vec` (the capacity stays) and copies from its query, leaving out the
//! entities that are not active.

use bevy_ecs::{
	entity::Entity,
	resource::Resource,
	system::{Query, Res, ResMut},
};
use glam::{DVec3, Quat, Vec2, Vec3};
use zenith_components::{IsActive, Transform};

use crate::{
	assets::AssetId,
	components::{
		Camera, CameraTarget, ClearMode, DirectionalLight, Environment, Model, PointLight, Projection, Sprite,
	},
	graph::{GraphId, ViewKey},
	targets::RenderTargetId,
};

/// Where a view comes from.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum ViewSource {
	/// The entity of a camera.
	Entity(Entity),
	/// A view the host added without an entity (the camera of the editor), by
	/// a number of its own.
	Host(u32),
}

/// What a view draws into.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum ViewTarget {
	/// The main output.
	Window,
	Target(RenderTargetId),
	/// The render texture `id` of `size` pixels (see `CameraTarget::Texture`).
	Texture {
		id: AssetId,
		size: (u32, u32),
	},
}

/// A camera of the frame.
#[derive(Clone, Debug, PartialEq)]
pub struct ExtractedView {
	pub source: ViewSource,
	pub position: DVec3,
	pub rotation: Quat,
	pub projection: Projection,
	pub target: ViewTarget,
	pub viewport: Option<[f32; 4]>,
	pub order: i32,
	pub clear: ClearMode,
	pub graph: Option<GraphId>,
	/// The sky and its intensity.
	pub environment: Option<(AssetId, f32)>,
}

impl ExtractedView {
	/// The same for the same camera every frame: a pass keeps its state per
	/// view by it.
	pub fn key(&self) -> ViewKey {
		match self.source {
			ViewSource::Entity(entity) => ViewKey(entity.to_bits()),
			ViewSource::Host(number) => ViewKey((1 << 63) | u64::from(number)),
		}
	}
}

/// A model of the frame.
#[derive(Clone, Debug, PartialEq)]
pub struct ExtractedMesh {
	pub entity: Entity,
	pub mesh: AssetId,
	pub material: AssetId,
	pub position: DVec3,
	pub rotation: Quat,
	pub scale: Vec3,
}

/// A sprite of the frame.
#[derive(Clone, Debug, PartialEq)]
pub struct ExtractedSprite {
	pub entity: Entity,
	pub texture: AssetId,
	pub position: DVec3,
	pub rotation: Quat,
	pub size: Vec2,
	pub color: [f32; 4],
	pub uv_rect: Option<[f32; 4]>,
	pub order: i32,
}

/// A directional light of the frame.
#[derive(Clone, Debug, PartialEq)]
pub struct ExtractedDirectionalLight {
	pub entity: Entity,
	/// The way the light travels.
	pub direction: Vec3,
	pub color: [f32; 3],
	pub illuminance: f32,
	pub shadows: bool,
}

/// A point light of the frame.
#[derive(Clone, Debug, PartialEq)]
pub struct ExtractedPointLight {
	pub entity: Entity,
	pub position: DVec3,
	pub color: [f32; 3],
	pub intensity: f32,
	pub range: f32,
}

/// The cameras of the frame, in the order they draw: by `order`, then by the
/// entity, so it is the same every frame. The views of [`ExtraViews`] come
/// among them.
#[derive(Resource, Default, Debug)]
pub struct ExtractedViews(pub Vec<ExtractedView>);

#[derive(Resource, Default, Debug)]
pub struct ExtractedMeshes(pub Vec<ExtractedMesh>);

#[derive(Resource, Default, Debug)]
pub struct ExtractedSprites(pub Vec<ExtractedSprite>);

#[derive(Resource, Default, Debug)]
pub struct ExtractedLights {
	pub directional: Vec<ExtractedDirectionalLight>,
	pub point: Vec<ExtractedPointLight>,
}

/// Views the host adds without an entity (the camera of the editor). They are
/// drawn every frame until the host takes them out.
#[derive(Resource, Default, Debug)]
pub struct ExtraViews(pub Vec<ExtractedView>);

/// An entity without `IsActive` is active.
fn active(is_active: Option<&IsActive>) -> bool {
	is_active.is_none_or(IsActive::is_active)
}

/// The cameras, and the views of the host, ordered.
#[allow(clippy::needless_pass_by_value)] // a system takes its parameters by value
pub fn extract_cameras(
	cameras: Query<(Entity, &Camera, &Transform, Option<&IsActive>, Option<&Environment>)>,
	extra: Res<ExtraViews>,
	mut views: ResMut<ExtractedViews>,
) {
	views.0.clear();
	for (entity, camera, transform, is_active, environment) in &cameras {
		if !active(is_active) {
			continue;
		}
		let target = match camera.target {
			CameraTarget::Window => ViewTarget::Window,
			CameraTarget::Texture { id, size } => ViewTarget::Texture { id, size: size.into() },
		};
		views.0.push(ExtractedView {
			source: ViewSource::Entity(entity),
			position: transform.position,
			rotation: transform.orientation,
			projection: camera.projection,
			target,
			viewport: camera.viewport,
			order: camera.order,
			clear: camera.clear,
			graph: camera.graph,
			environment: environment.map(|environment| (environment.sky, environment.intensity)),
		});
	}
	views.0.extend(extra.0.iter().cloned());
	views.0.sort_by_key(|view| (view.order, view.key()));
}

pub fn extract_meshes(
	models: Query<(Entity, &Model, &Transform, Option<&IsActive>)>,
	mut meshes: ResMut<ExtractedMeshes>,
) {
	meshes.0.clear();
	for (entity, model, transform, is_active) in &models {
		if active(is_active) {
			meshes.0.push(ExtractedMesh {
				entity,
				mesh: model.mesh,
				material: model.material,
				position: transform.position,
				rotation: transform.orientation,
				scale: transform.scale.as_vec3(),
			});
		}
	}
}

pub fn extract_sprites(
	sprites: Query<(Entity, &Sprite, &Transform, Option<&IsActive>)>,
	mut extracted: ResMut<ExtractedSprites>,
) {
	extracted.0.clear();
	for (entity, sprite, transform, is_active) in &sprites {
		if active(is_active) {
			extracted.0.push(ExtractedSprite {
				entity,
				texture: sprite.texture,
				position: transform.position,
				rotation: transform.orientation,
				size: Vec2::from(sprite.size),
				color: sprite.color,
				uv_rect: sprite.uv_rect,
				order: sprite.order,
			});
		}
	}
}

pub fn extract_lights(
	directional: Query<(Entity, &DirectionalLight, &Transform, Option<&IsActive>)>,
	point: Query<(Entity, &PointLight, &Transform, Option<&IsActive>)>,
	mut lights: ResMut<ExtractedLights>,
) {
	lights.directional.clear();
	lights.point.clear();
	for (entity, light, transform, is_active) in &directional {
		if active(is_active) {
			lights.directional.push(ExtractedDirectionalLight {
				entity,
				direction: transform.orientation * Vec3::NEG_Z,
				color: light.color,
				illuminance: light.illuminance,
				shadows: light.shadows,
			});
		}
	}
	for (entity, light, transform, is_active) in &point {
		if active(is_active) {
			lights.point.push(ExtractedPointLight {
				entity,
				position: transform.position,
				color: light.color,
				intensity: light.intensity,
				range: light.range,
			});
		}
	}
}
