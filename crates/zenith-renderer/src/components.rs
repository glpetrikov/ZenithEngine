//! The components of the renderer: [`Model`], [`Sprite`], [`Camera`], the
//! lights and [`Environment`].
//!
//! They are saved like the other components (`PascalCase`), and a host
//! registers them with [`register_components`]. Every one of them goes with a
//! `Transform` (a `Sprite`, a `Model`, a light, a camera) and an entity
//! without an `IsActive` counts as active.

use bevy_ecs::component::Component;
use serde::{Deserialize, Serialize};
use zenith_registry::ComponentRegistry;
use zenith_types::Version;

use crate::{assets::AssetId, graph::GraphId};

/// A mesh drawn with a material. A limit of the first version: one material
/// for the whole mesh.
#[derive(Component, Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "PascalCase")]
pub struct Model {
	pub mesh: AssetId,
	pub material: AssetId,
}

/// A flat picture drawn by a 2D (orthographic) camera.
#[derive(Component, Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "PascalCase")]
pub struct Sprite {
	pub texture: AssetId,
	/// The size in world units.
	pub size: [f32; 2],
	/// Linear RGBA, multiplied with the texture.
	pub color: [f32; 4],
	/// A part of the texture, in 0..1 (x, y, width, height); `None` is all of
	/// it.
	pub uv_rect: Option<[f32; 4]>,
	/// Sprites with a higher order are drawn over the ones with a lower.
	pub order: i32,
}

/// How a camera sees. A 2D camera is an orthographic one.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "PascalCase")]
pub enum Projection {
	Perspective {
		/// Vertical field of view, in radians.
		fov_y: f32,
		near: f32,
		far: f32,
	},
	Orthographic {
		/// The height of what is seen, in world units; the width follows the
		/// shape of the viewport.
		height: f32,
		near: f32,
		far: f32,
	},
}

impl Default for Projection {
	fn default() -> Self {
		Self::Perspective {
			fov_y: 60.0_f32.to_radians(),
			near: 0.1,
			far: 1000.0,
		}
	}
}

impl Projection {
	/// View space to clip space for a viewport of `size` pixels (depth from 0
	/// to 1, the way wgpu wants it; the camera looks along -Z).
	pub fn matrix(&self, size: (u32, u32)) -> glam::Mat4 {
		use glam::camera::rh::proj::directx;

		let aspect = size.0.max(1) as f32 / size.1.max(1) as f32;
		match *self {
			Self::Perspective { fov_y, near, far } => directx::perspective(fov_y, aspect, near, far),
			Self::Orthographic { height, near, far } => {
				let half_height = height * 0.5;
				let half_width = half_height * aspect;
				directx::orthographic(-half_width, half_width, -half_height, half_height, near, far)
			}
		}
	}

	pub const fn near(&self) -> f32 {
		match *self {
			Self::Perspective { near, .. } | Self::Orthographic { near, .. } => near,
		}
	}

	pub const fn far(&self) -> f32 {
		match *self {
			Self::Perspective { far, .. } | Self::Orthographic { far, .. } => far,
		}
	}
}

/// What a camera draws into.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "PascalCase")]
pub enum CameraTarget {
	/// The main output: the window (in the editor, the Game panel).
	#[default]
	Window,
	/// A render texture asset. Not drawn yet: it comes with the render
	/// targets (stage 7 of the plan).
	Texture(AssetId),
}

/// What a camera does with its target before it draws.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "PascalCase")]
pub enum ClearMode {
	/// The first camera on a target clears it to this color (linear RGBA).
	Color([f32; 4]),
	/// It draws over what the camera before it drew.
	None,
}

impl Default for ClearMode {
	fn default() -> Self {
		Self::Color([0.1, 0.1, 0.1, 1.0])
	}
}

/// A camera: a view of the world, drawn by a graph into a target.
#[derive(Component, Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "PascalCase", default)]
pub struct Camera {
	pub projection: Projection,
	pub target: CameraTarget,
	/// A part of the target, in 0..1 (x, y, width, height); `None` is all of
	/// it.
	pub viewport: Option<[f32; 4]>,
	/// The order of the cameras on one target: the lowest draws first.
	pub order: i32,
	pub clear: ClearMode,
	/// `None`: the main graph.
	pub graph: Option<GraphId>,
}

/// A light far away that shines along the -Z of its entity.
#[derive(Component, Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "PascalCase")]
pub struct DirectionalLight {
	pub color: [f32; 3],
	pub illuminance: f32,
	pub shadows: bool,
}

impl Default for DirectionalLight {
	fn default() -> Self {
		Self {
			color: [1.0; 3],
			illuminance: 3.0,
			shadows: false,
		}
	}
}

/// A light that shines everywhere from its place.
#[derive(Component, Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "PascalCase")]
pub struct PointLight {
	pub color: [f32; 3],
	pub intensity: f32,
	/// Nothing farther than this is lit.
	pub range: f32,
}

impl Default for PointLight {
	fn default() -> Self {
		Self {
			color: [1.0; 3],
			intensity: 10.0,
			range: 20.0,
		}
	}
}

/// The sky of a camera: seen behind the scene and mirrored in it. Goes on the
/// entity of the camera.
#[derive(Component, Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "PascalCase")]
pub struct Environment {
	pub sky: AssetId,
	pub intensity: f32,
}

/// Registers the components of the renderer, so worlds can save and load
/// them.
pub fn register_components(registry: &mut ComponentRegistry) {
	let version = Version::new(1, 0, 0);
	registry.register::<Model>("zenith_engine.model", version.clone());
	registry.register::<Sprite>("zenith_engine.sprite", version.clone());
	registry.register::<Camera>("zenith_engine.camera", version.clone());
	registry.register::<DirectionalLight>("zenith_engine.directional_light", version.clone());
	registry.register::<PointLight>("zenith_engine.point_light", version.clone());
	registry.register::<Environment>("zenith_engine.environment", version);
}

#[cfg(test)]
mod tests {
	use super::*;

	#[test]
	fn a_camera_with_only_some_fields_gets_the_rest_by_default() -> Result<(), serde_json::Error> {
		let camera: Camera = serde_json::from_str(r#"{ "Order": 3 }"#)?;
		assert_eq!(camera.order, 3);
		assert_eq!(camera.projection, Projection::default());
		assert_eq!(camera.target, CameraTarget::Window);
		Ok(())
	}

	#[test]
	fn projections_look_along_minus_z_into_zero_to_one_depth() {
		let ortho = Projection::Orthographic {
			height: 2.0,
			near: 1.0,
			far: 3.0,
		};
		let matrix = ortho.matrix((200, 100));
		let near = matrix.project_point3(glam::Vec3::new(0.0, 0.0, -1.0));
		let far = matrix.project_point3(glam::Vec3::new(0.0, 0.0, -3.0));
		assert!(near.z.abs() < 1e-5, "{near}");
		assert!((far.z - 1.0).abs() < 1e-5, "{far}");
		// The width follows the aspect: x = 2 is the edge of a 200 x 100 view.
		let edge = matrix.project_point3(glam::Vec3::new(2.0, 0.0, -2.0));
		assert!((edge.x - 1.0).abs() < 1e-5, "{edge}");
	}
}
