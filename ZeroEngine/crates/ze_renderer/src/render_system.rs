use ze_assets::{AssetRef, ResourceManager};
use ze_core::{Mat4, Result, Vec2, Vec3};
use ze_ecs::{
	Collider, ColliderShape, EditorOnly, EntitiesView, EntityId, Inactive, Name, PhysicsSettings, RigidBody,
	RigidBodyType, Scene, System, Tag, Transform,
	shipyard::{IntoIter, View},
};
use ze_input::{Input, ZKeyCode};

use crate::{
	Renderer,
	components::{Camera, CameraProjection, Sprite, SpriteColorSettings, SpriteSize},
};

#[derive(Debug, Clone)]
pub struct CameraRenderData {
	pub view_projection: Mat4,
	pub clear_color: [f32; 4],
}

#[derive(Debug, Clone)]
pub struct SpriteRenderItem {
	pub entity: ze_ecs::ze_entity_id::ZeEntityId,
	pub label: String,
	pub transform: Mat4,
	pub texture: AssetRef,
	pub size: SpriteSize,
	pub color: SpriteColorSettings,
	pub layer: i32,
	pub texture_rotation_degrees: f32,
}

#[derive(Debug, Clone)]
pub struct DebugLine {
	pub start: Vec3,
	pub end: Vec3,
	pub color: [f32; 4],
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RenderStatus {
	Rendered,
	MissingPrimaryCamera { scene_name: String },
}

#[derive(Default)]
pub struct RenderSystem {
	items: Vec<SpriteRenderItem>,
	debug_lines: Vec<DebugLine>,
	active_scene_name: Option<String>,
	missing_primary_camera_reported_scene: Option<String>,
}

impl RenderSystem {
	pub fn new() -> Self { Self::default() }

	pub fn render(&mut self, scene: &Scene, renderer: &mut Renderer, resources: &ResourceManager) -> RenderStatus {
		self.render_scene(scene, renderer, resources)
	}

	fn render_scene(&mut self, scene: &Scene, renderer: &mut Renderer, resources: &ResourceManager) -> RenderStatus {
		self.reset_missing_primary_camera_report_on_scene_change(&scene.name);
		let Some(camera) = Self::find_primary_camera(scene, renderer.aspect_ratio()) else {
			self.items.clear();
			self.debug_lines.clear();
			if self.missing_primary_camera_reported_scene.as_deref() != Some(&scene.name) {
				self.missing_primary_camera_reported_scene = Some(scene.name.clone());
				let name = if scene.name.is_empty() { "unnamed" } else { &scene.name };
				ze_log::error!("No primary camera found in scene `{name}`");
			}
			return RenderStatus::MissingPrimaryCamera {
				scene_name: scene.name.clone(),
			};
		};

		self.items = Self::collect_items(scene);
		self.debug_lines = Self::collect_debug_lines(scene);
		renderer.request_sprite_redraw(&self.items, &self.debug_lines, &camera, resources);
		RenderStatus::Rendered
	}

	fn reset_missing_primary_camera_report_on_scene_change(&mut self, scene_name: &str) {
		if self.active_scene_name.as_deref() == Some(scene_name) {
			return;
		}

		self.active_scene_name = Some(scene_name.to_string());
		self.missing_primary_camera_reported_scene = None;
	}

	fn find_primary_camera(scene: &Scene, aspect: f32) -> Option<CameraRenderData> {
		let entity = Self::primary_camera_entity(scene)?;
		let world = scene.world();
		let camera = world.get::<&Camera>(entity).ok()?;
		let transform = scene.world_transform(entity)?;
		Some(Self::build_camera_data(&transform, &camera, aspect))
	}

	fn primary_camera_entity(scene: &Scene) -> Option<EntityId> {
		let world = scene.world();
		let Ok(cameras) = world.borrow::<View<Camera>>() else {
			return None;
		};
		let mut game_camera = None;
		let mut editor_camera = None;

		for (entity, camera) in cameras.iter().with_id() {
			if !camera.primary || world.get::<&Inactive>(entity).is_ok() {
				continue;
			}

			if world.get::<&EditorOnly>(entity).is_ok() {
				if editor_camera.is_none() {
					editor_camera = Some(entity);
				}
				continue;
			}

			game_camera = Some(entity);
			break;
		}

		game_camera.or(editor_camera)
	}

	#[cfg(test)]
	fn primary_camera_exists(scene: &Scene) -> bool { Self::primary_camera_entity(scene).is_some() }

	fn build_camera_data(transform: &Transform, camera: &Camera, aspect: f32) -> CameraRenderData {
		let camera_transform = Mat4::from_scale_rotation_translation(Vec3::ONE, transform.rotation, transform.position);
		let view = camera_transform.inverse();

		let projection = match camera.projection {
			CameraProjection::Orthographic { size, near, far } => {
				let half_height = size * 0.5;
				let half_width = half_height * aspect;
				Mat4::orthographic_rh(-half_width, half_width, -half_height, half_height, near, far)
			}
			CameraProjection::Perspective {
				fov_y_radians,
				near,
				far,
			} => Mat4::perspective_rh(fov_y_radians, aspect, near, far),
		};

		CameraRenderData {
			view_projection: projection * view,
			clear_color: camera.clear_color,
		}
	}

