//! The camera of the Viewport: it flies around the world and is not an entity
//! of it.

use glam::{DVec3, Mat4, Quat, Vec2, Vec3};
use zenith_renderer::components::Projection;

/// Radians of turning for each pixel the mouse moves.
const LOOK_SPEED: f32 = 0.0035;
/// The steepest the camera looks up or down.
const MAX_PITCH: f32 = 89.0 * std::f32::consts::PI / 180.0;
const MIN_SPEED: f32 = 0.1;
const MAX_SPEED: f32 = 500.0;
/// What a step of the mouse wheel does to the speed.
const SPEED_STEP: f32 = 1.15;
/// Units a step of the wheel moves the camera, for each unit of speed.
const DOLLY: f32 = 0.25;
/// How much faster Shift makes it.
const FAST: f32 = 4.0;

/// What the user asked the camera for in a frame.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct FlyInput {
	/// -1..1 on each axis: x to the right, y up, z forward.
	pub movement: Vec3,
	/// How far the mouse moved, in pixels.
	pub look: Vec2,
	pub fast: bool,
	/// Steps of the mouse wheel: the speed of flying changes by them.
	pub scroll: f32,
}

/// A camera that is moved with the keys and turned with the mouse.
#[derive(Clone, Debug, PartialEq)]
pub struct EditorCamera {
	pub position: DVec3,
	/// Turn around the Y axis, in radians (0 looks along -Z).
	pub yaw: f32,
	/// Turn up (positive) or down, in radians.
	pub pitch: f32,
	/// Units a second.
	pub speed: f32,
	pub projection: Projection,
}

impl Default for EditorCamera {
	fn default() -> Self {
		Self {
			position: DVec3::new(0.0, 3.0, 8.0),
			yaw: 0.0,
			pitch: -0.3,
			speed: 6.0,
			projection: Projection::default(),
		}
	}
}

impl EditorCamera {
	pub fn rotation(&self) -> Quat {
		Quat::from_rotation_y(self.yaw) * Quat::from_rotation_x(self.pitch)
	}

	/// The way the camera looks.
	pub fn forward(&self) -> Vec3 {
		self.rotation() * Vec3::NEG_Z
	}

	/// Turns and moves the camera for a frame of `dt` seconds.
	pub fn fly(&mut self, input: &FlyInput, dt: f32) {
		self.yaw = input.look.x.mul_add(-LOOK_SPEED, self.yaw);
		self.pitch = input
			.look
			.y
			.mul_add(-LOOK_SPEED, self.pitch)
			.clamp(-MAX_PITCH, MAX_PITCH);
		if input.scroll != 0.0 {
			self.speed = (self.speed * SPEED_STEP.powf(input.scroll)).clamp(MIN_SPEED, MAX_SPEED);
		}
		let local = Vec3::new(input.movement.x, input.movement.y, -input.movement.z).normalize_or_zero();
		let speed = self.speed * if input.fast { FAST } else { 1.0 };
		self.position += (self.rotation() * local * speed * dt).as_dvec3();
	}

	/// Comes closer (positive `steps`) or goes away along the way it looks, by
	/// the wheel of the mouse.
	pub fn dolly(&mut self, steps: f32) {
		self.position += (self.forward() * steps * self.speed * DOLLY).as_dvec3();
	}

	/// Stands `distance` away from `target`, looking at it.
	pub fn focus(&mut self, target: DVec3, distance: f64) {
		self.position = target - self.forward().as_dvec3() * distance;
	}

	/// World to view: the camera is at the origin (everything the renderer
	/// draws is relative to it).
	pub fn view_matrix(&self) -> Mat4 {
		Mat4::from_quat(self.rotation().inverse())
	}

	/// View to clip space in the way `ImGuizmo` expects (depth from -1 to 1;
	/// the renderer's own goes from 0 to 1).
	pub fn gizmo_projection(&self, size: (u32, u32)) -> Mat4 {
		let to_opengl = Mat4::from_cols(
			glam::Vec4::new(1.0, 0.0, 0.0, 0.0),
			glam::Vec4::new(0.0, 1.0, 0.0, 0.0),
			glam::Vec4::new(0.0, 0.0, 2.0, 0.0),
			glam::Vec4::new(0.0, 0.0, -1.0, 1.0),
		);
		to_opengl * self.projection.matrix(size)
	}
}

#[cfg(test)]
mod tests {
	use super::*;

	fn close(a: Vec3, b: Vec3) -> bool {
		(a - b).length() < 1e-4
	}

