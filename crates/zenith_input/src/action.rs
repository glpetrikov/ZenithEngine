//! Actions: names the game asks for ("Jump") that are bound to inputs.

use std::{fmt, str::FromStr};

use zenith_error::ZenithError;

use crate::{ZGamePad, ZGamePadAxis, ZKeyCode, ZMouseButton};

/// Where the value of a binding comes from.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Source {
	Key(ZKeyCode),
	Mouse(ZMouseButton),
	/// A button of any connected gamepad.
	GamepadButton(ZGamePad),
	/// An axis of any connected gamepad (the one that is pushed furthest).
	GamepadAxis(ZGamePadAxis),
}

impl From<ZKeyCode> for Source {
	fn from(key: ZKeyCode) -> Self {
		Self::Key(key)
	}
}

impl From<ZMouseButton> for Source {
	fn from(button: ZMouseButton) -> Self {
		Self::Mouse(button)
	}
}

impl From<ZGamePad> for Source {
	fn from(button: ZGamePad) -> Self {
		Self::GamepadButton(button)
	}
}

impl From<ZGamePadAxis> for Source {
	fn from(axis: ZGamePadAxis) -> Self {
		Self::GamepadAxis(axis)
	}
}

impl fmt::Display for Source {
	fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
		match self {
			Self::Key(key) => write!(f, "Key:{key}"),
			Self::Mouse(button) => write!(f, "Mouse:{button}"),
			Self::GamepadButton(button) => write!(f, "Gamepad:{button}"),
			Self::GamepadAxis(axis) => write!(f, "Axis:{axis}"),
		}
	}
}

impl FromStr for Source {
	type Err = ZenithError;

	/// `Key:Space`, `Mouse:Left`, `Gamepad:South` or `Axis:LeftStickX`.
	fn from_str(text: &str) -> Result<Self, Self::Err> {
		let (kind, name) = text.split_once(':').ok_or_else(|| {
			ZenithError::InvalidSettings(format!("{text:?}: expected `Kind:Name`, for example `Key:Space`"))
		})?;
		match kind.trim().to_ascii_lowercase().as_str() {
			"key" => Ok(Self::Key(name.trim().parse()?)),
			"mouse" => Ok(Self::Mouse(name.trim().parse()?)),
			"gamepad" | "pad" | "button" => Ok(Self::GamepadButton(name.trim().parse()?)),
			"axis" => Ok(Self::GamepadAxis(name.trim().parse()?)),
			_ => Err(ZenithError::InvalidSettings(format!(
				"{text:?}: the kind must be Key, Mouse, Gamepad or Axis"
			))),
		}
	}
}

/// An input bound to an action, with a factor.
///
/// Buttons give `1.0` while they are down, axes their value; the factor then
/// multiplies it. A factor of `-1.0` makes the "left" half of an axis from a
/// key: `D` and a negated `A` together are one `-1..=1` value.
///
/// A binding can also need other inputs held at the same time, a chord:
/// `Binding::from(ZKeyCode::S).with(ZKeyCode::LeftControl)` is `Ctrl+S`.
#[derive(Clone, Debug, PartialEq)]
pub struct Binding {
	pub source: Source,
	pub scale: f32,
	/// Inputs that must be held (or pushed past the press threshold) for the
	/// binding to count at all.
	pub with: Vec<Source>,
	/// For a gamepad button or axis: only this gamepad counts (`None`: any
	/// of them). Two players on two pads have two bindings, each on its own.
	pub pad: Option<usize>,
}

impl Binding {
	#[must_use]
	pub const fn new(source: Source) -> Self {
		Self {
			source,
			scale: 1.0,
			with: Vec::new(),
			pad: None,
		}
	}

	/// The same binding, reading only gamepad number `pad` (for a button or an
	/// axis of a gamepad; it changes nothing for a key or a mouse button).
	#[must_use]
	pub fn on_pad(self, pad: usize) -> Self {
		Self { pad: Some(pad), ..self }
	}

	/// The same binding with the sign of its value flipped.
	#[must_use]
	pub fn negated(self) -> Self {
		Self {
			scale: -self.scale,
			..self
		}
	}

	/// The same binding with its value multiplied by `factor`.
	#[must_use]
	pub fn scaled(self, factor: f32) -> Self {
		Self {
			scale: self.scale * factor,
			..self
		}
	}

	/// The same binding, counting only while `modifier` is held too.
	#[must_use]
	pub fn with(mut self, modifier: impl Into<Source>) -> Self {
		self.with.push(modifier.into());
		self
	}
}

impl<T: Into<Source>> From<T> for Binding {
	fn from(source: T) -> Self {
		Self::new(source.into())
	}
}

impl fmt::Display for Binding {
	/// `Key:Space`, `-Key:A`, `Key:LeftControl+Key:S`, `Axis:LeftStickX`: what
	/// `Input.toml` uses. A factor other than `1` and `-1` has no text form and
	/// is not shown.
	fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
		if self.scale < 0.0 {
			f.write_str("-")?;
		}
		for modifier in &self.with {
			write!(f, "{modifier}+")?;
		}
		write!(f, "{}", self.source)?;
		if let Some(pad) = self.pad {
			write!(f, "@{pad}")?;
		}
		Ok(())
	}
}

