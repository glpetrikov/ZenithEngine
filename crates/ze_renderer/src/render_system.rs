use ze_assets::ResourceManager;
use ze_core::{Mat4, Result, Vec2, Vec3};
use ze_ecs::{
	ActiveCameraView, Collider, ColliderShape, EditorOnly, EntitiesView, EntityId, Inactive, Name, PhysicsSettings,
	RigidBody, RigidBodyType, Scene, System, Tag, Transform, UiReferenceResolution, ViewportInfo, fit_aspect,
	shipyard::{IntoIter, UniqueView, View},
};
use ze_input::{Input, ZKeyCode};

use crate::{
	Renderer,
	components::{Camera, CameraProjection, Sprite, SpriteColorSettings, SpriteSize, TextureSource},
};

#[derive(Debug, Clone)]
pub struct CameraRenderData {
	pub view_projection: Mat4,
	pub clear_color: [f32; 4],
	/// Top-left offset and size, in render-target pixels, of the letterboxed/
	/// pillarboxed sub-rectangle the GPU viewport/scissor should be
	/// restricted to -- `Vec2::ZERO`/`Vec2::ZERO` (the `build_camera_data`
	/// default) means "not computed, use the full render target", which is
	/// what every caller except `RenderSystem::render_scene` wants (tests,
	/// and the scripting fallback path that only reads `view_projection`).
	pub viewport_offset: Vec2,
	pub viewport_size: Vec2,
}

#[derive(Debug, Clone)]
pub struct SpriteRenderItem {
	pub entity: ze_ecs::ze_entity_id::ZeEntityId,
	pub label: String,
	pub transform: Mat4,
	pub texture_source: TextureSource,
	pub size: SpriteSize,
	pub color: SpriteColorSettings,
	pub layer: i32,
	pub texture_rotation_degrees: f32,
	pub glow_strength: f32,
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

/// Finds the entity to render through: the entity with the primary `Camera`
/// component if one is set, otherwise the first available camera in the
/// scene. Either way, non-editor cameras are preferred over `EditorOnly`
/// ones, and `Inactive` entities are never considered.
///
/// Camera switching isn't supported yet, so requiring a scene to have exactly
/// one correctly-flagged Primary camera is more fragile than useful: it's
/// easy to forget to set/re-set the flag on a new camera, and there's no way
/// yet to pick a different camera at runtime anyway. Falling back to "first
/// available" means a scene with a single camera just works, while `primary`
/// remains a valid explicit override for scenes that do have more than one
/// usable camera.
pub fn find_primary_camera_entity(scene: &Scene) -> Option<EntityId> {
	let world = scene.world();
	let Ok(cameras) = world.borrow::<View<Camera>>() else {
		return None;
	};
	let mut primary_game_camera = None;
	let mut primary_editor_camera = None;
	let mut first_game_camera = None;
	let mut first_editor_camera = None;

	for (entity, camera) in cameras.iter().with_id() {
		if world.get::<&Inactive>(entity).is_ok() {
			continue;
		}
		let is_editor_only = world.get::<&EditorOnly>(entity).is_ok();

		if camera.primary && !is_editor_only {
			// Highest-priority match: a primary, non-editor-only camera wins outright.
			primary_game_camera = Some(entity);
			break;
		}
		if camera.primary && is_editor_only && primary_editor_camera.is_none() {
			primary_editor_camera = Some(entity);
		}
		if !is_editor_only && first_game_camera.is_none() {
			first_game_camera = Some(entity);
		}
		if is_editor_only && first_editor_camera.is_none() {
			first_editor_camera = Some(entity);
		}
	}

	primary_game_camera
		.or(primary_editor_camera)
		.or(first_game_camera)
		.or(first_editor_camera)
}

/// Builds the combined view-projection matrix for the given camera.
///
/// `aspect` should be the camera's fixed reference aspect ratio (see
/// `UiReferenceResolution::aspect_ratio`), not the runtime viewport's aspect
/// ratio -- the visible world area must stay fixed across resolutions, with
/// `fit_aspect` handling the letterbox/pillarbox mismatch against the actual
/// viewport separately. Leaves `viewport_offset`/`viewport_size` zeroed;
/// callers that need the GPU viewport rect (`RenderSystem::render_scene`)
/// fill those in themselves.
pub fn build_camera_data(transform: &Transform, camera: &Camera, aspect: f32) -> CameraRenderData {
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
		viewport_offset: Vec2::ZERO,
		viewport_size: Vec2::ZERO,
	}
}

