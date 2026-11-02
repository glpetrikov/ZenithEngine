//! Mouse buttons, gamepad buttons and gamepad axes.

use std::{fmt, str::FromStr};

use serde::{Deserialize, Deserializer, Serialize, Serializer};
use zenith_error::ZenithError;

use crate::names::{normalize, string_enum};

/// A mouse button.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum ZMouseButton {
	Left,
	Right,
	Middle,
	Back,
	Forward,
	/// Any other button, by the number the system gives it.
	Other(u16),
}

impl fmt::Display for ZMouseButton {
	fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
		match self {
			Self::Left => f.write_str("Left"),
			Self::Right => f.write_str("Right"),
			Self::Middle => f.write_str("Middle"),
			Self::Back => f.write_str("Back"),
			Self::Forward => f.write_str("Forward"),
			Self::Other(number) => write!(f, "Other{number}"),
		}
	}
}

impl FromStr for ZMouseButton {
	type Err = ZenithError;

	/// `Left`, `Right`, `Middle`, `Back`, `Forward` or `Other<number>`, in any
	/// case.
	fn from_str(text: &str) -> Result<Self, Self::Err> {
		let wanted = normalize(text);
		match wanted.as_str() {
			"left" => Ok(Self::Left),
			"right" => Ok(Self::Right),
			"middle" => Ok(Self::Middle),
			"back" => Ok(Self::Back),
			"forward" => Ok(Self::Forward),
			other => other
				.strip_prefix("other")
				.and_then(|number| number.parse().ok())
				.map(Self::Other)
				.ok_or_else(|| ZenithError::InvalidSettings(format!("unknown mouse button {text:?}"))),
		}
	}
}

impl Serialize for ZMouseButton {
	fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
		serializer.collect_str(self)
	}
}

impl<'de> Deserialize<'de> for ZMouseButton {
	fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
		let text = String::deserialize(deserializer)?;
		text.parse().map_err(serde::de::Error::custom)
	}
}

string_enum! {
	/// A gamepad button.
	///
	/// Named by place, not by symbol, so that it means the same on every pad:
	/// `South` is the bottom face button (A on Xbox, cross on `PlayStation`),
	/// `East` the right one, `West` the left, `North` the top.
	pub enum ZGamePad {
		South,
		East,
		West,
		North,
		LeftBumper,
		RightBumper,
		/// The trigger as a button (it counts as pressed when it is pulled far
		/// enough); the amount is the axis of the same name.
		LeftTrigger,
		RightTrigger,
		Select,
		Start,
		/// The big middle button (Guide, Home, PS).
		Mode,
		/// Pressing the left stick.
		LeftStick,
		RightStick,
		DPadUp,
		DPadDown,
		DPadLeft,
		DPadRight,
	}
}

string_enum! {
	/// A gamepad axis. Sticks go from `-1.0` (left, down) to `1.0` (right,
	/// up); triggers from `0.0` to `1.0`.
	pub enum ZGamePadAxis {
		LeftStickX,
		LeftStickY,
		RightStickX,
		RightStickY,
		LeftTrigger,
		RightTrigger,
	}
}

impl ZGamePadAxis {
	/// The other axis of the same stick; `None` for triggers.
	#[must_use]
	pub const fn stick_pair(self) -> Option<Self> {
		match self {
			Self::LeftStickX => Some(Self::LeftStickY),
			Self::LeftStickY => Some(Self::LeftStickX),
			Self::RightStickX => Some(Self::RightStickY),
			Self::RightStickY => Some(Self::RightStickX),
			Self::LeftTrigger | Self::RightTrigger => None,
		}
	}
}

string_enum! {
	/// Which of the common families a gamepad belongs to, judged by its maker.
	/// Meant for choosing the pictures of the buttons in a game ("press
	/// `Cross`" or "press `A`").
	pub enum ZGamepadKind {
		Xbox,
		PlayStation,
		Nintendo,
		/// Anything else.
		Generic,
	}
}

