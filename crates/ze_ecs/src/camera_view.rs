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
	/// Size, in the same pixel units as `ViewportInfo`, of the letterboxed/
	/// pillarboxed sub-rectangle the camera actually renders into -- not
	/// necessarily the full window/panel size. See [`fit_aspect`].
	pub viewport_size: Vec2,
	/// Top-left offset of that sub-rectangle within the full window/panel,
	/// i.e. the pillarbox/letterbox bar thickness on the left/top side.
	/// `Vec2::ZERO` when the runtime aspect ratio already matches the
	/// camera's reference aspect (no bars).
	pub viewport_offset: Vec2,
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

/// Computes the centered sub-rectangle of `viewport_size` that preserves
/// `target_aspect`, returning `(offset, size)` in the same units as
/// `viewport_size` (pixels or logical points). This is the classic
/// letterbox/pillarbox "fit" calculation: whichever axis is more constraining
/// is filled edge-to-edge, and the other axis is shrunk to match
/// `target_aspect`, leaving equal bars on both of its sides.
///
/// Used so the camera's visible world area is identical at every window
/// shape (see `Camera`'s reference-aspect projection in `ze_renderer`) --
/// without this, a level boundary collider positioned to match the visible
/// edge at one aspect ratio would not match at another.
///
/// Falls back to filling `viewport_size` with no offset if either dimension
/// or `target_aspect` is non-positive.
pub fn fit_aspect(viewport_size: Vec2, target_aspect: f32) -> (Vec2, Vec2) {
	if viewport_size.x <= 0.0 || viewport_size.y <= 0.0 || target_aspect <= 0.0 {
		return (Vec2::ZERO, viewport_size);
	}

	let runtime_aspect = viewport_size.x / viewport_size.y;
	let size = if runtime_aspect > target_aspect {
		Vec2::new(viewport_size.y * target_aspect, viewport_size.y)
	} else {
		Vec2::new(viewport_size.x, viewport_size.x / target_aspect)
	};
	let offset = Vec2::new((viewport_size.x - size.x) * 0.5, (viewport_size.y - size.y) * 0.5);
	(offset, size)
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

	/// The aspect ratio (width / height) the world camera treats as its
	/// reference: the visible world area is fixed at every resolution
	/// relative to this ratio, with letterbox/pillarbox bars filling any
	/// mismatch against the live viewport (see [`fit_aspect`]). Falls back to
	/// the default (16:9) when either dimension is non-positive.
	pub fn aspect_ratio(&self) -> f32 {
		if self.size.x > 0.0 && self.size.y > 0.0 {
			self.size.x / self.size.y
		} else {
			let default = Self::default().size;
			default.x / default.y
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
		let screen_x = ndc
			.x
			.mul_add(0.5, 0.5)
			.mul_add(self.viewport_size.x, self.viewport_offset.x);
		let screen_y = (1.0 - ndc.y.mul_add(0.5, 0.5)).mul_add(self.viewport_size.y, self.viewport_offset.y);
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

		let local = screen_pos - self.viewport_offset;
		let ndc_x = (local.x / self.viewport_size.x).mul_add(2.0, -1.0);
		let ndc_y = -(local.y / self.viewport_size.y).mul_add(2.0, -1.0);

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

	use super::{ActiveCameraView, fit_aspect};

	#[test]
	fn unproject_is_inverse_of_project() {
		let camera = ActiveCameraView {
			view_projection: Mat4::orthographic_rh(-10.0, 10.0, -10.0, 10.0, -1.0, 1.0),
			viewport_size: Vec2::new(200.0, 100.0),
			viewport_offset: Vec2::ZERO,
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
			viewport_offset: Vec2::ZERO,
		};

		assert!(camera.unproject_to_world(Vec2::new(10.0, 10.0)).is_none());
	}

	#[test]
	fn unproject_roundtrips_with_different_aspect_ratios() {
		let world_points = [
			Vec3::new(0.0, 0.0, 0.0),
			Vec3::new(5.0, -3.0, 0.0),
			Vec3::new(-8.0, 4.5, 0.0),
			Vec3::new(10.0, 7.0, 0.0),
		];

		for aspect in [0.5, 1.0, 16.0 / 9.0, 21.0 / 9.0, 3.0] {
			let half_height = 10.0;
			let half_width = half_height * aspect;
			let view_projection = Mat4::orthographic_rh(-half_width, half_width, -half_height, half_height, -1.0, 1.0);
			let viewport_size = Vec2::new(1920.0, 1920.0 / aspect);

			let camera = ActiveCameraView {
				view_projection,
				viewport_size,
				viewport_offset: Vec2::ZERO,
			};

			for &world in &world_points {
				let screen = camera.project_to_screen(world).expect("project");
				let round_tripped = camera.unproject_to_world(screen).expect("unproject");
				assert!(
					(round_tripped.x - world.x).abs() < 1e-3,
					"aspect={aspect}: x={} vs {}",
					round_tripped.x,
					world.x,
				);
				assert!(
					(round_tripped.y - world.y).abs() < 1e-3,
					"aspect={aspect}: y={} vs {}",
					round_tripped.y,
					world.y,
				);
			}
		}
	}

	#[test]
	fn fit_aspect_pillarboxes_a_wider_than_target_viewport() {
		// Viewport is 16:9 but the reference/target is 4:3 -- bars go on the
		// left/right, full height is used, and the box is horizontally centered.
		let (offset, size) = fit_aspect(Vec2::new(1920.0, 1080.0), 4.0 / 3.0);
		assert!((size.y - 1080.0).abs() < 1e-3, "full height should be used: {size:?}");
		assert!(
			(size.x - 1440.0).abs() < 1e-3,
			"width should shrink to match target aspect: {size:?}"
		);
		assert!((offset.y).abs() < 1e-3, "no vertical bars: {offset:?}");
		assert!(
			(offset.x - 240.0).abs() < 1e-3,
			"should be centered horizontally: {offset:?}"
		);
	}

	#[test]
	fn fit_aspect_letterboxes_a_taller_than_target_viewport() {
		// Viewport is 4:3 but the reference/target is 16:9 -- bars go on the
		// top/bottom, full width is used, and the box is vertically centered.
		let (offset, size) = fit_aspect(Vec2::new(1200.0, 900.0), 16.0 / 9.0);
		assert!((size.x - 1200.0).abs() < 1e-3, "full width should be used: {size:?}");
		assert!(
			(size.y - 675.0).abs() < 1e-3,
			"height should shrink to match target aspect: {size:?}"
		);
		assert!((offset.x).abs() < 1e-3, "no horizontal bars: {offset:?}");
		assert!(
			(offset.y - 112.5).abs() < 1e-3,
			"should be centered vertically: {offset:?}"
		);
	}

	#[test]
	fn fit_aspect_matching_ratio_has_no_bars() {
		let (offset, size) = fit_aspect(Vec2::new(1920.0, 1080.0), 16.0 / 9.0);
		assert_eq!(offset, Vec2::ZERO);
		assert_eq!(size, Vec2::new(1920.0, 1080.0));
	}

	#[test]
	fn unproject_accounts_for_letterbox_offset() {
		// A 4:3 world rendered pillarboxed inside a 16:9 viewport: content
		// occupies the centered 1440x1080 sub-rect, bars fill the rest.
		let half_height = 10.0;
		let half_width = half_height * 4.0 / 3.0;
		let view_projection = Mat4::orthographic_rh(-half_width, half_width, -half_height, half_height, -1.0, 1.0);
		let camera = ActiveCameraView {
			view_projection,
			viewport_size: Vec2::new(1440.0, 1080.0),
			viewport_offset: Vec2::new(240.0, 0.0),
		};

		let world = Vec3::new(3.0, -2.0, 0.0);
		let screen = camera.project_to_screen(world).expect("project");
		assert!(
			screen.x >= 240.0 && screen.x <= 1680.0,
			"screen x should land inside the pillarboxed rect: {screen:?}"
		);

		let round_tripped = camera.unproject_to_world(screen).expect("unproject");
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
}