	#[test]
	fn the_camera_starts_looking_down_a_bit_along_minus_z() {
		let camera = EditorCamera {
			pitch: 0.0,
			..EditorCamera::default()
		};
		assert!(close(camera.forward(), Vec3::NEG_Z));
		assert!(EditorCamera::default().forward().y < 0.0);
	}

	#[test]
	fn forward_goes_where_it_looks_and_right_goes_sideways() {
		let mut camera = EditorCamera {
			position: DVec3::ZERO,
			pitch: 0.0,
			speed: 2.0,
			..EditorCamera::default()
		};
		camera.fly(
			&FlyInput {
				movement: Vec3::new(0.0, 0.0, 1.0),
				..FlyInput::default()
			},
			1.0,
		);
		assert!(close(camera.position.as_vec3(), Vec3::new(0.0, 0.0, -2.0)));
		camera.fly(
			&FlyInput {
				movement: Vec3::new(1.0, 0.0, 0.0),
				..FlyInput::default()
			},
			0.5,
		);
		assert!(close(camera.position.as_vec3(), Vec3::new(1.0, 0.0, -2.0)));
	}

	#[test]
	fn going_diagonally_is_not_faster_and_shift_is() {
		let mut slow = EditorCamera {
			position: DVec3::ZERO,
			pitch: 0.0,
			speed: 1.0,
			..EditorCamera::default()
		};
		let mut fast = slow.clone();
		let diagonal = Vec3::new(1.0, 0.0, 1.0);
		slow.fly(
			&FlyInput {
				movement: diagonal,
				..FlyInput::default()
			},
			1.0,
		);
		fast.fly(
			&FlyInput {
				movement: diagonal,
				fast: true,
				..FlyInput::default()
			},
			1.0,
		);
		assert!((slow.position.length() - 1.0).abs() < 1e-6);
		assert!((fast.position.length() - f64::from(FAST)).abs() < 1e-6);
	}

	#[test]
	fn the_mouse_turns_it_and_it_cannot_look_over_the_top() {
		let mut camera = EditorCamera::default();
		let before = camera.yaw;
		camera.fly(
			&FlyInput {
				look: Vec2::new(100.0, 0.0),
				..FlyInput::default()
			},
			0.0,
		);
		assert!(camera.yaw < before, "moving the mouse right turns it right");
		camera.fly(
			&FlyInput {
				look: Vec2::new(0.0, -100_000.0),
				..FlyInput::default()
			},
			0.0,
		);
		assert!((camera.pitch - MAX_PITCH).abs() < 1e-6);
	}

	#[test]
	fn the_wheel_changes_the_speed_within_limits() {
		let mut camera = EditorCamera::default();
		let before = camera.speed;
		camera.fly(
			&FlyInput {
				scroll: 2.0,
				..FlyInput::default()
			},
			0.0,
		);
		assert!(camera.speed > before);
		camera.fly(
			&FlyInput {
				scroll: -10_000.0,
				..FlyInput::default()
			},
			0.0,
		);
		assert!((camera.speed - MIN_SPEED).abs() < 1e-6);
	}

	#[test]
	fn the_wheel_moves_the_camera_along_where_it_looks() {
		let mut camera = EditorCamera::default();
		let before = camera.position;
		camera.dolly(2.0);
		let moved = (camera.position - before).as_vec3();
		assert!(moved.dot(camera.forward()) > 0.0);
		camera.dolly(-2.0);
		assert!((camera.position - before).length() < 1e-6, "and back");
	}

	#[test]
	fn focusing_puts_the_target_in_front_of_the_camera() {
		let mut camera = EditorCamera::default();
		let target = DVec3::new(5.0, 1.0, -3.0);
		camera.focus(target, 4.0);
		let ahead = camera.position + camera.forward().as_dvec3() * 4.0;
		assert!((ahead - target).length() < 1e-4);
	}

	#[test]
	fn the_gizmo_projection_maps_near_and_far_to_minus_one_and_one() {
		let camera = EditorCamera::default();
		let Projection::Perspective { near, far, .. } = camera.projection else {
			return;
		};
		let matrix = camera.gizmo_projection((200, 100));
		let at = |distance: f32| matrix.project_point3(Vec3::new(0.0, 0.0, -distance)).z;
		assert!((at(near) + 1.0).abs() < 1e-3, "{}", at(near));
		assert!((at(far) - 1.0).abs() < 1e-3, "{}", at(far));
	}
}
