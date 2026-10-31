use glam::{
	Mat4, Vec3,
	camera::rh::{proj::directx, view},
};

pub struct Camera {
	pub eye: Vec3,
	pub target: Vec3,
	pub up: Vec3,
	pub aspect: f32,
	pub fovy: f32, // Radians
	pub znear: f32,
	pub zfar: f32,
}

impl Camera {
	fn build_view_projection_matrix(&self) -> Mat4 {
		let view = view::look_at_mat4(self.eye, self.target, self.up);
		let proj = directx::perspective(self.fovy, self.aspect, self.znear, self.zfar);
		proj * view
	}
}

#[repr(C)]
#[derive(Debug, Copy, Clone, bytemuck::Pod, bytemuck::Zeroable)]
pub(crate) struct CameraUniform {
	view_proj: Mat4,
}

impl CameraUniform {
	pub const fn new() -> Self {
		Self {
			view_proj: Mat4::IDENTITY,
		}
	}

	pub fn update_view_proj(&mut self, camera: &Camera) {
		self.view_proj = camera.build_view_projection_matrix();
	}
}