	fn collect_items(scene: &Scene) -> Vec<SpriteRenderItem> {
		let mut items = Vec::new();
		let world = scene.world();

		world.run(|entities: EntitiesView| {
			for entity in entities.iter() {
				if world.get::<&Inactive>(entity).is_ok() {
					continue;
				}

				let Ok(sprite) = world.get::<&Sprite>(entity) else {
					continue;
				};

				if !sprite.settings.visible {
					continue;
				}

				let Some(transform) = scene.world_transform(entity) else {
					continue;
				};

				let mut scale = transform.scale;
				if sprite.settings.flip_x {
					scale.x = -scale.x;
				}
				if sprite.settings.flip_y {
					scale.y = -scale.y;
				}

				items.push(SpriteRenderItem {
					entity: ze_ecs::ze_entity_id::ZeEntityId::from(entity),
					label: sprite_debug_label(scene, entity),
					transform: Mat4::from_scale_rotation_translation(scale, transform.rotation, transform.position),
					texture: sprite.texture.clone(),
					size: sprite.size.clone(),
					color: sprite.color.clone(),
					layer: sprite.settings.layer,
					texture_rotation_degrees: sprite.settings.texture_rotation_degrees,
				});
			}
		});

		items.sort_by_key(|item| item.layer);
		items
	}

	fn collect_debug_lines(scene: &Scene) -> Vec<DebugLine> {
		if !Self::debug_draw_enabled(scene) {
			return Vec::new();
		}

		let mut lines = Vec::new();
		let world = scene.world();

		world.run(|entities: EntitiesView| {
			for entity in entities.iter() {
				if world.get::<&Inactive>(entity).is_ok() {
					continue;
				}

				let Ok(collider) = world.get::<&Collider>(entity) else {
					continue;
				};

				let Some(transform) = scene.world_transform(entity) else {
					continue;
				};

				let rigid_body = world.get::<&RigidBody>(entity).ok();
				let color = debug_color(&collider, rigid_body.as_deref().copied());
				append_collider_lines(&mut lines, &transform, &collider, color);
			}
		});

		lines
	}

	fn debug_draw_enabled(scene: &Scene) -> bool {
		let world = scene.world();
		let mut enabled = false;

		world.run(|entities: EntitiesView| {
			for entity in entities.iter() {
				let Ok(settings) = world.get::<&PhysicsSettings>(entity) else {
					continue;
				};

				enabled = settings.enable_debug_draw;
				break;
			}
		});

		enabled
	}

	fn toggle_debug_draw(scene: &mut Scene) -> Result<()> {
		if let Some(entity) = settings_entity(scene) {
			let mut settings = scene.world_mut().get::<&mut PhysicsSettings>(entity)?;
			settings.enable_debug_draw = !settings.enable_debug_draw;
			return Ok(());
		}

		let entity = scene.create_entity("PhysicsSettings");
		scene.entity_mut(entity).add_component(PhysicsSettings {
			enable_debug_draw: true,
			..PhysicsSettings::default()
		});
		Ok(())
	}
}

impl System for RenderSystem {
	fn name(&self) -> &'static str { "RenderSystem" }

	fn update(&mut self, scene: &mut Scene, _dt: f32) -> Result<()> {
		if Input::is_key_just_pressed(ZKeyCode::KF1) {
			Self::toggle_debug_draw(scene)?;
		}

		Ok(())
	}

