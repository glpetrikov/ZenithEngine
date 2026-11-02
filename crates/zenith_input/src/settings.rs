//! `Settings/Input.toml`: how input is read and which actions exist.
//!
//! ```toml
//! [gamepad]
//! dead_zone = 0.15          # sticks: pushes shorter than this count as 0
//! trigger_dead_zone = 0.05
//! press_threshold = 0.5     # an axis counts as "pressed" beyond this
//! curve = 1.0               # response: 1 is linear, 2 is gentle near the middle
//!
//! [mouse]
//! sensitivity = 1.0         # multiplies mouse_delta
//! invert_y = false
//!
//! [[action]]                # pressed when any of its bindings is
//! name = "Jump"
//! bindings = ["Key:Space", "Gamepad:South"]
//!
//! [[action]]
//! name = "Save"
//! bindings = ["Key:LeftControl+Key:S"]
//!
//! [[vector]]                # two values, for walking and looking
//! name = "Move"
//! x = ["Key:D", "-Key:A", "Axis:LeftStickX"]
//! y = ["Key:W", "-Key:S", "Axis:LeftStickY"]
//! normalize = true
//! ```

use std::fmt::Write as _;

use serde::Deserialize;
use zenith_error::{ZResult, ZenithError};

use crate::action::{Binding, VectorAction};

/// How gamepads are read.
#[derive(Clone, Copy, Debug, PartialEq, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct GamepadSettings {
	/// Radius around the middle of a stick in which it reads as 0; the rest of
	/// the range is stretched to `0..=1`. `0.0..1.0`.
	pub dead_zone: f32,
	/// The same for triggers.
	pub trigger_dead_zone: f32,
	/// How far an axis must go for an action bound to it to count as pressed.
	pub press_threshold: f32,
	/// The response of sticks and triggers after the dead zone: the length of
	/// a push is raised to this power. `1.0` is linear, `2.0` makes small
	/// pushes finer and big ones faster. `0.1..=8.0`.
	pub curve: f32,
}

impl Default for GamepadSettings {
	fn default() -> Self {
		Self {
			dead_zone: 0.15,
			trigger_dead_zone: 0.05,
			press_threshold: 0.5,
			curve: 1.0,
		}
	}
}

/// How the mouse is read.
#[derive(Clone, Copy, Debug, PartialEq, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct MouseSettings {
	/// Multiplies `mouse_delta`.
	pub sensitivity: f32,
	/// `mouse_delta` goes up when the mouse goes down.
	pub invert_y: bool,
}

impl Default for MouseSettings {
	fn default() -> Self {
		Self {
			sensitivity: 1.0,
			invert_y: false,
		}
	}
}

/// All the numbers of `Input.toml`.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct InputSettings {
	pub gamepad: GamepadSettings,
	pub mouse: MouseSettings,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ActionDef {
	name: String,
	#[serde(default)]
	bindings: Vec<String>,
}

