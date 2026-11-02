//! What happens on an input device, as plain values.

use glam::Vec2;
use serde::{Deserialize, Serialize};

use crate::{ZGamePad, ZGamePadAxis, ZGamepadKind, ZKeyCode, ZMouseButton, ZPowerInfo};

/// One thing that happened. Everything that feeds [`Input`](crate::Input)
/// (the window, a gamepad library, a test, a recording, the emulated gamepad of
/// the editor) produces these and nothing else.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub enum InputEvent {
	KeyDown(ZKeyCode),
	KeyUp(ZKeyCode),
	MouseDown(ZMouseButton),
	MouseUp(ZMouseButton),
	/// The cursor is at this position in the window (physical pixels).
	MouseMoved(Vec2),
	/// The cursor left the window.
	MouseLeft,
	/// The mouse moved by this much, whatever the cursor does (it is the one to
	/// use while the cursor is grabbed).
	MouseMotion(Vec2),
	/// The wheel turned, in lines; `y` is positive away from the user.
	Scroll(Vec2),
	/// Characters that were typed.
	Text(String),
	/// The window got or lost the focus. Losing it lets go of every key and
	/// mouse button, since their release will not be seen.
	Focus(bool),
	/// A gamepad is there. `ff_supported` says whether it can vibrate and
	/// `mapped` whether its buttons follow the standard layout (an unmapped
	/// pad reports buttons nobody can name).
	GamepadConnected {
		id: usize,
		name: String,
		kind: ZGamepadKind,
		ff_supported: bool,
		mapped: bool,
	},
	GamepadDisconnected {
		id: usize,
	},
	GamepadButton {
		id: usize,
		button: ZGamePad,
		pressed: bool,
	},
	GamepadAxis {
		id: usize,
		axis: ZGamePadAxis,
		value: f32,
	},
	/// The battery of a gamepad changed.
	GamepadPower {
		id: usize,
		power: ZPowerInfo,
	},
}
