//! Real gamepads, through `gilrs` (the Gamepad API in a browser).
//!
//! Only `Gilrs::next_event` is used, never `next_event_blocking`: that one
//! panics on the web, and a game loop must not wait for a gamepad anyway.

use std::{collections::HashMap, time::Duration};

use gilrs::{
	Axis, Button, Event, EventType, GamepadId, Gilrs, GilrsBuilder, MappingSource, PowerInfo,
	ff::{BaseEffect, BaseEffectType, Effect, EffectBuilder, Envelope, Repeat, Replay, Ticks},
};
use web_time::Instant;
use zenith_error::{ZResult, ZenithError};

use crate::{Input, InputEvent, PadCommand, RumbleEffect, ZGamePad, ZGamePadAxis, ZGamepadKind, ZPowerInfo};

/// How often the battery of the pads is looked at (`gilrs` has no event for
/// it).
const POWER_EVERY: Duration = Duration::from_secs(1);

const fn button(button: Button) -> Option<ZGamePad> {
	Some(match button {
		Button::South => ZGamePad::South,
		Button::East => ZGamePad::East,
		Button::West => ZGamePad::West,
		Button::North => ZGamePad::North,
		Button::LeftTrigger => ZGamePad::LeftBumper,
		Button::RightTrigger => ZGamePad::RightBumper,
		Button::LeftTrigger2 => ZGamePad::LeftTrigger,
		Button::RightTrigger2 => ZGamePad::RightTrigger,
		Button::Select => ZGamePad::Select,
		Button::Start => ZGamePad::Start,
		Button::Mode => ZGamePad::Mode,
		Button::LeftThumb => ZGamePad::LeftStick,
		Button::RightThumb => ZGamePad::RightStick,
		Button::DPadUp => ZGamePad::DPadUp,
		Button::DPadDown => ZGamePad::DPadDown,
		Button::DPadLeft => ZGamePad::DPadLeft,
		Button::DPadRight => ZGamePad::DPadRight,
		Button::C | Button::Z | Button::Unknown => return None,
	})
}

const fn axis(axis: Axis) -> Option<ZGamePadAxis> {
	Some(match axis {
		Axis::LeftStickX => ZGamePadAxis::LeftStickX,
		Axis::LeftStickY => ZGamePadAxis::LeftStickY,
		Axis::RightStickX => ZGamePadAxis::RightStickX,
		Axis::RightStickY => ZGamePadAxis::RightStickY,
		// Triggers of pads without a mapping; see `Gamepads::poll`.
		Axis::LeftZ | Axis::RightZ | Axis::DPadX | Axis::DPadY | Axis::Unknown => return None,
	})
}

/// The analog triggers arrive as the changing value of a button.
const fn trigger(button: Button) -> Option<ZGamePadAxis> {
	match button {
		Button::LeftTrigger2 => Some(ZGamePadAxis::LeftTrigger),
		Button::RightTrigger2 => Some(ZGamePadAxis::RightTrigger),
		_ => None,
	}
}

const fn power(info: PowerInfo) -> ZPowerInfo {
	match info {
		PowerInfo::Unknown => ZPowerInfo::Unknown,
		PowerInfo::Wired => ZPowerInfo::Wired,
		PowerInfo::Discharging(percent) => ZPowerInfo::Discharging(percent),
		PowerInfo::Charging(percent) => ZPowerInfo::Charging(percent),
		PowerInfo::Charged => ZPowerInfo::Charged,
	}
}

/// The motors a [`RumbleEffect`] asks for, as `gilrs` effects.
fn base_effects(effect: RumbleEffect) -> Vec<BaseEffect> {
	// The strengths are limited to 0..=1 by `RumbleEffect::checked`.
	#[allow(clippy::cast_sign_loss)]
	let magnitude = |strength: f32| (strength.clamp(0.0, 1.0) * f32::from(u16::MAX)) as u16;
	let scheduling = Replay {
		after: Ticks::from_ms(effect.delay_ms),
		play_for: Ticks::from_ms(effect.duration_ms),
		with_delay: Ticks::from_ms(0),
	};
	let mut effects = Vec::new();
	for kind in [
		(effect.strong > 0.0).then(|| BaseEffectType::Strong {
			magnitude: magnitude(effect.strong),
		}),
		(effect.weak > 0.0).then(|| BaseEffectType::Weak {
			magnitude: magnitude(effect.weak),
		}),
	]
	.into_iter()
	.flatten()
	{
		effects.push(BaseEffect {
			kind,
			scheduling,
			envelope: Envelope::default(),
		});
	}
	effects
}

