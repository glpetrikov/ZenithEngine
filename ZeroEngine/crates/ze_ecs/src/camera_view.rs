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

/// Current window viewport size, refreshed once per frame (before systems
/// run) by the app shell, which is the only place that owns the `Renderer`.
/// Lets `CameraViewSystem` compute a fresh `ActiveCameraView` from inside
/// `Scene::update_systems` without needing a `&Renderer` reference of its own.
#[derive(Debug, Clone, Copy)]
pub struct ViewportInfo {
	pub size: Vec2,
}

impl Unique for ViewportInfo {}

impl ViewportInfo {
	pub fn aspect_ratio(&self) -> f32 { self.size.x / self.size.y.max(1.0) }
}

/// The design/reference resolution that `ScreenSpaceOverlay` UI is authored
/// against. The UI canvas scale factor is `viewport_width / reference_width`
/// (match-by-width), so UI keeps its authored proportions and relative layout
/// at any window size or aspect ratio instead of having to be re-tested per
/// resolution.
///
/// Written once per frame by the app shell / editor from the active project's
/// UI settings, alongside `ViewportInfo`. When absent (e.g. a scene with no
/// project, or a unit test), consumers fall back to
/// `UiReferenceResolution::default()`, which is 1920x1080. When the live
/// viewport matches the reference exactly the scale factor is `1.0`, so
/// existing UI authored/tested at that resolution renders unchanged.
#[derive(Debug, Clone, Copy)]
pub struct UiReferenceResolution {
	pub size: Vec2,
}

impl Unique for UiReferenceResolution {}

impl Default for UiReferenceResolution {
	fn default() -> Self {
		Self {
			size: Vec2::new(1920.0, 1080.0),
		}
	}
}

impl UiReferenceResolution {
	/// The match-by-width scale factor for a given live viewport size: the
	/// ratio of the live viewport width to the reference width. Falls back to
	/// `1.0` when either width is non-positive (e.g. before the first frame
	/// has a real viewport), so UI never collapses to zero size.
	pub fn scale_for(&self, viewport_size: Vec2) -> f32 {
		if self.size.x > 0.0 && viewport_size.x > 0.0 {
			viewport_size.x / self.size.x
		} else {
			1.0
		}
	}
}

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

	/// Inverse of [`project_to_screen`](Self::project_to_screen): maps a
	/// screen-space pixel coordinate (origin top-left, +Y down) back onto the
	/// world-space `z = 0` plane. Intended for 2D cursor picking, where the
	/// camera is orthographic and every relevant object lives on that plane.
	/// Returns `None` if the viewport has zero size or the view-projection is
	/// non-invertible.
	pub fn unproject_to_world(&self, screen_pos: Vec2) -> Option<Vec2> {
		use ze_core::Vec4;

		if self.viewport_size.x <= 0.0 || self.viewport_size.y <= 0.0 {
			return None;
		}

		let ndc_x = (screen_pos.x / self.viewport_size.x).mul_add(2.0, -1.0);
		let ndc_y = -(screen_pos.y / self.viewport_size.y).mul_add(2.0, -1.0);

		let inverse = self.view_projection.inverse();
		if !inverse.is_finite() {
			return None;
		}

		let world = inverse * Vec4::new(ndc_x, ndc_y, 0.0, 1.0);
		if world.w.abs() <= f32::EPSILON {
			return None;
		}

		Some(Vec2::new(world.x / world.w, world.y / world.w))
	}
}

#[cfg(test)]
mod tests {
	use ze_core::{Mat4, Vec2, Vec3};

	use super::ActiveCameraView;

	#[test]
	fn unproject_is_inverse_of_project() {
		let camera = ActiveCameraView {
			view_projection: Mat4::orthographic_rh(-10.0, 10.0, -10.0, 10.0, -1.0, 1.0),
			viewport_size: Vec2::new(200.0, 100.0),
		};

		let world = Vec3::new(2.5, -3.0, 0.0);
		let screen = camera.project_to_screen(world).expect("point projects");
		let round_tripped = camera.unproject_to_world(screen).expect("point unprojects");

		assert!(
			(round_tripped.x - world.x).abs() < 1e-3,
			"x: {} vs {}",
			round_tripped.x,
			world.x
		);
		assert!(
			(round_tripped.y - world.y).abs() < 1e-3,
			"y: {} vs {}",
			round_tripped.y,
			world.y
		);
	}

	#[test]
	fn unproject_rejects_zero_viewport() {
		let camera = ActiveCameraView {
			view_projection: Mat4::IDENTITY,
			viewport_size: Vec2::ZERO,
		};

		assert!(camera.unproject_to_world(Vec2::new(10.0, 10.0)).is_none());
	}
}
