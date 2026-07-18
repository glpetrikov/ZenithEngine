use ze_assets::{AssetRef, CellId};
use ze_ecs::{Deserialize, JsonSchema, Serialize};

/// Where a `Sprite`'s pixels come from: either a plain file (unchanged,
/// existing behavior) or a specific cell of a `TextureSheet` asset.
///
/// `#[serde(untagged)]` with disjoint field sets (`path`/`source` for
/// `File` vs `sheet_path`/`cell_id` for `SheetCell`) is what makes this
/// backward compatible: an existing scene's plain `{"path":...,"source":
/// "Game"}` texture value still deserializes as `File` with zero migration
/// code, since serde tries each variant's shape in turn.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(crate = "ze_ecs::serde", untagged)]
#[schemars(crate = "ze_ecs::schemars")]
pub enum TextureSource {
	File(AssetRef),
	SheetCell { sheet_path: String, cell_id: CellId },
}

impl From<AssetRef> for TextureSource {
	fn from(asset: AssetRef) -> Self { Self::File(asset) }
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(crate = "ze_ecs::serde")]
#[schemars(crate = "ze_ecs::schemars")]
pub struct Sprite {
	pub texture: TextureSource,
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