/// Locates the primary camera in `scene` and builds its view-projection
/// matrix for the given viewport aspect ratio.
pub fn compute_active_camera(scene: &Scene, aspect: f32) -> Option<CameraRenderData> {
	let entity = find_primary_camera_entity(scene)?;
	let world = scene.world();
	let camera = world.get::<&Camera>(entity).ok()?;
	let transform = scene.world_transform(entity)?;
	Some(build_camera_data(&transform, &camera, aspect))
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
		let target_aspect = reference_aspect(scene);
		let Some(mut camera) = compute_active_camera(scene, target_aspect) else {
			self.items.clear();
			self.debug_lines.clear();
			let _ = scene.world().remove_unique::<ActiveCameraView>();
			if self.missing_primary_camera_reported_scene.as_deref() != Some(&scene.name) {
				self.missing_primary_camera_reported_scene = Some(scene.name.clone());
				let name = if scene.name.is_empty() { "unnamed" } else { &scene.name };
				ze_log::error!("No usable camera found in scene `{name}`");
			}
			return RenderStatus::MissingPrimaryCamera {
				scene_name: scene.name.clone(),
			};
		};

		// Restrict the GPU viewport/scissor to the letterboxed/pillarboxed
		// sub-rect of the render target that matches `target_aspect`, so the
		// visible world area is identical regardless of the window/panel's
		// actual shape -- the rest is left at the camera's clear color,
		// forming the bars.
		let render_target_size = renderer.viewport_size();
		let (viewport_offset, viewport_size) = fit_aspect(
			Vec2::new(render_target_size.width as f32, render_target_size.height as f32),
			target_aspect,
		);
		camera.viewport_offset = viewport_offset;
		camera.viewport_size = viewport_size;

		// ActiveCameraView is already set by CameraViewSystem earlier this
		// frame — do not overwrite it here, which would silently replace the
		// value scripts may have already read with a potentially-different
		// viewport size.

		let _prev_count = self.items.len();
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

	#[cfg(test)]
	fn primary_camera_exists(scene: &Scene) -> bool { find_primary_camera_entity(scene).is_some() }

	fn collect_items(scene: &Scene) -> Vec<SpriteRenderItem> {
		let mut items = Vec::new();
		let world = scene.world();

		let mut total_entities = 0u32;
		let mut has_sprite = 0u32;
		let mut has_inactive = 0u32;
		let mut visible_sprites = 0u32;
		let mut no_transform = 0u32;

		world.run(|entities: EntitiesView| {
			for entity in entities.iter() {
				total_entities += 1;
				if world.get::<&Inactive>(entity).is_ok() {
					has_inactive += 1;
					continue;
				}

				let Ok(sprite) = world.get::<&Sprite>(entity) else {
					continue;
				};

				has_sprite += 1;

				if !sprite.settings.visible {
					continue;
				}

				let Some(transform) = scene.world_transform(entity) else {
					no_transform += 1;
					continue;
				};

				visible_sprites += 1;

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
					texture_source: sprite.texture.clone(),
					size: sprite.size.clone(),
					color: sprite.color.clone(),
					layer: sprite.settings.layer,
					texture_rotation_degrees: sprite.settings.texture_rotation_degrees,
					glow_strength: sprite.glow_strength,
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

/// The aspect ratio the world camera treats as fixed across every
/// resolution, sourced from the project's `UiReferenceResolution` (shared
/// with the UI match-by-width scale factor) so there's a single place that
/// defines "the resolution this project was designed at". Falls back to its
/// 16:9 default when the unique is absent (e.g. a scene with no project, or
/// a unit test).
pub fn reference_aspect(scene: &Scene) -> f32 {
	scene.world().borrow::<UniqueView<UiReferenceResolution>>().map_or_else(
		|_| UiReferenceResolution::default().aspect_ratio(),
		|r| r.aspect_ratio(),
	)
}

fn refresh_active_camera_view(scene: &mut Scene) -> Result<()> {
	let Some(viewport) = scene
		.world()
		.borrow::<UniqueView<ViewportInfo>>()
		.ok()
		.map(|view| *view)
	else {
		return Ok(());
	};

	let target_aspect = reference_aspect(scene);
	let Some(camera) = compute_active_camera(scene, target_aspect) else {
		let _ = scene.world().remove_unique::<ActiveCameraView>();
		return Ok(());
	};

	let (viewport_offset, viewport_size) = fit_aspect(viewport.size, target_aspect);
	scene.world().add_unique(ActiveCameraView {
		view_projection: camera.view_projection,
		viewport_size,
		viewport_offset,
	});
	Ok(())
}

/// Refreshes `ActiveCameraView` from the primary camera's transform *before*
/// `ScriptingSystem` runs, so C# scripts calling `GetMouseWorldPosition()`
/// read a fresh view instead of a stale/missing one. Since this runs before
/// `PhysicsSystem`/`ScriptingSystem` move anything, the view it produces is
/// last tick's post-simulation camera transform -- acceptable for cursor
/// picking, but stale for `UISystem`'s `UIAnchorMode::WorldSpace` projection
/// (see `LateCameraViewSystem`, which exists specifically to refresh it again
/// after simulation for that purpose).
#[derive(Default)]
pub struct CameraViewSystem;

impl CameraViewSystem {
	pub fn new() -> Self { Self::default() }
}

impl System for CameraViewSystem {
	fn name(&self) -> &'static str { "CameraViewSystem" }

	fn update(&mut self, scene: &mut Scene, _dt: f32) -> Result<()> { refresh_active_camera_view(scene) }

	fn as_any_mut(&mut self) -> &mut dyn std::any::Any { self }
}

/// Re-refreshes `ActiveCameraView` from the primary camera's *current-tick*
/// transform, after `PhysicsSystem`/`ScriptingSystem`/`AnimationSystem` and
/// before `UISystem` runs.
///
/// `ActiveCameraView` used to only be written by `RenderSystem::render_scene`,
/// which runs during `RedrawRequested`, strictly after `UISystem::update` had
/// already run for that tick inside `Scene::update_systems`. `UISystem`
/// projects `UIAnchorMode::WorldSpace` entity positions with this matrix (see
/// `resolve_screen_pos`), so it was always one whole frame behind: fine while
/// the camera is still, but visibly-lagging/drifting UI anchors relative to
/// the sprite whenever the camera (or the entity the UI anchors to) moves
/// every tick -- e.g. a health bar following a falling/moving entity drifts
/// steadily downward, one tick behind, rather than jittering symmetrically.
/// Recomputing it here -- after simulation, before `UISystem` -- makes it
/// fresh for this tick's `UISystem` run. `render_scene` still recomputes its
/// own copy for the actual draw; that's a few redundant matrix multiplies,
/// not a correctness concern, since both computations read the same
/// already-physics-stepped transform.
///
/// This deliberately uses `find_primary_camera`, matching the camera
/// `render_scene` actually renders through: World Space UI is meant to stay
/// visually attached to whatever's on screen, including the editor's
/// `EditorOnly` camera while it's the one flagged `primary` and being
/// panned/zoomed for scene navigation in Edit mode. `UIAnchorMode::
/// ScreenSpaceOverlay` elements don't consult `ActiveCameraView` at all, so
/// they're unaffected either way.
#[derive(Default)]
pub struct LateCameraViewSystem;

impl LateCameraViewSystem {
	pub fn new() -> Self { Self::default() }
}

impl System for LateCameraViewSystem {
	fn name(&self) -> &'static str { "LateCameraViewSystem" }

	fn update(&mut self, scene: &mut Scene, _dt: f32) -> Result<()> { refresh_active_camera_view(scene) }

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

		assert_eq!(find_primary_camera_entity(&scene), Some(entity));
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

		assert_eq!(find_primary_camera_entity(&scene), None);
		assert!(!RenderSystem::primary_camera_exists(&scene));
	}

	#[test]
	fn primary_camera_lookup_prefers_non_editor_camera_over_editor_only_primary() {
		let mut scene = Scene::new("main");
		crate::register_renderer_components(scene.registry_mut());

		let editor_camera = scene.create_entity("EditorCamera");
		scene.entity_mut(editor_camera).add_component(Transform::default());
		scene.entity_mut(editor_camera).add_component(primary_camera());
		scene.entity_mut(editor_camera).add_component(EditorOnly);

		let game_camera = scene.create_entity("GameCamera");
		scene.entity_mut(game_camera).add_component(Transform::default());
		scene.entity_mut(game_camera).add_component(primary_camera());

		assert_eq!(find_primary_camera_entity(&scene), Some(game_camera));
	}

	#[test]
	fn primary_camera_lookup_falls_back_to_editor_only_camera_when_no_game_camera() {
		let mut scene = Scene::new("main");
		crate::register_renderer_components(scene.registry_mut());

		let editor_camera = scene.create_entity("EditorCamera");
		scene.entity_mut(editor_camera).add_component(Transform::default());
		scene.entity_mut(editor_camera).add_component(primary_camera());
		scene.entity_mut(editor_camera).add_component(EditorOnly);

		assert_eq!(find_primary_camera_entity(&scene), Some(editor_camera));
	}

	fn non_primary_camera() -> Camera {
		Camera {
			projection: CameraProjection::Orthographic {
				size: 10.0,
				near: -100.0,
				far: 100.0,
			},
			primary: false,
			clear_color: [0.1, 0.1, 0.1, 1.0],
		}
	}

	#[test]
	fn primary_camera_lookup_falls_back_to_first_camera_when_none_marked_primary() {
		let mut scene = Scene::new("main");
		crate::register_renderer_components(scene.registry_mut());
		let entity = scene.create_entity("Camera");
		scene.entity_mut(entity).add_component(Transform::default());
		scene.entity_mut(entity).add_component(non_primary_camera());

		assert_eq!(find_primary_camera_entity(&scene), Some(entity));
	}

	#[test]
	fn primary_camera_lookup_fallback_prefers_game_camera_over_editor_only() {
		let mut scene = Scene::new("main");
		crate::register_renderer_components(scene.registry_mut());

		let editor_camera = scene.create_entity("EditorCamera");
		scene.entity_mut(editor_camera).add_component(Transform::default());
		scene.entity_mut(editor_camera).add_component(non_primary_camera());
		scene.entity_mut(editor_camera).add_component(EditorOnly);

		let game_camera = scene.create_entity("GameCamera");
		scene.entity_mut(game_camera).add_component(Transform::default());
		scene.entity_mut(game_camera).add_component(non_primary_camera());

		assert_eq!(find_primary_camera_entity(&scene), Some(game_camera));
	}

	#[test]
	fn primary_camera_lookup_fallback_ignores_inactive_camera() {
		let mut scene = Scene::new("main");
		crate::register_renderer_components(scene.registry_mut());

		let inactive_camera = scene.create_entity("InactiveCamera");
		scene.entity_mut(inactive_camera).add_component(Transform::default());
		scene.entity_mut(inactive_camera).add_component(non_primary_camera());
		scene.entity_mut(inactive_camera).add_component(Inactive);

		let active_camera = scene.create_entity("ActiveCamera");
		scene.entity_mut(active_camera).add_component(Transform::default());
		scene.entity_mut(active_camera).add_component(non_primary_camera());

		assert_eq!(find_primary_camera_entity(&scene), Some(active_camera));
	}

	/// A stand-in for "something that moves the camera during simulation this
	/// tick" (physics, a following script, ...). Registered between
	/// `CameraViewSystem` and `LateCameraViewSystem`, exactly like
	/// `PhysicsSystem`/`ScriptingSystem`/`AnimationSystem` are in
	/// `ze_app::load_project_scene`.
	struct TranslateCameraDown(EntityId);

	impl System for TranslateCameraDown {
		fn name(&self) -> &'static str { "TranslateCameraDown" }

		fn update(&mut self, scene: &mut Scene, _dt: f32) -> Result<()> {
			let mut transform = scene.world().get::<&mut Transform>(self.0)?;
			transform.position.y -= 5.0;
			Ok(())
		}

		fn as_any_mut(&mut self) -> &mut dyn std::any::Any { self }
	}

	/// Regression test for the World Space UI drift bug: `UISystem` (which
	/// runs after simulation, see `ze_app::load_project_scene`) must see an
	/// `ActiveCameraView` reflecting *this tick's* camera transform, not the
	/// transform from before simulation moved the camera. Without
	/// `LateCameraViewSystem` running after whatever moves the camera,
	/// `ActiveCameraView` would still hold the pre-move projection, and a
	/// World Space UI element would render one tick behind a moving camera --
	/// which reads as steady drift under continuous camera motion.
	#[test]
	fn late_camera_view_system_reflects_this_ticks_camera_move() {
		let mut scene = Scene::new("main");
		crate::register_renderer_components(scene.registry_mut());

		let camera = scene.create_entity("Camera");
		scene.entity_mut(camera).add_component(Transform::default());
		scene.entity_mut(camera).add_component(primary_camera());

		scene.world().add_unique(ViewportInfo {
			size: Vec2::new(1920.0, 1080.0),
		});

		let aspect = 1920.0 / 1080.0;
		// Ground truth for "what the camera view looked like before this
		// tick's move" -- computed directly, not via any System.
		let stale_view = compute_active_camera(&scene, aspect).expect("camera should resolve");

		scene.add_system(CameraViewSystem::new());
		scene.add_system(TranslateCameraDown(camera));
		scene.add_system(LateCameraViewSystem::new());

		scene.update_systems(1.0).expect("systems should update cleanly");

		// Ground truth for "what the camera view looks like after this
		// tick's move" -- computed directly, matching what UISystem should see.
		let fresh_view = compute_active_camera(&scene, aspect).expect("camera should resolve");

		let active_view = *scene
			.world()
			.borrow::<UniqueView<ActiveCameraView>>()
			.expect("LateCameraViewSystem should have written ActiveCameraView");

		assert_eq!(
			active_view.view_projection, fresh_view.view_projection,
			"ActiveCameraView must be recomputed from the camera's post-move transform \
			 (this tick's), not left over from before TranslateCameraDown ran"
		);
		assert_ne!(
			active_view.view_projection, stale_view.view_projection,
			"the camera actually moved this tick, so its view-projection must have changed"
		);

		// A world-space UI anchor at the origin must project to a different
		// screen position once the camera has moved -- if UISystem read the
		// stale pre-move view (the regression this test guards against), it
		// would incorrectly still match the pre-move projection.
		let world_pos = Vec3::new(0.0, 0.0, 0.0);
		let stale_active_view = ActiveCameraView {
			view_projection: stale_view.view_projection,
			viewport_size: Vec2::new(1920.0, 1080.0),
			viewport_offset: Vec2::ZERO,
		};
		assert_ne!(
			active_view.project_to_screen(world_pos),
			stale_active_view.project_to_screen(world_pos),
			"World Space UI anchored at this point must move on screen once the camera moves"
		);
	}
}
