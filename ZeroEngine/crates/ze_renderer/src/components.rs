use ze_assets::AssetRef;
use ze_ecs::{Deserialize, JsonSchema, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(crate = "ze_ecs::serde")]
#[schemars(crate = "ze_ecs::schemars")]
pub struct Sprite {
	pub texture: AssetRef,
	pub size: SpriteSize,
	pub color: SpriteColorSettings,
	pub settings: SpriteSettings,
	/// Opt-in emissive/glow strength fed into the `GBuffer`'s emissive
	/// attachment (and from there, bloom). `0.0` (default) means "doesn't
	/// glow"; values above `1.0` are expected for strongly emissive sprites
	/// (engine trails, lasers, explosions) -- the emissive attachment is HDR
	/// (`Rgba16Float`) specifically to have headroom for that.
	#[serde(default)]
	pub glow_strength: f32,
}

impl ze_ecs::Component for Sprite {
	type Tracking = ze_ecs::track::Untracked;
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, JsonSchema)]
#[serde(crate = "ze_ecs::serde")]
#[schemars(crate = "ze_ecs::schemars")]
pub enum SpriteSize {
	#[default]
	Auto,
	Custom {
		width: f32,
		height: f32,
	},
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(crate = "ze_ecs::serde")]
#[schemars(crate = "ze_ecs::schemars")]
pub struct SpriteSettings {
	pub visible: bool,
	pub flip_x: bool,
	pub flip_y: bool,
	pub layer: i32,
	#[serde(default)]
	pub texture_rotation_degrees: f32,
}

impl Default for SpriteSettings {
	fn default() -> Self {
		Self {
			visible: true,
			flip_x: false,
			flip_y: false,
			layer: 0,
			texture_rotation_degrees: 0.0,
		}
	}
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(crate = "ze_ecs::serde")]
#[schemars(crate = "ze_ecs::schemars")]
pub struct SpriteColorSettings {
	#[serde(default, deserialize_with = "ze_core::deserialize_optional_rgba")]
	pub tint: Option<[f32; 4]>,
	pub mode: SpriteColorMode,
	pub strength: f32,
	pub saturation_threshold: f32,
}

impl Default for SpriteColorSettings {
	fn default() -> Self {
		Self {
			tint: None,
			mode: SpriteColorMode::None,
			strength: 1.0,
			saturation_threshold: 0.15,
		}
	}
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, JsonSchema, PartialEq, Eq)]
#[serde(crate = "ze_ecs::serde")]
#[schemars(crate = "ze_ecs::schemars")]
pub enum SpriteColorMode {
	None,
	Multiply,
	GrayscaleTint,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(crate = "ze_ecs::serde")]
#[schemars(crate = "ze_ecs::schemars")]
pub struct Camera {
	pub projection: CameraProjection,
	pub primary: bool,
	#[serde(deserialize_with = "ze_core::deserialize_rgba")]
	pub clear_color: [f32; 4],
}

impl ze_ecs::Component for Camera {
	type Tracking = ze_ecs::track::Untracked;
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(crate = "ze_ecs::serde")]
#[schemars(crate = "ze_ecs::schemars")]
pub enum CameraProjection {
	Orthographic { size: f32, near: f32, far: f32 },
	Perspective { fov_y_radians: f32, near: f32, far: f32 },
}

pub fn register_renderer_components(registry: &mut ze_ecs::ComponentRegistry) {
	registry.register::<Sprite>("ze.renderer.sprite");
	registry.register::<Camera>("ze.renderer.camera");
}
