use std::f32::consts::FRAC_PI_2;

use glam::{
	Mat4, Vec2, Vec3, Vec4,
	camera::rh::{proj::directx, view},
};

/// The camera looks where `yaw` and `pitch` say and stands at `position`.
///
/// `yaw` is the turn around the `y` axis (zero looks along `+x`, a quarter of a
/// turn along `+z`), `pitch` the look up (positive) or down. Both are in
/// radians. There is no roll: the camera is always upright.
#[derive(Clone, Copy, Debug)]
pub struct Camera {
	pub position: Vec3,
	pub yaw: f32,
	pub pitch: f32,
}

impl Camera {
	#[must_use]
	pub const fn new(position: Vec3, yaw: f32, pitch: f32) -> Self {
		Self { position, yaw, pitch }
	}

	/// The direction the camera looks in (a unit vector).
	#[must_use]
	pub fn direction(&self) -> Vec3 {
		let (sin_pitch, cos_pitch) = self.pitch.sin_cos();
		let (sin_yaw, cos_yaw) = self.yaw.sin_cos();
		Vec3::new(cos_pitch * cos_yaw, sin_pitch, cos_pitch * sin_yaw)
	}

	/// World space to view space.
	#[must_use]
	pub fn calc_matrix(&self) -> Mat4 {
		view::look_to_mat4(self.position, self.direction(), Vec3::Y)
	}
}

/// How the camera sees: the lens.
#[derive(Clone, Copy, Debug)]
pub struct Projection {
	aspect: f32,
	/// Vertical field of view, in radians.
	pub fovy: f32,
	pub znear: f32,
	pub zfar: f32,
}

impl Projection {
	#[must_use]
	pub fn new(width: u32, height: u32, fovy: f32, znear: f32, zfar: f32) -> Self {
		Self {
			aspect: width.max(1) as f32 / height.max(1) as f32,
			fovy,
			znear,
			zfar,
		}
	}

	pub fn resize(&mut self, width: u32, height: u32) {
		self.aspect = width.max(1) as f32 / height.max(1) as f32;
	}

	/// View space to clip space (depth from 0 to 1, the way wgpu wants it).
	#[must_use]
	pub fn calc_matrix(&self) -> Mat4 {
		directx::perspective(self.fovy, self.aspect, self.znear, self.zfar)
	}
}

/// What the player asks of the camera in one frame. The renderer does not know
/// where it comes from: the runtime fills it from the keyboard, the mouse or a
/// gamepad.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct CameraInput {
	/// `1` forward, `-1` back (along the ground, whatever the pitch).
	pub forward: f32,
	/// `1` to the right, `-1` to the left.
	pub right: f32,
	/// `1` up, `-1` down.
	pub up: f32,
	/// How far the mouse moved this frame, in pixels (`x` to the right, `y`
	/// down).
	pub look: Vec2,
	/// How far the wheel turned this frame, in lines. It dollies the camera
	/// along its view direction.
	pub scroll: f32,
}

/// The camera cannot look straight up or down: that is where yaw loses its
/// meaning, and the image turns over.
const SAFE_PITCH: f32 = FRAC_PI_2 - 0.0001;

/// Flies a [`Camera`] about.
#[derive(Clone, Copy, Debug)]
pub struct CameraController {
	/// Units per second.
	pub speed: f32,
	/// Radians per pixel of the mouse.
	pub sensitivity: f32,
	/// Units of dolly per line of the wheel.
	pub scroll_speed: f32,
}

impl CameraController {
	#[must_use]
	pub const fn new(speed: f32, sensitivity: f32) -> Self {
		Self {
			speed,
			sensitivity,
			scroll_speed: 1.0,
		}
	}

	/// Moves and turns the camera by what was asked in `dt` seconds.
	pub fn update_camera(&self, camera: &mut Camera, input: &CameraInput, dt: f32) {
		let (yaw_sin, yaw_cos) = camera.yaw.sin_cos();
		let forward = Vec3::new(yaw_cos, 0.0, yaw_sin);
		let right = Vec3::new(-yaw_sin, 0.0, yaw_cos);

		camera.position += forward * input.forward * self.speed * dt;
		camera.position += right * input.right * self.speed * dt;
		camera.position.y = (input.up * self.speed).mul_add(dt, camera.position.y);

		// The wheel is a step, not a speed: it does not wait for `dt`.
		camera.position += camera.direction() * input.scroll * self.scroll_speed;

		// A mouse reports how far it moved, not how fast, so turning does not
		// depend on `dt` either (or a slow frame would turn the camera less).
		camera.yaw = input.look.x.mul_add(self.sensitivity, camera.yaw);
		camera.pitch = input
			.look
			.y
			.mul_add(-self.sensitivity, camera.pitch)
			.clamp(-SAFE_PITCH, SAFE_PITCH);
	}
}

/// The camera as the shaders read it.
#[repr(C)]
#[derive(Debug, Copy, Clone, bytemuck::Pod, bytemuck::Zeroable)]
pub struct CameraUniform {
	/// Where the camera is (`w` is `1`), for the specular light and the
	/// reflections.
	view_position: Vec4,
	view: Mat4,
	view_proj: Mat4,
	/// The sky is drawn from these two: they take a point of the screen back
	/// to a direction in the world.
	inv_proj: Mat4,
	inv_view: Mat4,
}

impl CameraUniform {
	#[must_use]
	pub const fn new() -> Self {
		Self {
			view_position: Vec4::ZERO,
			view: Mat4::IDENTITY,
			view_proj: Mat4::IDENTITY,
			inv_proj: Mat4::IDENTITY,
			inv_view: Mat4::IDENTITY,
		}
	}

	pub fn update_view_proj(&mut self, camera: &Camera, projection: &Projection) {
		let view = camera.calc_matrix();
		let proj = projection.calc_matrix();

		self.view_position = camera.position.extend(1.0);
		self.view = view;
		self.view_proj = proj * view;
		self.inv_proj = proj.inverse();
		self.inv_view = view.inverse();
	}
}

impl Default for CameraUniform {
	fn default() -> Self {
		Self::new()
	}
}