impl ZGamepadKind {
	/// The family for a USB vendor id (`None` when the system does not say).
	#[must_use]
	pub const fn from_vendor(vendor_id: Option<u16>) -> Self {
		match vendor_id {
			Some(0x045e) => Self::Xbox,
			Some(0x054c) => Self::PlayStation,
			Some(0x057e) => Self::Nintendo,
			_ => Self::Generic,
		}
	}
}

string_enum! {
	/// One of the two sticks of a gamepad.
	pub enum ZStick {
		Left,
		Right,
	}
}

impl ZStick {
	/// The two axes of the stick, `x` then `y`.
	#[must_use]
	pub const fn axes(self) -> (ZGamePadAxis, ZGamePadAxis) {
		match self {
			Self::Left => (ZGamePadAxis::LeftStickX, ZGamePadAxis::LeftStickY),
			Self::Right => (ZGamePadAxis::RightStickX, ZGamePadAxis::RightStickY),
		}
	}
}

/// The power supply of a gamepad.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum ZPowerInfo {
	/// The system does not say (always the case in a browser).
	Unknown,
	/// No battery: the pad is on a cable.
	Wired,
	/// On the battery; the charge is `0..=100` percent.
	Discharging(u8),
	/// The battery is being charged; the charge is `0..=100` percent.
	Charging(u8),
	/// The battery is full.
	Charged,
}

/// A connected gamepad.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct GamepadInfo {
	/// The number to give to `input.gamepad(id)` and the like. Numbers are
	/// reused: the first free one goes to the next pad that is connected.
	pub id: usize,
	pub name: String,
	pub kind: ZGamepadKind,
}

#[cfg(test)]
mod tests {
	use super::*;

	#[test]
	fn mouse_buttons_read_back() {
		for button in [
			ZMouseButton::Left,
			ZMouseButton::Right,
			ZMouseButton::Middle,
			ZMouseButton::Back,
			ZMouseButton::Forward,
			ZMouseButton::Other(7),
		] {
			assert_eq!(
				button.to_string().parse::<ZMouseButton>().ok(),
				Some(button),
				"{button}"
			);
		}
		assert_eq!(
			"left".parse::<ZMouseButton>().ok(),
			Some(ZMouseButton::Left),
			"any case"
		);
		assert!("other".parse::<ZMouseButton>().is_err(), "a number is needed");
		assert!("thumb".parse::<ZMouseButton>().is_err(), "unknown");
	}

	#[test]
	fn gamepad_names_read_back() {
		for button in ZGamePad::ALL {
			assert_eq!(button.name().parse::<ZGamePad>().ok(), Some(*button), "{button}");
		}
		for axis in ZGamePadAxis::ALL {
			assert_eq!(axis.name().parse::<ZGamePadAxis>().ok(), Some(*axis), "{axis}");
		}
		assert_eq!(
			"d_pad_up".parse::<ZGamePad>().ok(),
			Some(ZGamePad::DPadUp),
			"separators are ignored"
		);
	}

	#[test]
	fn makers_tell_the_family() {
		assert_eq!(ZGamepadKind::from_vendor(Some(0x045e)), ZGamepadKind::Xbox, "Microsoft");
		assert_eq!(
			ZGamepadKind::from_vendor(Some(0x054c)),
			ZGamepadKind::PlayStation,
			"Sony"
		);
		assert_eq!(
			ZGamepadKind::from_vendor(Some(0x057e)),
			ZGamepadKind::Nintendo,
			"Nintendo"
		);
		assert_eq!(
			ZGamepadKind::from_vendor(Some(0x1234)),
			ZGamepadKind::Generic,
			"unknown maker"
		);
		assert_eq!(ZGamepadKind::from_vendor(None), ZGamepadKind::Generic, "no maker");
		assert_eq!(
			ZStick::Right.axes(),
			(ZGamePadAxis::RightStickX, ZGamePadAxis::RightStickY),
			"stick axes"
		);
	}

	#[test]
	fn sticks_come_in_pairs() {
		assert_eq!(
			ZGamePadAxis::LeftStickX.stick_pair(),
			Some(ZGamePadAxis::LeftStickY),
			"x goes with y"
		);
		assert_eq!(ZGamePadAxis::LeftTrigger.stick_pair(), None, "a trigger is alone");
	}
}