impl FromStr for Binding {
	type Err = ZenithError;

	/// `Key:Space`, `Mouse:Left`, `Gamepad:South`, `Axis:LeftStickX`; a `-` in
	/// front negates the value (`-Key:A`), and inputs joined with `+` are a
	/// chord (`Key:LeftControl+Key:S`).
	fn from_str(text: &str) -> Result<Self, Self::Err> {
		let bad = |why: &str| ZenithError::InvalidSettings(format!("binding {text:?}: {why}"));
		let trimmed = text.trim();
		let (negative, rest) = trimmed
			.strip_prefix('-')
			.map_or((false, trimmed), |rest| (true, rest.trim_start()));
		let mut parts: Vec<&str> = rest.split('+').map(str::trim).collect();
		// `@n` after the last input: the gamepad it is read from.
		let last = parts.pop().ok_or_else(|| bad("empty"))?;
		let (last, pad) = match last.split_once('@') {
			Some((source, number)) => {
				let pad = number
					.trim()
					.parse::<usize>()
					.map_err(|_| bad("the number after `@` is not a gamepad number"))?;
				(source.trim(), Some(pad))
			}
			None => (last, None),
		};
		let source: Source = last.parse()?;
		if pad.is_some() && !matches!(source, Source::GamepadButton(_) | Source::GamepadAxis(_)) {
			return Err(bad("`@n` is for gamepad buttons and axes"));
		}
		let with = parts
			.into_iter()
			.map(str::parse::<Source>)
			.collect::<Result<Vec<_>, _>>()?;
		Ok(Self {
			source,
			scale: if negative { -1.0 } else { 1.0 },
			with,
			pad,
		})
	}
}

/// An action of two values, such as walking: the two axes are sums of bindings
/// like those of a button action, so `x` can be `D`, a negated `A` and a stick.
#[derive(Clone, Debug, PartialEq)]
pub struct VectorAction {
	pub x: Vec<Binding>,
	pub y: Vec<Binding>,
	/// The vector is never longer than `1.0`, so that two keys at once (a
	/// diagonal) are no faster than one. A stick, which is analog, is not made
	/// longer or shorter. Without it each axis is limited on its own.
	pub normalize: bool,
}

impl Default for VectorAction {
	fn default() -> Self {
		Self {
			x: Vec::new(),
			y: Vec::new(),
			normalize: true,
		}
	}
}

#[cfg(test)]
mod tests {
	use super::*;

	#[test]
	fn bindings_read_back() {
		for text in [
			"Key:Space",
			"-Key:A",
			"Mouse:Left",
			"Gamepad:South",
			"Axis:LeftStickX",
			"-Axis:RightStickY",
			"Key:LeftControl+Key:S",
			"Key:LeftControl+Key:LeftShift+Key:S",
			"-Key:LeftAlt+Mouse:Left",
			"Gamepad:South@1",
			"-Axis:LeftStickX@0",
			"Key:LeftControl+Gamepad:South@2",
		] {
			let binding: Binding = text.parse().unwrap_or_else(|e| unreachable!("{text}: {e}"));
			assert_eq!(binding.to_string(), text, "round trip of {text}");
		}
	}

	#[test]
	fn bindings_are_forgiving_and_strict() {
		assert_eq!(
			"key: left_shift".parse::<Binding>().ok(),
			Some(Binding::from(ZKeyCode::LeftShift)),
			"case, spaces and separators"
		);
		assert_eq!(
			" - KEY:a ".parse::<Binding>().ok(),
			Some(Binding::from(ZKeyCode::A).negated()),
			"a minus in front"
		);
		assert!("ctrl:s".parse::<Binding>().is_err(), "a modifier is a `Kind:Name` too");
		for bad in [
			"Key:A@1",
			"Gamepad:South@x",
			"Gamepad:South@",
			"Gamepad:South@-1",
			"Space",
			"Key:",
			"Key:Banana",
			"Joystick:1",
			":A",
			"",
			"Key:A+",
			"+Key:A",
		] {
			assert!(bad.parse::<Binding>().is_err(), "{bad:?} must be rejected");
		}
	}

	#[test]
	fn a_chord_is_built_from_parts() {
		let chord = Binding::from(ZKeyCode::S).with(ZKeyCode::LeftControl);
		assert_eq!(chord.to_string(), "Key:LeftControl+Key:S", "Ctrl+S");
		assert_eq!(chord.with, vec![Source::Key(ZKeyCode::LeftControl)], "the modifier");
	}

	#[test]
	fn the_factor_composes() {
		let binding = Binding::from(ZKeyCode::W).scaled(2.0).negated();
		assert!((binding.scale + 2.0).abs() < f32::EPSILON, "scaled then negated");
	}
}