/// The connected gamepads, as a source of events for [`Input`] and the one
/// that carries out what the game asks of them (vibration).
///
/// Call [`Self::poll`] once per frame. Gamepads get the lowest free number
/// (`0`, `1`, ...) in the order they are connected; the number of a pad that
/// is unplugged is given to the next one.
pub struct Gamepads {
	gilrs: Gilrs,
	slots: HashMap<GamepadId, usize>,
	/// Vibrations that may still be playing; dropping one stops it.
	effects: HashMap<usize, Effect>,
	last_power_check: Instant,
}

impl Gamepads {
	/// Starts listening to gamepads. The mappings of `SDL_GAMECONTROLLERCONFIG`
	/// (if set) are used on top of the ones that come with `gilrs`.
	///
	/// # Errors
	/// `Unsupported` if the system cannot give gamepads (then the game is
	/// better off without them).
	pub fn new() -> ZResult<Self> {
		Self::from_builder(GilrsBuilder::new().add_env_mappings(true))
	}

	/// The same, with more mappings: `sdl_mappings` is the text of
	/// `gamecontrollerdb.txt` (one line per pad), for pads that `gilrs` does
	/// not know.
	///
	/// # Errors
	/// As for [`Self::new`].
	pub fn with_mappings(sdl_mappings: &str) -> ZResult<Self> {
		Self::from_builder(GilrsBuilder::new().add_env_mappings(true).add_mappings(sdl_mappings))
	}

	fn from_builder(builder: GilrsBuilder) -> ZResult<Self> {
		let gilrs = builder
			.build()
			.map_err(|e| ZenithError::Unsupported(format!("gamepads: {e}")))?;
		Ok(Self {
			gilrs,
			slots: HashMap::new(),
			effects: HashMap::new(),
			last_power_check: Instant::now(),
		})
	}

	fn slot(&mut self, id: GamepadId) -> usize {
		if let Some(slot) = self.slots.get(&id) {
			return *slot;
		}
		let slot = (0..=self.slots.len())
			.find(|candidate| !self.slots.values().any(|taken| taken == candidate))
			.unwrap_or(self.slots.len());
		self.slots.insert(id, slot);
		slot
	}

	fn gilrs_id(&self, slot: usize) -> Option<GamepadId> {
		self.slots.iter().find(|(_, taken)| **taken == slot).map(|(id, _)| *id)
	}

	/// Hands everything that happened since the last call to `input`, carries
	/// out the vibrations it asked for, and keeps the batteries up to date.
	pub fn poll(&mut self, input: &mut Input) {
		while let Some(Event { id, event, .. }) = self.gilrs.next_event() {
			match event {
				EventType::Connected => self.connected(id, input),
				EventType::Disconnected => {
					if let Some(slot) = self.slots.remove(&id) {
						self.effects.remove(&slot);
						input.apply(InputEvent::GamepadDisconnected { id: slot });
					}
				}
				EventType::ButtonPressed(pressed, _) | EventType::ButtonReleased(pressed, _) => {
					if let Some(button) = button(pressed) {
						let slot = self.slot(id);
						input.apply(InputEvent::GamepadButton {
							id: slot,
							button,
							pressed: matches!(event, EventType::ButtonPressed(..)),
						});
					}
				}
				EventType::ButtonChanged(changed, value, _) => {
					if let Some(axis) = trigger(changed) {
						let slot = self.slot(id);
						input.apply(InputEvent::GamepadAxis { id: slot, axis, value });
					}
				}
				EventType::AxisChanged(changed, value, _) => self.axis_changed(id, changed, value, input),
				_ => {}
			}
		}
		for command in input.take_pad_commands() {
			self.carry_out(command);
		}
		if self.last_power_check.elapsed() >= POWER_EVERY {
			self.last_power_check = Instant::now();
			self.check_power(input);
		}
	}

	fn connected(&mut self, id: GamepadId, input: &mut Input) {
		let slot = self.slot(id);
		let gamepad = self.gilrs.gamepad(id);
		let mapped = gamepad.mapping_source() != MappingSource::None;
		let name = gamepad.name().to_string();
		if !mapped {
			zenith_log::warn!(
				"The gamepad {name:?} has no button layout: its buttons may not be the ones printed on it \
				 (add one with Gamepads::with_mappings or SDL_GAMECONTROLLERCONFIG)"
			);
		}
		input.apply(InputEvent::GamepadConnected {
			id: slot,
			name,
			kind: ZGamepadKind::from_vendor(gamepad.vendor_id()),
			ff_supported: gamepad.is_ff_supported(),
			mapped,
		});
		input.apply(InputEvent::GamepadPower {
			id: slot,
			power: power(gamepad.power_info()),
		});
	}

