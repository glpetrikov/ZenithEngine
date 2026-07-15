use shipyard::Unique;
use ze_core::{Mat4, Vec2, Vec3};

/// Per-frame snapshot of the primary camera's view-projection matrix and the
/// viewport pixel size it was built for.
///
/// Written once per frame by the renderer (which owns the
/// `Camera`/`CameraProjection` component types) and read by systems, such as
/// `UISystem`, that need to project world positions into screen space
/// without depending on renderer-specific types.
#[derive(Debug, Clone, Copy)]
pub struct ActiveCameraView {
	pub view_projection: Mat4,
	pub viewport_size: Vec2,
}

impl Unique for ActiveCameraView {}

impl ActiveCameraView {
	/// Projects a world-space position to screen-space pixel coordinates
	/// (origin top-left, +Y down), matching the convention used by the
	/// editor's viewport picking code. Returns `None` if the point is behind
	/// the camera or the viewport has zero size.
	pub fn project_to_screen(&self, world_pos: Vec3) -> Option<Vec2> {
		if self.viewport_size.x <= 0.0 || self.viewport_size.y <= 0.0 {
			return None;
		}

		let clip = self.view_projection * world_pos.extend(1.0);
		if clip.w <= f32::EPSILON {
			return None;
		}

		let ndc = clip.truncate() / clip.w;
		let screen_x = ndc.x.mul_add(0.5, 0.5) * self.viewport_size.x;
		let screen_y = (1.0 - ndc.y.mul_add(0.5, 0.5)) * self.viewport_size.y;
		Some(Vec2::new(screen_x, screen_y))
	}
}
