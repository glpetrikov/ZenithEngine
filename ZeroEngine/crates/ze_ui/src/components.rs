use ze_ecs::{Deserialize, JsonSchema, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(default, crate = "ze_ecs::serde")]
#[schemars(crate = "ze_ecs::schemars")]
pub struct UIRect {
	/// Pixel offset from the anchor. The anchor is the owning entity's world
	/// position projected to screen space when the entity has a `Transform`
	/// and the scene has a primary camera; otherwise it's the screen origin
	/// (top-left), making `x`/`y` behave as absolute screen coordinates.
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

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(default, crate = "ze_ecs::serde")]
#[schemars(crate = "ze_ecs::schemars")]
pub struct UIButton {
	pub rect: UIRect,
	pub text: String,
	pub font_size: f32,
	pub color: [f32; 4],
	pub hover_color: [f32; 4],
	pub pressed_color: [f32; 4],
	pub pressed: bool,
	pub hovered: bool,
	/// Paint order among all UI elements (buttons, bars, texts). Higher
	/// values paint later, on top of lower ones. Ties break by entity id.
	pub z_index: i32,
}

impl Default for UIButton {
	fn default() -> Self {
		Self {
			rect: UIRect::default(),
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