	fn axis_changed(&mut self, id: GamepadId, changed: Axis, value: f32, input: &mut Input) {
		let slot = self.slot(id);
		if let Some(axis) = axis(changed) {
			input.apply(InputEvent::GamepadAxis { id: slot, axis, value });
			return;
		}
		// Pads without a layout often give their triggers as the Z axes, from
		// -1 (released) to 1 (pulled). Mapped pads never do.
		let unmapped = self.gilrs.gamepad(id).mapping_source() == MappingSource::None;
		let trigger = match changed {
			Axis::LeftZ if unmapped => Some(ZGamePadAxis::LeftTrigger),
			Axis::RightZ if unmapped => Some(ZGamePadAxis::RightTrigger),
			_ => None,
		};
		if let Some(axis) = trigger {
			input.apply(InputEvent::GamepadAxis {
				id: slot,
				axis,
				value: f32::midpoint(value, 1.0),
			});
		}
	}

	fn check_power(&self, input: &mut Input) {
		for (id, slot) in &self.slots {
			let now = power(self.gilrs.gamepad(*id).power_info());
			if input.gamepad(*slot).map(|pad| pad.power_info()) != Some(now) {
				input.apply(InputEvent::GamepadPower { id: *slot, power: now });
			}
		}
	}

	/// Carries out a request; a failure is logged (the pad may have been
	/// unplugged since the game asked).
	fn carry_out(&mut self, command: PadCommand) {
		match command {
			PadCommand::StopRumble { id } => {
				self.effects.remove(&id);
			}
			PadCommand::Rumble { id, effect } => match self.rumble(id, effect) {
				Ok(effect) => {
					self.effects.insert(id, effect);
				}
				Err(e) => zenith_log::warn!("Cannot vibrate gamepad {id}: {e}"),
			},
		}
	}

	fn rumble(&mut self, slot: usize, effect: RumbleEffect) -> ZResult<Effect> {
		let failed = |e: gilrs::ff::Error| ZenithError::Unsupported(format!("force feedback: {e}"));
		let id = self
			.gilrs_id(slot)
			.ok_or_else(|| ZenithError::Unsupported(format!("gamepad {slot} is gone")))?;
		let mut builder = EffectBuilder::new();
		for base in base_effects(effect) {
			builder.add_effect(base);
		}
		let playing = builder
			.gamepads(&[id])
			.repeat(Repeat::For(Ticks::from_ms(
				effect.delay_ms.saturating_add(effect.duration_ms),
			)))
			.finish(&mut self.gilrs)
			.map_err(failed)?;
		playing.play().map_err(failed)?;
		Ok(playing)
	}
}

#[cfg(test)]
mod tests {
	use super::*;

	#[test]
	fn power_maps_one_to_one() {
		assert_eq!(power(PowerInfo::Unknown), ZPowerInfo::Unknown, "unknown");
		assert_eq!(power(PowerInfo::Wired), ZPowerInfo::Wired, "wired");
		assert_eq!(
			power(PowerInfo::Discharging(40)),
			ZPowerInfo::Discharging(40),
			"discharging"
		);
		assert_eq!(power(PowerInfo::Charging(90)), ZPowerInfo::Charging(90), "charging");
		assert_eq!(power(PowerInfo::Charged), ZPowerInfo::Charged, "charged");
	}

	#[test]
	fn a_rumble_becomes_one_effect_per_motor() {
		let both = base_effects(RumbleEffect::new().strong(1.0).weak(0.5).for_ms(200));
		assert_eq!(both.len(), 2, "two motors, two effects");
		assert!(
			matches!(both[0].kind, BaseEffectType::Strong { magnitude: u16::MAX }),
			"full strength is the largest number: {:?}",
			both[0].kind
		);
		assert!(
			matches!(both[1].kind, BaseEffectType::Weak { magnitude } if (32_000..33_500).contains(&magnitude)),
			"half is about half: {:?}",
			both[1].kind
		);
		assert_eq!(
			base_effects(RumbleEffect::new().weak(0.1)).len(),
			1,
			"one motor, one effect"
		);
		assert!(base_effects(RumbleEffect::new()).is_empty(), "no motor, no effect");
	}

	#[test]
	fn buttons_and_triggers_are_mapped() {
		assert_eq!(button(Button::South), Some(ZGamePad::South), "south");
		assert_eq!(
			button(Button::LeftTrigger),
			Some(ZGamePad::LeftBumper),
			"L1 is a bumper"
		);
		assert_eq!(
			button(Button::LeftTrigger2),
			Some(ZGamePad::LeftTrigger),
			"L2 is a trigger"
		);
		assert_eq!(button(Button::Unknown), None, "unknown");
		assert_eq!(
			trigger(Button::RightTrigger2),
			Some(ZGamePadAxis::RightTrigger),
			"R2 as an axis"
		);
		assert_eq!(trigger(Button::South), None, "a plain button has no axis");
		assert_eq!(axis(Axis::LeftStickX), Some(ZGamePadAxis::LeftStickX), "stick");
		assert_eq!(axis(Axis::LeftZ), None, "Z is handled by the pad");
	}
}
