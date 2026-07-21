use ze_assets::{AssetRef, TextureSource};
use ze_ecs::{Deserialize, JsonSchema, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(default, crate = "ze_ecs::serde")]
#[schemars(crate = "ze_ecs::schemars")]
pub struct UIRect {
	/// Pixel offset from the anchor. In `UIAnchorMode::WorldSpace`, the anchor
	/// is the owning entity's world position projected to screen space
	/// through the camera actually rendering the scene (when the entity has
	/// a `Transform` and the scene has an active camera); otherwise it's the
	/// screen origin (top-left). In `UIAnchorMode::ScreenSpaceOverlay`, `x`/`y`
	/// are always absolute screen pixel coordinates, with no camera involved.
	pub x: f32,
	pub y: f32,
	pub width: f32,
	pub height: f32,
}

impl Default for UIRect {
	fn default() -> Self {
		Self {
			x: 0.0,
			y: 0.0,
			width: 0.0,
			height: 0.0,
		}
	}
}

/// How a UI element's screen position is derived from `UIRect::x`/`y`.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, JsonSchema, PartialEq, Eq, Default)]
#[serde(crate = "ze_ecs::serde")]
#[schemars(crate = "ze_ecs::schemars")]
pub enum UIAnchorMode {
	/// The element has a world position (via `Transform`) that gets
	/// projected through whichever camera is currently rendering the scene
	/// every frame -- the Editor Camera while navigating in Edit mode, the
	/// gameplay camera in Play mode. Correct for UI that's conceptually
	/// attached to something in the world (nameplates, markers, prompts).
	#[default]
	WorldSpace,
	/// The element's position is `UIRect::x`/`y` interpreted directly as
	/// absolute screen pixel coordinates, bypassing camera projection
	/// entirely. Stays fixed on screen regardless of camera movement --
	/// correct for HUD-style UI (health bars, menus, score counters).
	ScreenSpaceOverlay,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(default, crate = "ze_ecs::serde")]
#[schemars(crate = "ze_ecs::schemars")]
pub struct UIButton {
	pub rect: UIRect,
	pub anchor_mode: UIAnchorMode,
	pub text: String,
	pub font_size: f32,
	pub color: [f32; 4],
	pub hover_color: [f32; 4],
	pub pressed_color: [f32; 4],
	/// Transient interaction state written every frame by the UI system as
	/// the mouse moves; excluded from serialization so it never taints the
	/// editor's save-dirty fingerprint or gets persisted to scene files.
	#[serde(skip)]
	pub pressed: bool,
	#[serde(skip)]
	pub hovered: bool,
	/// Paint order among all UI elements (buttons, bars, texts). Higher
	/// values paint later, on top of lower ones. Ties break by entity id.
	pub z_index: i32,
}

impl Default for UIButton {
	fn default() -> Self {
		Self {
			rect: UIRect::default(),
			anchor_mode: UIAnchorMode::default(),
			text: String::new(),
			font_size: 14.0,
			color: [0.2, 0.2, 0.8, 1.0],
			hover_color: [0.3, 0.3, 0.9, 1.0],
			pressed_color: [0.1, 0.1, 0.5, 1.0],
			pressed: false,
			hovered: false,
			z_index: 0,
		}
	}
}

impl ze_ecs::Component for UIButton {
	type Tracking = ze_ecs::track::Untracked;
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(default, crate = "ze_ecs::serde")]
#[schemars(crate = "ze_ecs::schemars")]
pub struct UIBar {
	pub rect: UIRect,
	pub anchor_mode: UIAnchorMode,
	pub current: f32,
	pub max: f32,
	pub color: [f32; 4],
	pub bg_color: [f32; 4],
	/// Optional label rendered centered on top of the bar, e.g. "42/100".
	pub text: Option<String>,
	/// Paint order among all UI elements (buttons, bars, texts). Higher
	/// values paint later, on top of lower ones. Ties break by entity id.
	pub z_index: i32,
}

impl Default for UIBar {
	fn default() -> Self {
		Self {
			rect: UIRect::default(),
			anchor_mode: UIAnchorMode::default(),
			current: 0.0,
			max: 1.0,
			color: [0.2, 0.8, 0.2, 1.0],
			bg_color: [0.2, 0.2, 0.2, 1.0],
			text: None,
			z_index: 0,
		}
	}
}

impl ze_ecs::Component for UIBar {
	type Tracking = ze_ecs::track::Untracked;
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(default, crate = "ze_ecs::serde")]
#[schemars(crate = "ze_ecs::schemars")]
pub struct UIText {
	pub rect: UIRect,
	pub anchor_mode: UIAnchorMode,
	pub text: String,
	pub font_size: f32,
	pub color: [f32; 4],
	/// Paint order among all UI elements (buttons, bars, texts). Higher
	/// values paint later, on top of lower ones. Ties break by entity id.
	pub z_index: i32,
}

impl Default for UIText {
	fn default() -> Self {
		Self {
			rect: UIRect::default(),
			anchor_mode: UIAnchorMode::default(),
			text: String::new(),
			font_size: 14.0,
			color: [1.0, 1.0, 1.0, 1.0],
			z_index: 0,
		}
	}
}

impl ze_ecs::Component for UIText {
	type Tracking = ze_ecs::track::Untracked;
}

/// A static image rendered in the UI layer, purely for display -- no
/// click/hover interaction. `texture` reuses the exact same
/// `Sprite`-style texture-reference format (`ze_assets::TextureSource`) so a
/// `UIImage` can point at either a whole file or a specific `TextureSheet`
/// cell, with no separate texture-loading path of its own.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(default, crate = "ze_ecs::serde")]
#[schemars(crate = "ze_ecs::schemars")]
pub struct UIImage {
	pub rect: UIRect,
	pub anchor_mode: UIAnchorMode,
	pub texture: TextureSource,
	/// Multiplied with the sampled texture color (tint). `[1,1,1,1]` (default)
	/// leaves the source image unmodified.
	pub color: [f32; 4],
	/// Paint order among all UI elements (buttons, bars, texts, images).
	/// Higher values paint later, on top of lower ones. Ties break by entity
	/// id.
	pub z_index: i32,
}

impl Default for UIImage {
	fn default() -> Self {
		Self {
			rect: UIRect::default(),
			anchor_mode: UIAnchorMode::default(),
			texture: TextureSource::File(AssetRef::game(String::new())),
			color: [1.0, 1.0, 1.0, 1.0],
			z_index: 0,
		}
	}
}

impl ze_ecs::Component for UIImage {
	type Tracking = ze_ecs::track::Untracked;
}
