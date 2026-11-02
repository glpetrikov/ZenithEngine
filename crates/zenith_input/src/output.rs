//! What the game asks the devices to do: the cursor, the motors of a gamepad.
//! `Input` only records the request; the code that owns the window or the
//! gamepads (the runtime, the editor) carries it out.

use serde::{Deserialize, Serialize};
use zenith_error::{ZResult, ZenithError};

/// What the cursor does while the game runs.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum CursorMode {
	/// An ordinary cursor.
	#[default]
	Normal,
	/// The cursor is not drawn.
	Hidden,
	/// The cursor is not drawn and cannot leave the window: for a camera that
	/// turns with the mouse (use `mouse_delta`). In a browser the player has
	/// to click first.
	Locked,
}

/// A vibration of a gamepad: the strong (low) and the weak (high) motor, for a
/// while.
///
/// ```
/// use zenith_input::RumbleEffect;
///
/// let effect = RumbleEffect::new().strong(0.8).weak(0.3).for_ms(250);
/// ```
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct RumbleEffect {
	/// The strength of the strong motor, `0.0..=1.0`.
	pub strong: f32,
	/// The strength of the weak motor, `0.0..=1.0`.
	pub weak: f32,
	/// How long it lasts, in milliseconds.
	pub duration_ms: u32,
	/// How long to wait before it starts, in milliseconds.
	pub delay_ms: u32,
}

impl Default for RumbleEffect {
	fn default() -> Self {
		Self {
			strong: 0.0,
			weak: 0.0,
			duration_ms: 250,
			delay_ms: 0,
		}
	}
}

impl RumbleEffect {
	#[must_use]
	pub fn new() -> Self {
		Self::default()
	}

	#[must_use]
	pub const fn strong(mut self, strength: f32) -> Self {
		self.strong = strength;
		self
	}

	#[must_use]
	pub const fn weak(mut self, strength: f32) -> Self {
		self.weak = strength;
		self
	}

	#[must_use]
	pub const fn for_ms(mut self, milliseconds: u32) -> Self {
		self.duration_ms = milliseconds;
		self
	}

	#[must_use]
	pub const fn after_ms(mut self, milliseconds: u32) -> Self {
		self.delay_ms = milliseconds;
		self
	}

	/// The effect with its strengths limited to `0.0..=1.0`.
	///
	/// # Errors
	/// `InvalidSettings` if a strength is not a number, or if there is nothing
	/// to play: both motors at `0.0` or no duration.
	pub fn checked(self) -> ZResult<Self> {
		if !self.strong.is_finite() || !self.weak.is_finite() {
			return Err(ZenithError::InvalidSettings("a rumble strength is not a number".into()));
		}
		let effect = Self {
			strong: self.strong.clamp(0.0, 1.0),
			weak: self.weak.clamp(0.0, 1.0),
			..self
		};
		if (effect.strong <= 0.0 && effect.weak <= 0.0) || effect.duration_ms == 0 {
			return Err(ZenithError::InvalidSettings(
				"a rumble needs a strength above 0 and a duration".into(),
			));
		}
		Ok(effect)
	}
}

/// A request for a gamepad.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum PadCommand {
	Rumble { id: usize, effect: RumbleEffect },
	StopRumble { id: usize },
}

#[cfg(test)]
mod tests {
	use super::*;

	#[test]
	fn effects_are_limited_and_checked() -> ZResult<()> {
		let effect = RumbleEffect::new().strong(2.0).weak(-1.0).for_ms(100).checked()?;
		assert!((effect.strong - 1.0).abs() < f32::EPSILON, "limited from above");
		assert!(effect.weak.abs() < f32::EPSILON, "and below");
		for bad in [
			RumbleEffect::new(),
			RumbleEffect::new().strong(0.5).for_ms(0),
			RumbleEffect::new().strong(f32::NAN),
		] {
			assert!(bad.checked().is_err(), "{bad:?} plays nothing");
		}
		Ok(())
	}
}
