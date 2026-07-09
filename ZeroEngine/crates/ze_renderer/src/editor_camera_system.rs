use ze_core::{Result, Vec3};
use ze_ecs::{EditorOnly, EntitiesView, EntityId, Scene, System, Transform};
use ze_input::{Input, ZKeyCode};

use crate::{Camera, CameraProjection};

pub struct EditorCameraSystem {
	speed: f32,
	fast_speed: f32,
	zoom_step: f32,
}

impl EditorCameraSystem {
	pub const fn new() -> Self {
		Self {
			speed: 5.0,
			fast_speed: 10.0,
			zoom_step: 0.1,
		}
	}

	fn primary_camera_entity(scene: &Scene) -> Option<EntityId> {
		let world = scene.world();
		let mut primary = None;

		world.run(|entities: EntitiesView| {
			for entity in entities.iter() {
				let Ok(camera) = world.get::<&Camera>(entity) else {
					continue;
				};

				if camera.primary {
					primary = Some(entity);
					break;
				}
			}
		});

		primary
	}
}

impl Default for EditorCameraSystem {
	fn default() -> Self { Self::new() }
}

impl System for EditorCameraSystem {
	fn name(&self) -> &'static str { "EditorCameraSystem" }

	fn update(&mut self, scene: &mut Scene, dt: f32) -> Result<()> {
		let Some(entity) = Self::primary_camera_entity(scene) else {
			return Ok(());
		};
		if !Self::is_editor_camera(scene, entity) {
			return Ok(());
		}

		let mut direction = Vec3::ZERO;

		if Input::is_key_pressed(ZKeyCode::W) {
			direction.y += 1.0;
		}
		if Input::is_key_pressed(ZKeyCode::S) {
			direction.y -= 1.0;
		}
		if Input::is_key_pressed(ZKeyCode::D) {
			direction.x += 1.0;
		}
		if Input::is_key_pressed(ZKeyCode::A) {
			direction.x -= 1.0;
		}
		if Input::is_key_pressed(ZKeyCode::Q) {
			direction.z += 1.0;
		}
		if Input::is_key_pressed(ZKeyCode::E) {
			direction.z -= 1.0;
		}

		let wheel_delta = Input::get_mouse_wheel_delta();

		if direction == Vec3::ZERO && wheel_delta == 0.0 {
			return Ok(());
		}

		if direction != Vec3::ZERO {
			let speed = if Input::is_key_pressed(ZKeyCode::LShift) {
				self.fast_speed
			} else {
				self.speed
			};

			let mut transform = scene.world_mut().get::<&mut Transform>(entity)?;
			transform.position += direction.normalize() * speed * dt;
		}

		if wheel_delta != 0.0 {
			let mut camera = scene.world_mut().get::<&mut Camera>(entity)?;
			match &mut camera.projection {
				CameraProjection::Orthographic { size, .. } => {
					let zoom_factor = wheel_delta.mul_add(-self.zoom_step, 1.0).clamp(0.2, 5.0);
					*size = (*size * zoom_factor).clamp(0.1, 1000.0);
				}
				CameraProjection::Perspective { fov_y_radians, .. } => {
					let zoom_delta = wheel_delta * self.zoom_step;
					*fov_y_radians = (*fov_y_radians - zoom_delta).clamp(10.0_f32.to_radians(), 120.0_f32.to_radians());
				}
			}
		}

		Ok(())
	}

	fn as_any_mut(&mut self) -> &mut dyn std::any::Any { self }
}

impl EditorCameraSystem {
	fn is_editor_camera(scene: &Scene, entity: EntityId) -> bool {
		let world = scene.world();
		world.get::<&EditorOnly>(entity).is_ok()
	}
}