	fn as_any_mut(&mut self) -> &mut dyn std::any::Any { self }
}

fn sprite_debug_label(scene: &Scene, entity: EntityId) -> String {
	let world = scene.world();
	let id = ze_ecs::ze_entity_id::ZeEntityId::from(entity);
	let name = world
		.get::<&Name>(entity)
		.map_or_else(|_| "<unnamed>".to_string(), |name| name.name.clone());
	let tag = world
		.get::<&Tag>(entity)
		.map_or_else(|_| "<untagged>".to_string(), |tag| tag.tag.clone());
	format!("entity=({}, {}) name=`{name}` tag=`{tag}`", id.index, id.generation)
}

fn settings_entity(scene: &Scene) -> Option<EntityId> {
	let world = scene.world();
	let mut matching_entity = None;

	world.run(|entities: EntitiesView| {
		for entity in entities.iter() {
			if world.get::<&PhysicsSettings>(entity).is_ok() {
				matching_entity = Some(entity);
				break;
			}
		}
	});

	matching_entity
}

fn debug_color(collider: &Collider, rigid_body: Option<&RigidBody>) -> [f32; 4] {
	if collider.is_sensor {
		return [1.0, 0.0, 0.0, 1.0];
	}

	match rigid_body.map(|body| body.body_type) {
		Some(RigidBodyType::Static) => [0.1, 0.35, 1.0, 1.0],
		_ => [0.0, 1.0, 0.0, 1.0],
	}
}

fn append_collider_lines(lines: &mut Vec<DebugLine>, transform: &Transform, collider: &Collider, color: [f32; 4]) {
	match &collider.shape {
		ColliderShape::Box { half_extents } => {
			let points = [
				Vec2::new(-half_extents.x, -half_extents.y),
				Vec2::new(half_extents.x, -half_extents.y),
				Vec2::new(half_extents.x, half_extents.y),
				Vec2::new(-half_extents.x, half_extents.y),
			];
			append_closed_polyline(lines, transform, &points, color);
		}
		ColliderShape::Circle { radius } => {
			let radius = radius * transform.scale.truncate().abs().max_element();
			let points = circle_points(radius, 32);
			append_world_space_closed_polyline(lines, transform, &points, color);
		}
		ColliderShape::Capsule { half_height, radius } => {
			let scale = transform.scale.truncate().abs();
			let half_height = half_height * scale.y;
			let radius = radius * scale.max_element();
			let points = capsule_points(half_height, radius, 12);
			append_world_space_closed_polyline(lines, transform, &points, color);
		}
		ColliderShape::ConvexPolygon { points } => {
			append_closed_polyline(lines, transform, points, color);
		}
	}
}

fn append_closed_polyline(lines: &mut Vec<DebugLine>, transform: &Transform, points: &[Vec2], color: [f32; 4]) {
	if points.len() < 2 {
		return;
	}

	for i in 0..points.len() {
		lines.push(DebugLine {
			start: transform_local_point(transform, points[i]),
			end: transform_local_point(transform, points[(i + 1) % points.len()]),
			color,
		});
	}
}

fn append_world_space_closed_polyline(
	lines: &mut Vec<DebugLine>,
	transform: &Transform,
	points: &[Vec2],
	color: [f32; 4],
) {
	if points.len() < 2 {
		return;
	}

	for i in 0..points.len() {
		lines.push(DebugLine {
			start: transform_unscaled_point(transform, points[i]),
			end: transform_unscaled_point(transform, points[(i + 1) % points.len()]),
			color,
		});
	}
}

fn transform_local_point(transform: &Transform, point: Vec2) -> Vec3 {
	let scaled = Vec3::new(point.x * transform.scale.x, point.y * transform.scale.y, 0.0);
	transform.position + transform.rotation * scaled
}

fn transform_unscaled_point(transform: &Transform, point: Vec2) -> Vec3 {
	transform.position + transform.rotation * Vec3::new(point.x, point.y, 0.0)
}

fn circle_points(radius: f32, segments: usize) -> Vec<Vec2> {
	(0..segments)
		.map(|i| {
			let angle = i as f32 / segments as f32 * std::f32::consts::TAU;
			Vec2::new(angle.cos() * radius, angle.sin() * radius)
		})
		.collect()
}

fn capsule_points(half_height: f32, radius: f32, arc_segments: usize) -> Vec<Vec2> {
	let mut points = Vec::with_capacity((arc_segments + 1) * 2);

	for i in 0..=arc_segments {
		let angle = i as f32 / arc_segments as f32 * std::f32::consts::PI;
		points.push(Vec2::new(
			angle.cos() * radius,
			angle.sin().mul_add(radius, half_height),
		));
	}

	for i in 0..=arc_segments {
		let angle =
			std::f32::consts::PI + (i as f32 / arc_segments as f32).mul_add(std::f32::consts::PI, std::f32::consts::PI);
		points.push(Vec2::new(
			angle.cos() * radius,
			-angle.sin().mul_add(radius, half_height),
		));
	}

	points
}

#[cfg(test)]
mod tests {
	use ze_ecs::{Collider, RigidBody};

	use super::*;

	fn primary_camera() -> Camera {
		Camera {
			projection: CameraProjection::Orthographic {
				size: 10.0,
				near: -100.0,
				far: 100.0,
			},
			primary: true,
			clear_color: [0.1, 0.1, 0.1, 1.0],
		}
	}

	#[test]
	fn primary_camera_lookup_accepts_camera_on_mixed_component_entity() {
		let mut scene = Scene::new("main");
		crate::register_renderer_components(scene.registry_mut());
		let entity = scene.create_entity("Ball");
		scene.entity_mut(entity).add_component(Transform::default());
		scene.entity_mut(entity).add_component(primary_camera());
		scene.entity_mut(entity).add_component(RigidBody::default());
		scene.entity_mut(entity).add_component(Collider::default());
		scene.entity_mut(entity).add_component(Tag {
			tag: "Ball".to_string(),
		});

		assert_eq!(RenderSystem::primary_camera_entity(&scene), Some(entity));
		assert!(RenderSystem::primary_camera_exists(&scene));
	}

	#[test]
	fn primary_camera_lookup_ignores_inactive_primary_camera() {
		let mut scene = Scene::new("main");
		crate::register_renderer_components(scene.registry_mut());
		let entity = scene.create_entity("DisabledCamera");
		scene.entity_mut(entity).add_component(Transform::default());
		scene.entity_mut(entity).add_component(primary_camera());
		scene.entity_mut(entity).add_component(Inactive);

		assert_eq!(RenderSystem::primary_camera_entity(&scene), None);
		assert!(!RenderSystem::primary_camera_exists(&scene));
	}
}