const fn yes() -> bool {
	true
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct VectorDef {
	name: String,
	#[serde(default)]
	x: Vec<String>,
	#[serde(default)]
	y: Vec<String>,
	#[serde(default = "yes")]
	normalize: bool,
}

#[derive(Default, Deserialize)]
#[serde(default, deny_unknown_fields)]
struct File {
	gamepad: GamepadSettings,
	mouse: MouseSettings,
	action: Vec<ActionDef>,
	vector: Vec<VectorDef>,
}

/// A parsed `Input.toml`.
#[derive(Clone, Debug, PartialEq)]
pub struct InputFile {
	pub settings: InputSettings,
	/// Button actions in the order of the file.
	pub actions: Vec<(String, Vec<Binding>)>,
	/// Vector actions in the order of the file.
	pub vectors: Vec<(String, VectorAction)>,
}

fn in_range(name: &str, value: f32, low: f32, high: f32) -> ZResult<()> {
	if value.is_finite() && value >= low && value < high {
		Ok(())
	} else {
		Err(ZenithError::InvalidSettings(format!(
			"Input.toml: {name} must be in {low}..{high}, got {value}"
		)))
	}
}

fn bindings(owner: &str, texts: &[String]) -> ZResult<Vec<Binding>> {
	texts
		.iter()
		.map(|text| text.parse())
		.collect::<ZResult<Vec<Binding>>>()
		.map_err(|e| ZenithError::InvalidSettings(format!("{owner}: {e}")))
}

fn check_name(name: &str, mut taken: impl Iterator<Item = bool>) -> ZResult<()> {
	if name.trim().is_empty() {
		return Err(ZenithError::InvalidSettings("Input.toml: an action has no name".into()));
	}
	if taken.any(|same| same) {
		return Err(ZenithError::InvalidSettings(format!(
			"Input.toml: the action {name:?} is defined twice"
		)));
	}
	Ok(())
}

impl InputFile {
	/// Reads the text of an `Input.toml`.
	///
	/// # Errors
	/// `InvalidSettings` for text that is not TOML, an unknown key, a number
	/// out of range, a binding that cannot be read, or two actions with one
	/// name.
	pub fn parse(text: &str) -> ZResult<Self> {
		let file: File = toml::from_str(text).map_err(|e| ZenithError::InvalidSettings(format!("Input.toml: {e}")))?;
		in_range("gamepad.dead_zone", file.gamepad.dead_zone, 0.0, 1.0)?;
		in_range("gamepad.trigger_dead_zone", file.gamepad.trigger_dead_zone, 0.0, 1.0)?;
		in_range("gamepad.press_threshold", file.gamepad.press_threshold, 0.0, 1.0)?;
		in_range("gamepad.curve", file.gamepad.curve, 0.1, 8.000_1)?;
		in_range("mouse.sensitivity", file.mouse.sensitivity, 0.0, f32::INFINITY)?;

		let mut actions: Vec<(String, Vec<Binding>)> = Vec::with_capacity(file.action.len());
		for def in file.action {
			check_name(&def.name, actions.iter().map(|(name, _)| *name == def.name))?;
			let list = bindings(&format!("action {:?}", def.name), &def.bindings)?;
			actions.push((def.name, list));
		}
		let mut vectors: Vec<(String, VectorAction)> = Vec::with_capacity(file.vector.len());
		for def in file.vector {
			check_name(
				&def.name,
				vectors
					.iter()
					.map(|(name, _)| *name == def.name)
					.chain(actions.iter().map(|(name, _)| *name == def.name)),
			)?;
			let owner = format!("vector {:?}", def.name);
			let vector = VectorAction {
				x: bindings(&owner, &def.x)?,
				y: bindings(&owner, &def.y)?,
				normalize: def.normalize,
			};
			vectors.push((def.name, vector));
		}
		Ok(Self {
			settings: InputSettings {
				gamepad: file.gamepad,
				mouse: file.mouse,
			},
			actions,
			vectors,
		})
	}
}

/// A TOML string for `text`.
fn quoted(text: &str) -> String {
	toml::Value::String(text.to_string()).to_string()
}

fn list(bindings: &[Binding]) -> String {
	let items: Vec<String> = bindings.iter().map(|binding| quoted(&binding.to_string())).collect();
	format!("[{}]", items.join(", "))
}

/// The text of the `[[action]]` table of a button action.
pub fn action_to_toml(name: &str, bindings: &[Binding]) -> String {
	let mut out = String::new();
	let _ = writeln!(
		out,
		"[[action]]\nname = {}\nbindings = {}\n",
		quoted(name),
		list(bindings)
	);
	out
}

/// The text of the `[[vector]]` table of a vector action.
pub fn vector_to_toml(name: &str, vector: &VectorAction) -> String {
	let mut out = String::new();
	let _ = writeln!(
		out,
		"[[vector]]\nname = {}\nx = {}\ny = {}\nnormalize = {}\n",
		quoted(name),
		list(&vector.x),
		list(&vector.y),
		vector.normalize
	);
	out
}

#[cfg(test)]
mod tests {
	use super::*;
	use crate::ZKeyCode;

	#[test]
	fn the_documented_file_is_read() -> ZResult<()> {
		let file = InputFile::parse(
			r#"
[gamepad]
dead_zone = 0.2
curve = 2.0

[mouse]
sensitivity = 2.5
invert_y = true

[[action]]
name = "Jump"
bindings = ["Key:Space", "Gamepad:South"]

[[action]]
name = "Save"
bindings = ["Key:LeftControl+Key:S"]

[[vector]]
name = "Move"
x = ["Key:D", "-Key:A", "Axis:LeftStickX"]
y = ["Key:W", "-Key:S", "Axis:LeftStickY"]
"#,
		)?;
		assert!(
			(file.settings.gamepad.dead_zone - 0.2).abs() < f32::EPSILON,
			"dead zone"
		);
		assert!((file.settings.gamepad.curve - 2.0).abs() < f32::EPSILON, "curve");
		assert!(
			(file.settings.gamepad.trigger_dead_zone - 0.05).abs() < f32::EPSILON,
			"what is not written stays at its default"
		);
		assert!(file.settings.mouse.invert_y, "invert_y");
		assert_eq!(file.actions.len(), 2, "two actions");
		assert_eq!(file.actions[0].0, "Jump", "in the order of the file");
		assert_eq!(file.actions[1].1[0].with.len(), 1, "a chord is read");
		assert_eq!(file.vectors.len(), 1, "one vector");
		assert!(file.vectors[0].1.normalize, "normalized unless said otherwise");
		assert_eq!(
			file.vectors[0].1.x.first(),
			Some(&Binding::from(ZKeyCode::D)),
			"vector bindings are read"
		);
		Ok(())
	}

	#[test]
	fn an_empty_file_is_the_defaults() -> ZResult<()> {
		let file = InputFile::parse("")?;
		assert_eq!(file.settings, InputSettings::default(), "default settings");
		assert!(file.actions.is_empty() && file.vectors.is_empty(), "no actions");
		Ok(())
	}

	#[test]
	fn mistakes_are_errors_that_say_what_is_wrong() {
		for (text, why) in [
			("[gamepad]\ndead_zone = 1.0\n", "dead zone too big"),
			("[gamepad]\ndead_zone = -0.1\n", "dead zone negative"),
			("[gamepad]\ncurve = 0.0\n", "curve too small"),
			("[gamepad]\ncurve = 20.0\n", "curve too big"),
			("[mouse]\nsensitivity = -1.0\n", "negative sensitivity"),
			("[gamepad]\nspeed = 1\n", "unknown key"),
			("[[action]]\nname = \"\"\n", "empty name"),
			("[[action]]\nname = \"A\"\n[[action]]\nname = \"A\"\n", "twice"),
			(
				"[[action]]\nname = \"A\"\n[[vector]]\nname = \"A\"\n",
				"vector and action share a name",
			),
			("[[action]]\nname = \"A\"\nbindings = [\"Key:Banana\"]\n", "bad binding"),
			("[[vector]]\nname = \"V\"\nx = [\"Nope\"]\n", "bad vector binding"),
			("not toml [", "not toml"),
		] {
			assert!(
				matches!(InputFile::parse(text), Err(ZenithError::InvalidSettings(_))),
				"{why}"
			);
		}
	}

	#[test]
	fn tables_written_by_us_are_read_back() -> ZResult<()> {
		let action = vec![Binding::from(ZKeyCode::Space), Binding::from(ZKeyCode::A).negated()];
		let vector = VectorAction {
			x: vec![Binding::from(ZKeyCode::D)],
			y: vec![Binding::from(ZKeyCode::W).with(ZKeyCode::LeftShift)],
			normalize: false,
		};
		let text = format!(
			"{}{}",
			action_to_toml("Odd \"name\"", &action),
			vector_to_toml("Move", &vector)
		);
		let file = InputFile::parse(&text)?;
		assert_eq!(file.actions, vec![("Odd \"name\"".to_string(), action)], "action");
		assert_eq!(file.vectors, vec![("Move".to_string(), vector)], "vector");
		Ok(())
	}
}
