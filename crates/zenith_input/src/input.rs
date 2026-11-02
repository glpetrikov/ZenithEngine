//! The state of every input device.

use std::{
	collections::{BTreeMap, HashMap, HashSet},
	hash::Hash,
	mem,
};

use glam::Vec2;
use zenith_error::{ZResult, ZenithError};

use crate::{
	CursorMode, GamepadInfo, InputEvent, InputFile, InputSettings, PadCommand, RecordedFrame, Recording, RumbleEffect,
	ZGamePad, ZGamePadAxis, ZGamepadKind, ZKeyCode, ZMouseButton, ZPowerInfo, ZStick,
	action::{Binding, Source, VectorAction},
	settings::{GamepadSettings, action_to_toml, vector_to_toml},
};

/// How far an axis must be pushed while listening for a binding to be taken.
const LISTEN_THRESHOLD: f32 = 0.8;

/// Which of a kind of button are down, and which went down or up since the
/// frame began.
#[derive(Clone, Debug)]
struct ButtonSet<T> {
	down: HashSet<T>,
	pressed: HashSet<T>,
	released: HashSet<T>,
	/// When (on the clock of `Input`) each held button went down.
	since: HashMap<T, f64>,
}

impl<T> Default for ButtonSet<T> {
	fn default() -> Self {
		Self {
			down: HashSet::new(),
			pressed: HashSet::new(),
			released: HashSet::new(),
			since: HashMap::new(),
		}
	}
}

impl<T: Copy + Eq + Hash> ButtonSet<T> {
	/// A button that is already down stays as it is: the repeat of a held key
	/// is not a new press.
	fn press(&mut self, button: T, now: f64) {
		if self.down.insert(button) {
			self.pressed.insert(button);
			self.since.insert(button, now);
		}
	}

	fn release(&mut self, button: T) {
		if self.down.remove(&button) {
			self.released.insert(button);
			self.since.remove(&button);
		}
	}

	fn release_all(&mut self) {
		self.released.extend(self.down.drain());
		self.since.clear();
	}

	fn end_frame(&mut self) {
		self.pressed.clear();
		self.released.clear();
	}
}

#[derive(Clone, Debug)]
struct Pad {
	name: String,
	kind: ZGamepadKind,
	ff_supported: bool,
	mapped: bool,
	power: ZPowerInfo,
	buttons: ButtonSet<ZGamePad>,
	axes: HashMap<ZGamePadAxis, f32>,
}

impl Pad {
	fn new(name: String) -> Self {
		Self {
			name,
			kind: ZGamepadKind::Generic,
			ff_supported: false,
			mapped: true,
			power: ZPowerInfo::Unknown,
			buttons: ButtonSet::default(),
			axes: HashMap::new(),
		}
	}

	fn raw(&self, axis: ZGamePadAxis) -> f32 {
		self.axes.get(&axis).copied().unwrap_or(0.0)
	}

	/// A push of `length` after the dead zone, stretched to `0..=1` and bent
	/// by the response curve.
	fn shaped(length: f32, zone: f32, curve: f32) -> f32 {
		if length <= zone {
			return 0.0;
		}
		((length - zone) / (1.0 - zone).max(f32::EPSILON)).min(1.0).powf(curve)
	}

	fn stick(&self, stick: ZStick, settings: &GamepadSettings) -> Vec2 {
		let (x, y) = stick.axes();
		let raw = Vec2::new(self.raw(x), self.raw(y));
		let length = raw.length();
		if length <= f32::EPSILON {
			return Vec2::ZERO;
		}
		// The direction is kept; only the length is reshaped.
		raw / length * Self::shaped(length, settings.dead_zone, settings.curve)
	}

	fn axis(&self, axis: ZGamePadAxis, settings: &GamepadSettings) -> f32 {
		match axis {
			ZGamePadAxis::LeftStickX => self.stick(ZStick::Left, settings).x,
			ZGamePadAxis::LeftStickY => self.stick(ZStick::Left, settings).y,
			ZGamePadAxis::RightStickX => self.stick(ZStick::Right, settings).x,
			ZGamePadAxis::RightStickY => self.stick(ZStick::Right, settings).y,
			ZGamePadAxis::LeftTrigger | ZGamePadAxis::RightTrigger => Self::shaped(
				self.raw(axis).clamp(0.0, 1.0),
				settings.trigger_dead_zone,
				settings.curve,
			),
		}
	}
}

/// A read-only view of one gamepad, from [`Input::gamepad`].
#[derive(Clone, Copy, Debug)]
pub struct GamepadRef<'a> {
	id: usize,
	pad: &'a Pad,
	settings: &'a GamepadSettings,
}

impl GamepadRef<'_> {
	/// The number of the gamepad.
	#[must_use]
	pub const fn id(&self) -> usize {
		self.id
	}

	#[must_use]
	pub fn name(&self) -> &str {
		&self.pad.name
	}

	/// The family of the pad (for the pictures of its buttons).
	#[must_use]
	pub const fn kind(&self) -> ZGamepadKind {
		self.pad.kind
	}

	/// The battery of the pad. `Unknown` when the system does not say, which
	/// is always so in a browser.
	#[must_use]
	pub const fn power_info(&self) -> ZPowerInfo {
		self.pad.power
	}

	/// Can the pad vibrate? (It cannot in a browser.)
	#[must_use]
	pub const fn is_ff_supported(&self) -> bool {
		self.pad.ff_supported
	}

	/// Do the buttons of the pad follow the standard layout? When not, what
	/// the pad reports may not be what is printed on its buttons.
	#[must_use]
	pub const fn has_standard_mapping(&self) -> bool {
		self.pad.mapped
	}

	#[must_use]
	pub fn is_pressed(&self, button: ZGamePad) -> bool {
		self.pad.buttons.down.contains(&button)
	}

	#[must_use]
	pub fn is_just_pressed(&self, button: ZGamePad) -> bool {
		self.pad.buttons.pressed.contains(&button)
	}

	#[must_use]
	pub fn is_just_released(&self, button: ZGamePad) -> bool {
		self.pad.buttons.released.contains(&button)
	}

	/// An axis after the dead zone and the response curve.
	#[must_use]
	pub fn axis(&self, axis: ZGamePadAxis) -> f32 {
		self.pad.axis(axis, self.settings)
	}

	/// A stick as a vector: `x` right, `y` up, never longer than `1.0`, after
	/// the dead zone (a round one) and the response curve. Normalize it
	/// (`.normalize_or_zero()`) for a direction only.
	#[must_use]
	pub fn stick(&self, stick: ZStick) -> Vec2 {
		self.pad.stick(stick, self.settings)
	}
}

/// A gamepad that can be given orders, from [`Input::gamepad_mut`].
#[derive(Debug)]
pub struct GamepadMut<'a> {
	input: &'a mut Input,
	id: usize,
}

impl GamepadMut<'_> {
	/// Makes the pad vibrate.
	///
	/// # Errors
	/// `Unsupported` if the pad cannot vibrate or is gone, `InvalidSettings`
	/// for an effect that plays nothing.
	pub fn rumble(&mut self, effect: RumbleEffect) -> ZResult<()> {
		self.input.rumble(self.id, effect)
	}

	/// Stops the vibration.
	pub fn stop_rumble(&mut self) {
		self.input.stop_rumble(self.id);
	}
}

/// A button action and what it has done.
///
/// The edges are noted when the event happens, not when somebody asks, so a
/// tap that begins and ends between two questions is not lost.
#[derive(Clone, Debug, Default)]
struct Action {
	bindings: Vec<Binding>,
	/// Was it pressed after the last event that could change it?
	pressed: bool,
	just_pressed: bool,
	just_released: bool,
	/// When it went down, while it is held.
	since: Option<f64>,
	/// When it last went down, unless that press has been used up.
	pressed_at: Option<f64>,
}

/// What is being recorded.
#[derive(Clone, Debug, Default)]
struct Recorder {
	frames: Vec<RecordedFrame>,
	events: Vec<InputEvent>,
}

/// Keyboard, mouse, gamepads and actions, with what they did this frame.
///
/// It is a plain value: it reads nothing by itself. Events from anywhere reach
/// it through [`Self::apply`] (or the adapters of the `winit` and `gamepad`
/// features), the game asks it questions, and [`Self::end_frame`] clears what
/// only lasts a frame. In the engine it is a resource of the world
/// (`world.input()`, or `Res<Input>` in a system) and `update_systems` calls
/// `end_frame` after the systems have run.
///
/// ```
/// use zenith_input::{Input, InputEvent, ZKeyCode};
///
/// let mut input = Input::default();
/// input.apply(InputEvent::KeyDown(ZKeyCode::W));
/// assert!(input.is_key_pressed(ZKeyCode::W));
/// assert!(input.is_key_just_pressed(ZKeyCode::W));
/// input.end_frame(0.016);
/// assert!(input.is_key_pressed(ZKeyCode::W), "still held");
/// assert!(!input.is_key_just_pressed(ZKeyCode::W), "but not new");
/// ```
#[derive(Clone, Debug)]
#[cfg_attr(feature = "ecs", derive(bevy_ecs::resource::Resource))]
// The flags are independent facts (focus, listening, a pending cursor change...).
#[allow(clippy::struct_excessive_bools)]
pub struct Input {
	/// Seconds of play, the sum of the `dt` of the frames that ended. Kept in
	/// `f64`: in `f32` a day of play leaves no room for a single frame.
	time: f64,
	keys: ButtonSet<ZKeyCode>,
	mouse: ButtonSet<ZMouseButton>,
	pads: BTreeMap<usize, Pad>,
	position: Option<Vec2>,
	cursor_delta: Vec2,
	raw_delta: Vec2,
	has_raw: bool,
	scroll: Vec2,
	text: String,
	focused: bool,
	settings: InputSettings,

	actions: BTreeMap<String, Action>,
	vectors: BTreeMap<String, VectorAction>,
	/// What the actions were defined as, before the player changed them.
	default_actions: BTreeMap<String, Vec<Binding>>,
	default_vectors: BTreeMap<String, VectorAction>,
	/// Buttons of gamepads that were unplugged while held, in this frame: the
	/// pad is gone, but its buttons did come up.
	unplugged_released: HashSet<(usize, ZGamePad)>,

	listening: bool,
	/// Modifier keys held since `listen_for_binding`, for a chord.
	listen_modifiers: Vec<Source>,
	captured: Option<Binding>,

	cursor_mode: CursorMode,
	cursor_changed: bool,
	pad_commands: Vec<PadCommand>,
	recorder: Option<Recorder>,
}

impl Default for Input {
	fn default() -> Self {
		Self {
			time: 0.0,
			keys: ButtonSet::default(),
			mouse: ButtonSet::default(),
			pads: BTreeMap::new(),
			position: None,
			cursor_delta: Vec2::ZERO,
			raw_delta: Vec2::ZERO,
			has_raw: false,
			scroll: Vec2::ZERO,
			text: String::new(),
			focused: true,
			settings: InputSettings::default(),
			actions: BTreeMap::new(),
			vectors: BTreeMap::new(),
			default_actions: BTreeMap::new(),
			default_vectors: BTreeMap::new(),
			unplugged_released: HashSet::new(),
			listening: false,
			listen_modifiers: Vec::new(),
			captured: None,
			cursor_mode: CursorMode::Normal,
			cursor_changed: false,
			pad_commands: Vec::new(),
			recorder: None,
		}
	}
}

impl Input {
	// --- feeding ------------------------------------------------------------

	/// Takes one event into account.
	pub fn apply(&mut self, event: InputEvent) {
		if let Some(recorder) = &mut self.recorder {
			recorder.events.push(event.clone());
		}
		self.listen(&event);
		// Actions are looked at when something that can change them happens,
		// so that a tap that begins and ends before anybody asks is not lost.
		let affects_actions = matches!(
			event,
			InputEvent::KeyDown(_)
				| InputEvent::KeyUp(_)
				| InputEvent::MouseDown(_)
				| InputEvent::MouseUp(_)
				| InputEvent::Focus(false)
				| InputEvent::GamepadConnected { .. }
				| InputEvent::GamepadDisconnected { .. }
				| InputEvent::GamepadButton { .. }
				| InputEvent::GamepadAxis { .. }
		);
		let now = self.time;
		match event {
			InputEvent::KeyDown(key) => self.keys.press(key, now),
			InputEvent::KeyUp(key) => self.keys.release(key),
			InputEvent::MouseDown(button) => self.mouse.press(button, now),
			InputEvent::MouseUp(button) => self.mouse.release(button),
			InputEvent::MouseMoved(position) => {
				if let Some(before) = self.position {
					self.cursor_delta += position - before;
				}
				self.position = Some(position);
			}
			InputEvent::MouseLeft => self.position = None,
			InputEvent::MouseMotion(delta) => {
				self.raw_delta += delta;
				self.has_raw = true;
			}
			InputEvent::Scroll(delta) => self.scroll += delta,
			InputEvent::Text(text) => self.text.extend(text.chars().filter(|c| !c.is_control())),
			InputEvent::Focus(focused) => {
				self.focused = focused;
				if !focused {
					self.keys.release_all();
					self.mouse.release_all();
				}
			}
			InputEvent::GamepadConnected {
				id,
				name,
				kind,
				ff_supported,
				mapped,
			} => {
				self.pads.insert(
					id,
					Pad {
						kind,
						ff_supported,
						mapped,
						..Pad::new(name)
					},
				);
			}
			InputEvent::GamepadDisconnected { id } => {
				// The buttons that were held came up with the pad, and "just
				// released" must still say so.
				if let Some(pad) = self.pads.remove(&id) {
					self.unplugged_released
						.extend(pad.buttons.down.into_iter().map(|button| (id, button)));
				}
			}
			InputEvent::GamepadButton { id, button, pressed } => {
				let pad = self.pad_or_new(id);
				if pressed {
					pad.buttons.press(button, now);
				} else {
					pad.buttons.release(button);
				}
			}
			InputEvent::GamepadAxis { id, axis, value } => {
				let value = if value.is_finite() { value.clamp(-1.0, 1.0) } else { 0.0 };
				self.pad_or_new(id).axes.insert(axis, value);
			}
			InputEvent::GamepadPower { id, power } => self.pad_or_new(id).power = power,
		}
		if affects_actions {
			self.refresh_actions();
		}
	}

	/// Compares every action with what it was after the previous event, and
	/// notes the ones that went down or up. Nothing is allocated: the map is
	/// taken out for a moment so that the devices can be read meanwhile.
	fn refresh_actions(&mut self) {
		let mut actions = mem::take(&mut self.actions);
		let now = self.time;
		for action in actions.values_mut() {
			let pressed = self.any_active(&action.bindings);
			if pressed && !action.pressed {
				action.just_pressed = true;
				action.since = Some(now);
				action.pressed_at = Some(now);
			} else if !pressed && action.pressed {
				action.just_released = true;
				action.since = None;
			}
			action.pressed = pressed;
		}
		self.actions = actions;
	}

	/// Makes the action agree with the devices without noting an edge: for an
	/// action that was just defined or whose bindings were changed while a key
	/// was held.
	fn resync_action(&mut self, name: &str) {
		let now = self.time;
		if let Some(mut action) = self.actions.remove(name) {
			action.pressed = self.any_active(&action.bindings);
			action.since = action.pressed.then(|| action.since.unwrap_or(now));
			self.actions.insert(name.to_string(), action);
		}
	}

	/// Listens for the player's choice of a binding (see
	/// [`Self::listen_for_binding`]). Modifier keys held down make a chord
	/// with the next press; a modifier pressed and let go alone is the binding.
	fn listen(&mut self, event: &InputEvent) {
		if !self.listening || self.captured.is_some() {
			return;
		}
		let is_modifier = |key: ZKeyCode| {
			matches!(
				key,
				ZKeyCode::LeftControl
					| ZKeyCode::RightControl
					| ZKeyCode::LeftShift
					| ZKeyCode::RightShift
					| ZKeyCode::LeftAlt
					| ZKeyCode::RightAlt
					| ZKeyCode::LeftSuper
					| ZKeyCode::RightSuper
			)
		};
		let captured = match event {
			InputEvent::KeyDown(key) if is_modifier(*key) => {
				let source = Source::Key(*key);
				if !self.listen_modifiers.contains(&source) {
					self.listen_modifiers.push(source);
				}
				None
			}
			InputEvent::KeyUp(key) if is_modifier(*key) => {
				let source = Source::Key(*key);
				let alone = self.listen_modifiers == [source];
				self.listen_modifiers.retain(|held| *held != source);
				alone.then(|| Binding::from(*key))
			}
			other => Self::binding_of(other).map(|binding| Binding {
				with: self.listen_modifiers.clone(),
				..binding
			}),
		};
		if captured.is_some() {
			self.captured = captured;
			self.listening = false;
			self.listen_modifiers.clear();
		}
	}

	/// The pad with this number; an event for a pad nobody announced makes one
	/// (emulated gamepads need no connect event).
	fn pad_or_new(&mut self, id: usize) -> &mut Pad {
		self.pads.entry(id).or_insert_with(|| Pad::new(format!("Gamepad {id}")))
	}

	/// The binding a press stands for, when someone is choosing one.
	fn binding_of(event: &InputEvent) -> Option<Binding> {
		match event {
			InputEvent::KeyDown(key) => Some(Binding::from(*key)),
			InputEvent::MouseDown(button) => Some(Binding::from(*button)),
			InputEvent::GamepadButton {
				button, pressed: true, ..
			} => Some(Binding::from(*button)),
			InputEvent::GamepadAxis { axis, value, .. } if value.abs() >= LISTEN_THRESHOLD => {
				let binding = Binding::from(*axis);
				Some(if *value < 0.0 { binding.negated() } else { binding })
			}
			_ => None,
		}
	}

	/// Ends the frame, which lasted `dt` seconds: what only lasts a frame (new
	/// presses and releases, the mouse movement, the wheel, the typed text) is
	/// cleared, and the clock moves on. What is held stays.
	pub fn end_frame(&mut self, dt: f32) {
		for action in self.actions.values_mut() {
			action.just_pressed = false;
			action.just_released = false;
		}
		self.unplugged_released.clear();
		self.keys.end_frame();
		self.mouse.end_frame();
		for pad in self.pads.values_mut() {
			pad.buttons.end_frame();
		}
		self.cursor_delta = Vec2::ZERO;
		self.raw_delta = Vec2::ZERO;
		self.has_raw = false;
		self.scroll = Vec2::ZERO;
		self.text.clear();

		if let Some(recorder) = &mut self.recorder {
			recorder.frames.push(RecordedFrame {
				dt,
				events: mem::take(&mut recorder.events),
			});
		}
		if dt.is_finite() && dt > 0.0 {
			self.time += f64::from(dt);
		}
	}

	/// Seconds of play: the sum of the `dt` of the frames that ended.
	#[must_use]
	pub const fn time(&self) -> f64 {
		self.time
	}

	// --- keyboard -----------------------------------------------------------

	/// Is the key held down?
	#[must_use]
	pub fn is_key_pressed(&self, key: ZKeyCode) -> bool {
		self.keys.down.contains(&key)
	}

	/// Did the key go down in this frame?
	#[must_use]
	pub fn is_key_just_pressed(&self, key: ZKeyCode) -> bool {
		self.keys.pressed.contains(&key)
	}

	/// Did the key come up in this frame?
	#[must_use]
	pub fn is_key_just_released(&self, key: ZKeyCode) -> bool {
		self.keys.released.contains(&key)
	}

	/// For how many seconds has the key been held? `None` if it is not.
	#[must_use]
	pub fn key_held_for(&self, key: ZKeyCode) -> Option<f32> {
		self.keys.since.get(&key).map(|since| (self.time - since) as f32)
	}

	/// The keys that are held down, in no particular order.
	pub fn pressed_keys(&self) -> impl Iterator<Item = ZKeyCode> + '_ {
		self.keys.down.iter().copied()
	}

	/// The characters typed in this frame.
	#[must_use]
	pub fn text(&self) -> &str {
		&self.text
	}

	// --- mouse --------------------------------------------------------------

	#[must_use]
	pub fn is_mouse_pressed(&self, button: ZMouseButton) -> bool {
		self.mouse.down.contains(&button)
	}

	#[must_use]
	pub fn is_mouse_just_pressed(&self, button: ZMouseButton) -> bool {
		self.mouse.pressed.contains(&button)
	}

	#[must_use]
	pub fn is_mouse_just_released(&self, button: ZMouseButton) -> bool {
		self.mouse.released.contains(&button)
	}

	/// The cursor in the window in physical pixels; `None` while it is outside.
	#[must_use]
	pub const fn mouse_position(&self) -> Option<Vec2> {
		self.position
	}

	/// How far the mouse moved in this frame, times the sensitivity (and
	/// flipped upside down with `invert_y`).
	///
	/// With an ordinary cursor this is the movement of the cursor in pixels,
	/// so the sensitivity means the same everywhere. With
	/// [`CursorMode::Locked`] the cursor does not move, and the raw movement
	/// of the mouse is used instead (its own units, steady from frame to
	/// frame).
	#[must_use]
	pub fn mouse_delta(&self) -> Vec2 {
		let delta = if self.cursor_mode == CursorMode::Locked && self.has_raw {
			self.raw_delta
		} else {
			self.cursor_delta
		};
		let flip = if self.settings.mouse.invert_y { -1.0 } else { 1.0 };
		delta * Vec2::new(1.0, flip) * self.settings.mouse.sensitivity
	}

	/// How far the wheel turned in this frame, in lines.
	#[must_use]
	pub const fn scroll(&self) -> Vec2 {
		self.scroll
	}

	/// Does the window have the focus?
	#[must_use]
	pub const fn is_focused(&self) -> bool {
		self.focused
	}

	// --- cursor -------------------------------------------------------------

	/// Asks for another cursor behaviour. It takes effect when the code that
	/// owns the window carries it out (the runtime does it every frame).
	pub fn set_cursor_mode(&mut self, mode: CursorMode) {
		if self.cursor_mode != mode {
			self.cursor_mode = mode;
			self.cursor_changed = true;
		}
	}

	/// The cursor behaviour that was asked for last.
	#[must_use]
	pub const fn cursor_mode(&self) -> CursorMode {
		self.cursor_mode
	}

	/// For the code that owns the window: the new cursor behaviour if it
	/// changed since the last call.
	pub const fn take_cursor_mode(&mut self) -> Option<CursorMode> {
		if self.cursor_changed {
			self.cursor_changed = false;
			Some(self.cursor_mode)
		} else {
			None
		}
	}

	// --- gamepads -----------------------------------------------------------

	/// The connected gamepads, by number.
	#[must_use]
	pub fn list_gamepads(&self) -> Vec<GamepadInfo> {
		self.pads
			.iter()
			.map(|(id, pad)| GamepadInfo {
				id: *id,
				name: pad.name.clone(),
				kind: pad.kind,
			})
			.collect()
	}

	/// A view of gamepad `id`; `None` if there is no such pad.
	#[must_use]
	pub fn gamepad(&self, id: usize) -> Option<GamepadRef<'_>> {
		self.pads.get(&id).map(|pad| GamepadRef {
			id,
			pad,
			settings: &self.settings.gamepad,
		})
	}

	/// Gamepad `id`, to give it orders; `None` if there is no such pad.
	pub fn gamepad_mut(&mut self, id: usize) -> Option<GamepadMut<'_>> {
		self.pads.contains_key(&id).then_some(GamepadMut { input: self, id })
	}

	/// Is the button of gamepad `id` held down? `false` for a gamepad that is
	/// not there.
	#[must_use]
	pub fn is_gamepad_pressed(&self, button: ZGamePad, id: usize) -> bool {
		self.gamepad(id).is_some_and(|pad| pad.is_pressed(button))
	}

	#[must_use]
	pub fn is_gamepad_just_pressed(&self, button: ZGamePad, id: usize) -> bool {
		self.gamepad(id).is_some_and(|pad| pad.is_just_pressed(button))
	}

	#[must_use]
	pub fn is_gamepad_just_released(&self, button: ZGamePad, id: usize) -> bool {
		self.gamepad(id).is_some_and(|pad| pad.is_just_released(button))
			|| self.unplugged_released.contains(&(id, button))
	}

	/// The axis of gamepad `id` after the dead zone and the curve: a stick is
	/// `-1.0..=1.0`, a trigger `0.0..=1.0`, and `0.0` for a pad that is not
	/// there. A stick is judged as a whole (a push that is short in both
	/// directions is no push), and the range outside the dead zone is
	/// stretched to the full one.
	#[must_use]
	pub fn gamepad_axis(&self, axis: ZGamePadAxis, id: usize) -> f32 {
		self.gamepad(id).map_or(0.0, |pad| pad.axis(axis))
	}

	/// A stick of gamepad `id` as a vector; zero for a pad that is not there.
	#[must_use]
	pub fn gamepad_stick(&self, stick: ZStick, id: usize) -> Vec2 {
		self.gamepad(id).map_or(Vec2::ZERO, |pad| pad.stick(stick))
	}

	/// Makes gamepad `id` vibrate. The request is carried out by the code that
	/// owns the gamepads (`Gamepads::poll`).
	///
	/// # Errors
	/// `Unsupported` if there is no such pad or it cannot vibrate (check
	/// `gamepad(id)?.is_ff_supported()` to know before); `InvalidSettings` for
	/// an effect that plays nothing.
	pub fn rumble(&mut self, id: usize, effect: RumbleEffect) -> ZResult<()> {
		let pad = self
			.pads
			.get(&id)
			.ok_or_else(|| ZenithError::Unsupported(format!("gamepad {id} is not connected")))?;
		if !pad.ff_supported {
			return Err(ZenithError::Unsupported(format!(
				"gamepad {id} ({}) does not support force feedback",
				pad.name
			)));
		}
		self.pad_commands.push(PadCommand::Rumble {
			id,
			effect: effect.checked()?,
		});
		Ok(())
	}

	/// Stops the vibration of gamepad `id`.
	pub fn stop_rumble(&mut self, id: usize) {
		if self.pads.contains_key(&id) {
			self.pad_commands.push(PadCommand::StopRumble { id });
		}
	}

	/// For the code that owns the gamepads: what was asked of them since the
	/// last call.
	pub fn take_pad_commands(&mut self) -> Vec<PadCommand> {
		mem::take(&mut self.pad_commands)
	}

	// --- actions ------------------------------------------------------------

	/// Defines the action `name`, replacing one with that name. The inputs are
	/// anything that turns into a [`Binding`]: a key, a mouse button, a gamepad
	/// button or axis. The action is pressed when **any** of them is.
	///
	/// This is also what the action is reset to by [`Self::reset_action`]
	/// after the player changes it.
	///
	/// ```
	/// use zenith_input::{Binding, Input, ZGamePad, ZKeyCode};
	///
	/// let mut input = Input::default();
	/// let space = Binding::from(ZKeyCode::Space);
	/// input.add_action("Jump", [space, ZGamePad::South.into()]);
	/// let ctrl_s = Binding::from(ZKeyCode::S).with(ZKeyCode::LeftControl);
	/// input.add_action("Save", [ctrl_s]);
	/// ```
	pub fn add_action(&mut self, name: impl Into<String>, bindings: impl IntoIterator<Item = Binding>) {
		let name = name.into();
		let bindings: Vec<Binding> = bindings.into_iter().collect();
		self.default_actions.insert(name.clone(), bindings.clone());
		self.actions.insert(
			name.clone(),
			Action {
				bindings,
				..Action::default()
			},
		);
		self.resync_action(&name);
	}

	/// Defines a vector action, such as walking (see [`VectorAction`]).
	pub fn add_vector_action(&mut self, name: impl Into<String>, action: VectorAction) {
		let name = name.into();
		self.default_vectors.insert(name.clone(), action.clone());
		self.vectors.insert(name, action);
	}

	/// Removes the button action; `true` if there was one.
	pub fn remove_action(&mut self, name: &str) -> bool {
		self.default_actions.remove(name);
		self.actions.remove(name).is_some()
	}

	/// Removes the vector action; `true` if there was one.
	pub fn remove_vector_action(&mut self, name: &str) -> bool {
		self.default_vectors.remove(name);
		self.vectors.remove(name).is_some()
	}

	/// The names of the button actions, sorted.
	pub fn actions(&self) -> impl Iterator<Item = &str> {
		self.actions.keys().map(String::as_str)
	}

	/// The names of the vector actions, sorted.
	pub fn vector_actions(&self) -> impl Iterator<Item = &str> {
		self.vectors.keys().map(String::as_str)
	}

	/// What the action is bound to now; `None` if there is no such action.
	#[must_use]
	pub fn action_bindings(&self, name: &str) -> Option<&[Binding]> {
		self.actions.get(name).map(|action| action.bindings.as_slice())
	}

	/// The vector action; `None` if there is no such action.
	#[must_use]
	pub fn vector_action(&self, name: &str) -> Option<&VectorAction> {
		self.vectors.get(name)
	}

	/// The vector action, to change its bindings.
	pub fn vector_action_mut(&mut self, name: &str) -> Option<&mut VectorAction> {
		self.vectors.get_mut(name)
	}

	/// The value of a source. Gamepad sources come from the pad `pad`, or from
	/// all of them (the one that pushes furthest) when it is `None`.
	fn source_value(&self, source: Source, pad: Option<usize>) -> f32 {
		let held = |down: bool| if down { 1.0 } else { 0.0 };
		let mut pads = self
			.pads
			.iter()
			.filter(move |(id, _)| pad.is_none_or(|wanted| wanted == **id))
			.map(|(_, pad)| pad);
		match source {
			Source::Key(key) => held(self.is_key_pressed(key)),
			Source::Mouse(button) => held(self.is_mouse_pressed(button)),
			Source::GamepadButton(button) => held(pads.any(|pad| pad.buttons.down.contains(&button))),
			Source::GamepadAxis(axis) => {
				pads.map(|pad| pad.axis(axis, &self.settings.gamepad))
					.fold(
						0.0,
						|best: f32, value| if value.abs() > best.abs() { value } else { best },
					)
			}
		}
	}

	fn is_source_active(&self, source: Source) -> bool {
		self.source_value(source, None).abs() > self.settings.gamepad.press_threshold
	}

	/// The value of a binding: that of its source times its factor, or 0 while
	/// the rest of its chord is not held.
	fn binding_value(&self, binding: &Binding) -> f32 {
		if binding.with.iter().all(|modifier| self.is_source_active(*modifier)) {
			self.source_value(binding.source, binding.pad) * binding.scale
		} else {
			0.0
		}
	}

	/// Is **any** of the bindings in use (past the press threshold, in either
	/// direction)?
	fn any_active(&self, bindings: &[Binding]) -> bool {
		let threshold = self.settings.gamepad.press_threshold;
		bindings
			.iter()
			.any(|binding| self.binding_value(binding).abs() > threshold)
	}

	fn sum(&self, bindings: &[Binding]) -> f32 {
		bindings
			.iter()
			.map(|binding| self.binding_value(binding))
			.sum::<f32>()
			.clamp(-1.0, 1.0)
	}

	/// The value of the action, `-1.0..=1.0`: the sum of its bindings, each a
	/// button (`1.0` while down) or an axis (after the dead zone) times its
	/// factor. `0.0` for an action that does not exist. For asking whether the
	/// action is in use, see [`Self::is_action_pressed`]: it does not depend on
	/// bindings cancelling each other out.
	#[must_use]
	pub fn action_value(&self, name: &str) -> f32 {
		self.actions.get(name).map_or(0.0, |action| self.sum(&action.bindings))
	}

	/// The two values of a vector action: `x` and `y` are the sums of their
	/// bindings, each in `-1.0..=1.0`. With `normalize` (the default) the
	/// vector is never longer than `1.0`, so a diagonal on the keys is not
	/// faster than a straight line. `Vec2::ZERO` for an action that does not
	/// exist.
	#[must_use]
	pub fn action_vector2(&self, name: &str) -> Vec2 {
		let Some(action) = self.vectors.get(name) else {
			return Vec2::ZERO;
		};
		let vector = Vec2::new(self.sum(&action.x), self.sum(&action.y));
		if action.normalize {
			vector.clamp_length_max(1.0)
		} else {
			vector
		}
	}

	/// Is the action in use: is **any** of its bindings held down (an axis
	/// pushed past the press threshold, in either direction)?
	#[must_use]
	pub fn is_action_pressed(&self, name: &str) -> bool {
		self.actions
			.get(name)
			.is_some_and(|action| self.any_active(&action.bindings))
	}

	/// Did the action become pressed in this frame? Noted when the press
	/// happened, so a tap that is over before this is asked still counts.
	#[must_use]
	pub fn is_action_just_pressed(&self, name: &str) -> bool {
		self.actions.get(name).is_some_and(|action| action.just_pressed)
	}

	/// Did the action stop being pressed in this frame?
	#[must_use]
	pub fn is_action_just_released(&self, name: &str) -> bool {
		self.actions.get(name).is_some_and(|action| action.just_released)
	}

	/// For how many seconds has the action been pressed? `None` if it is not.
	#[must_use]
	pub fn action_held_for(&self, name: &str) -> Option<f32> {
		let action = self.actions.get(name)?;
		action.since.map(|since| (self.time - since) as f32)
	}

	/// Was the action pressed within the last `seconds`, and not used up with
	/// [`Self::consume_action`] since? A jump pressed a moment before landing
	/// still counts: ask `action_pressed_within("Jump", 0.15)` when you land.
	#[must_use]
	pub fn action_pressed_within(&self, name: &str, seconds: f32) -> bool {
		self.actions
			.get(name)
			.and_then(|action| action.pressed_at)
			.is_some_and(|at| self.time - at <= f64::from(seconds))
	}

	/// Uses up the last press of the action, so that `action_pressed_within`
	/// does not report it again (until the action is pressed again).
	pub fn consume_action(&mut self, name: &str) {
		if let Some(action) = self.actions.get_mut(name) {
			action.pressed_at = None;
		}
	}

	// --- changing bindings --------------------------------------------------

	/// Replaces all the bindings of an existing action (an option menu).
	///
	/// # Errors
	/// `InvalidSettings` if there is no such action.
	pub fn set_bindings(&mut self, name: &str, bindings: impl IntoIterator<Item = Binding>) -> ZResult<()> {
		*self.bindings_mut(name)? = bindings.into_iter().collect();
		self.resync_action(name);
		Ok(())
	}

	/// Replaces binding `index` of an existing action.
	///
	/// # Errors
	/// `InvalidSettings` if there is no such action or binding.
	pub fn rebind(&mut self, name: &str, index: usize, binding: Binding) -> ZResult<()> {
		let slot = self
			.bindings_mut(name)?
			.get_mut(index)
			.ok_or_else(|| ZenithError::InvalidSettings(format!("the action {name:?} has no binding {index}")))?;
		*slot = binding;
		self.resync_action(name);
		Ok(())
	}

	/// Adds a binding to an existing action.
	///
	/// # Errors
	/// `InvalidSettings` if there is no such action.
	pub fn add_binding(&mut self, name: &str, binding: Binding) -> ZResult<()> {
		self.bindings_mut(name)?.push(binding);
		self.resync_action(name);
		Ok(())
	}

	/// Removes binding `index` of an existing action and returns it.
	///
	/// # Errors
	/// `InvalidSettings` if there is no such action or binding.
	pub fn remove_binding(&mut self, name: &str, index: usize) -> ZResult<Binding> {
		let bindings = self.bindings_mut(name)?;
		if index >= bindings.len() {
			return Err(ZenithError::InvalidSettings(format!(
				"the action {name:?} has no binding {index}"
			)));
		}
		let removed = bindings.remove(index);
		self.resync_action(name);
		Ok(removed)
	}

	fn bindings_mut(&mut self, name: &str) -> ZResult<&mut Vec<Binding>> {
		self.actions
			.get_mut(name)
			.map(|action| &mut action.bindings)
			.ok_or_else(|| ZenithError::InvalidSettings(format!("there is no action {name:?}")))
	}

	/// Puts an action (button or vector) back to what it was defined as.
	/// `false` if there is no such action.
	pub fn reset_action(&mut self, name: &str) -> bool {
		if let Some(default) = self.default_actions.get(name).cloned() {
			if let Some(action) = self.actions.get_mut(name) {
				action.bindings = default;
			}
			self.resync_action(name);
			true
		} else if let Some(default) = self.default_vectors.get(name) {
			self.vectors.insert(name.to_string(), default.clone());
			true
		} else {
			false
		}
	}

	/// Waits for the player's next press (a key, a mouse button, a gamepad
	/// button, or an axis pushed far) and keeps it as a binding, for "press
	/// the key you want for Jump". Pick it up with [`Self::take_captured`].
	/// Hold Ctrl, Shift, Alt or Super first for a chord (`Ctrl+S`); a modifier
	/// pressed and let go on its own is taken as it is. The press still counts
	/// for the game too, so listen in a menu.
	pub fn listen_for_binding(&mut self) {
		self.listening = true;
		self.listen_modifiers.clear();
		self.captured = None;
	}

	/// Are we waiting for a press?
	#[must_use]
	pub const fn is_listening(&self) -> bool {
		self.listening
	}

	/// Stops waiting.
	pub fn cancel_listening(&mut self) {
		self.listening = false;
		self.listen_modifiers.clear();
		self.captured = None;
	}

	/// The binding the player pressed, once, since `listen_for_binding`.
	pub const fn take_captured(&mut self) -> Option<Binding> {
		self.captured.take()
	}

	/// The actions the player changed, as the text of a file (the same format
	/// as `Input.toml`, with only the actions that differ from how they were
	/// defined). Keep it per player, not in the project; `load_overrides` puts
	/// it back.
	#[must_use]
	pub fn overrides_toml(&self) -> String {
		let mut out = String::new();
		for (name, action) in &self.actions {
			if self.default_actions.get(name) != Some(&action.bindings) {
				out.push_str(&action_to_toml(name, &action.bindings));
			}
		}
		for (name, vector) in &self.vectors {
			if self.default_vectors.get(name) != Some(vector) {
				out.push_str(&vector_to_toml(name, vector));
			}
		}
		out
	}

	/// Applies a file written by [`Self::overrides_toml`]. Actions the game
	/// does not have (the file may be older than the game) are skipped, and
	/// the settings of the file are not used. Nothing changes if the file is
	/// bad.
	///
	/// # Errors
	/// `InvalidSettings` for a file that cannot be read.
	pub fn load_overrides(&mut self, text: &str) -> ZResult<()> {
		let file = InputFile::parse(text)?;
		for (name, bindings) in file.actions {
			if let Some(action) = self.actions.get_mut(&name) {
				action.bindings = bindings;
				self.resync_action(&name);
			}
		}
		for (name, vector) in file.vectors {
			if let Some(slot) = self.vectors.get_mut(&name) {
				*slot = vector;
			}
		}
		Ok(())
	}

	// --- settings -----------------------------------------------------------

	#[must_use]
	pub const fn settings(&self) -> &InputSettings {
		&self.settings
	}

	pub const fn settings_mut(&mut self) -> &mut InputSettings {
		&mut self.settings
	}

	/// Takes the settings and the actions of an `Input.toml` (see
	/// [`InputFile`]). Actions with the same names are replaced (and become the
	/// defaults to reset to), others stay. Read the text yourself, with
	/// `Project::read_string("Settings://Input.toml")`.
	///
	/// # Errors
	/// `InvalidSettings` for a file that cannot be used; nothing is changed
	/// then.
	pub fn load_settings(&mut self, text: &str) -> ZResult<()> {
		let file = InputFile::parse(text)?;
		self.settings = file.settings;
		for (name, bindings) in file.actions {
			self.add_action(name, bindings);
		}
		for (name, vector) in file.vectors {
			self.add_vector_action(name, vector);
		}
		Ok(())
	}

	// --- recording ----------------------------------------------------------

	/// Starts writing down every event, frame by frame (a recording already
	/// under way is dropped).
	pub fn start_recording(&mut self) {
		self.recorder = Some(Recorder::default());
	}

	#[must_use]
	pub const fn is_recording(&self) -> bool {
		self.recorder.is_some()
	}

	/// Stops recording and returns what happened. Events of the frame that has
	/// not ended yet become a last frame of zero length. An empty recording if
	/// none was under way.
	pub fn stop_recording(&mut self) -> Recording {
		let Some(mut recorder) = self.recorder.take() else {
			return Recording::default();
		};
		if !recorder.events.is_empty() {
			recorder.frames.push(RecordedFrame {
				dt: 0.0,
				events: mem::take(&mut recorder.events),
			});
		}
		Recording::new(recorder.frames)
	}
}

#[cfg(test)]
mod tests {
	use super::*;

	const FRAME: f32 = 0.1;

	fn key_down(input: &mut Input, key: ZKeyCode) {
		input.apply(InputEvent::KeyDown(key));
	}

	fn stick(input: &mut Input, x: f32, y: f32) {
		input.apply(InputEvent::GamepadAxis {
			id: 0,
			axis: ZGamePadAxis::LeftStickX,
			value: x,
		});
		input.apply(InputEvent::GamepadAxis {
			id: 0,
			axis: ZGamePadAxis::LeftStickY,
			value: y,
		});
	}

	fn pad(ff_supported: bool) -> InputEvent {
		InputEvent::GamepadConnected {
			id: 0,
			name: "Pad".into(),
			kind: ZGamepadKind::Xbox,
			ff_supported,
			mapped: true,
		}
	}

	fn near(a: f32, b: f32) -> bool {
		(a - b).abs() < 1e-4
	}

	#[test]
	fn a_press_lasts_a_frame_and_a_hold_lasts_until_release() {
		let mut input = Input::default();
		key_down(&mut input, ZKeyCode::W);
		assert!(input.is_key_pressed(ZKeyCode::W), "held");
		assert!(input.is_key_just_pressed(ZKeyCode::W), "new");
		input.end_frame(FRAME);
		assert!(input.is_key_pressed(ZKeyCode::W), "still held");
		assert!(!input.is_key_just_pressed(ZKeyCode::W), "no longer new");
		input.apply(InputEvent::KeyUp(ZKeyCode::W));
		assert!(!input.is_key_pressed(ZKeyCode::W), "let go");
		assert!(input.is_key_just_released(ZKeyCode::W), "just let go");
		input.end_frame(FRAME);
		assert!(!input.is_key_just_released(ZKeyCode::W), "the release lasts one frame");
	}

	#[test]
	fn a_repeating_key_is_not_pressed_again() {
		let mut input = Input::default();
		key_down(&mut input, ZKeyCode::A);
		input.end_frame(FRAME);
		key_down(&mut input, ZKeyCode::A);
		assert!(
			!input.is_key_just_pressed(ZKeyCode::A),
			"auto-repeat is not a new press"
		);
	}

	#[test]
	fn a_tap_inside_one_frame_is_seen_as_both() {
		let mut input = Input::default();
		key_down(&mut input, ZKeyCode::F);
		input.apply(InputEvent::KeyUp(ZKeyCode::F));
		assert!(input.is_key_just_pressed(ZKeyCode::F), "pressed");
		assert!(input.is_key_just_released(ZKeyCode::F), "and released");
		assert!(!input.is_key_pressed(ZKeyCode::F), "not held");
	}

	#[test]
	fn losing_the_focus_lets_go_of_everything() {
		let mut input = Input::default();
		key_down(&mut input, ZKeyCode::W);
		input.apply(InputEvent::MouseDown(ZMouseButton::Left));
		input.end_frame(FRAME);
		input.apply(InputEvent::Focus(false));
		assert!(!input.is_key_pressed(ZKeyCode::W), "key released");
		assert!(input.is_key_just_released(ZKeyCode::W), "and seen as released");
		assert!(!input.is_mouse_pressed(ZMouseButton::Left), "button released");
		assert!(!input.is_focused(), "no focus");
		assert_eq!(input.key_held_for(ZKeyCode::W), None, "nothing is held");
	}

	#[test]
	fn the_mouse_moves_and_the_wheel_turns() {
		let mut input = Input::default();
		assert_eq!(input.mouse_position(), None, "no position yet");
		input.apply(InputEvent::MouseMoved(Vec2::new(10.0, 10.0)));
		input.apply(InputEvent::MouseMoved(Vec2::new(14.0, 7.0)));
		assert_eq!(input.mouse_position(), Some(Vec2::new(14.0, 7.0)), "position");
		assert_eq!(
			input.mouse_delta(),
			Vec2::new(4.0, -3.0),
			"the first position is no movement"
		);
		input.apply(InputEvent::Scroll(Vec2::new(0.0, 1.0)));
		input.apply(InputEvent::Scroll(Vec2::new(0.0, 2.0)));
		assert_eq!(input.scroll(), Vec2::new(0.0, 3.0), "scroll adds up");
		input.end_frame(FRAME);
		assert_eq!(input.mouse_delta(), Vec2::ZERO, "movement is per frame");
		assert_eq!(input.scroll(), Vec2::ZERO, "so is scrolling");
		assert_eq!(input.mouse_position(), Some(Vec2::new(14.0, 7.0)), "the position stays");
		input.apply(InputEvent::MouseLeft);
		assert_eq!(input.mouse_position(), None, "the cursor left");
	}

	#[test]
	fn the_mouse_delta_is_in_pixels_until_the_cursor_is_locked() -> ZResult<()> {
		let mut input = Input::default();
		input.load_settings("[mouse]\nsensitivity = 2.0\ninvert_y = true\n")?;
		input.apply(InputEvent::MouseMoved(Vec2::ZERO));
		input.apply(InputEvent::MouseMoved(Vec2::new(10.0, 10.0)));
		input.apply(InputEvent::MouseMotion(Vec2::new(1.0, 2.0)));
		assert_eq!(
			input.mouse_delta(),
			Vec2::new(20.0, -20.0),
			"an ordinary cursor: its pixels, doubled, upside down; the raw motion is not mixed in"
		);
		input.set_cursor_mode(CursorMode::Locked);
		assert_eq!(
			input.mouse_delta(),
			Vec2::new(2.0, -4.0),
			"a locked cursor does not move, so the raw motion is used"
		);
		Ok(())
	}

	#[test]
	fn text_is_collected_without_control_characters() {
		let mut input = Input::default();
		input.apply(InputEvent::Text("héllo".into()));
		input.apply(InputEvent::Text("\r\u{7f}!".into()));
		assert_eq!(input.text(), "héllo!", "typed characters only");
		input.end_frame(FRAME);
		assert_eq!(input.text(), "", "per frame");
	}

	#[test]
	fn gamepads_come_and_go() {
		let mut input = Input::default();
		assert!(input.list_gamepads().is_empty(), "none at first");
		assert!(input.gamepad(0).is_none(), "no object for a pad that is not there");
		input.apply(pad(false));
		input.apply(InputEvent::GamepadButton {
			id: 0,
			button: ZGamePad::South,
			pressed: true,
		});
		assert_eq!(
			input.list_gamepads(),
			vec![GamepadInfo {
				id: 0,
				name: "Pad".into(),
				kind: ZGamepadKind::Xbox
			}],
			"listed"
		);
		let gamepad = input.gamepad(0);
		assert!(gamepad.is_some_and(|pad| pad.is_pressed(ZGamePad::South)), "held");
		assert!(gamepad.is_some_and(|pad| pad.is_just_pressed(ZGamePad::South)), "new");
		assert!(input.is_gamepad_pressed(ZGamePad::South, 0), "the short form agrees");
		assert!(!input.is_gamepad_pressed(ZGamePad::South, 1), "another pad");
		input.end_frame(FRAME);
		assert!(!input.is_gamepad_just_pressed(ZGamePad::South, 0), "no longer new");
		input.apply(InputEvent::GamepadDisconnected { id: 0 });
		assert!(!input.is_gamepad_pressed(ZGamePad::South, 0), "gone with the pad");
		assert!(input.list_gamepads().is_empty(), "none left");
	}

	#[test]
	fn a_gamepad_tells_about_itself() {
		let mut input = Input::default();
		input.apply(pad(true));
		input.apply(InputEvent::GamepadPower {
			id: 0,
			power: ZPowerInfo::Discharging(40),
		});
		let gamepad = input.gamepad(0);
		assert_eq!(gamepad.map(|pad| pad.name().to_string()), Some("Pad".into()), "name");
		assert_eq!(gamepad.map(|pad| pad.kind()), Some(ZGamepadKind::Xbox), "kind");
		assert_eq!(
			gamepad.map(|pad| pad.power_info()),
			Some(ZPowerInfo::Discharging(40)),
			"battery"
		);
		assert_eq!(gamepad.map(|pad| pad.is_ff_supported()), Some(true), "can vibrate");
		assert_eq!(
			gamepad.map(|pad| pad.has_standard_mapping()),
			Some(true),
			"standard layout"
		);
		assert_eq!(gamepad.map(|pad| pad.id()), Some(0), "number");
	}

	#[test]
	fn an_emulated_gamepad_needs_no_connect_event() {
		let mut input = Input::default();
		input.apply(InputEvent::GamepadAxis {
			id: 2,
			axis: ZGamePadAxis::LeftTrigger,
			value: 1.0,
		});
		assert_eq!(input.list_gamepads().len(), 1, "made by its first event");
		assert!(
			near(input.gamepad_axis(ZGamePadAxis::LeftTrigger, 2), 1.0),
			"reads back"
		);
		assert_eq!(
			input.gamepad(2).map(|pad| pad.power_info()),
			Some(ZPowerInfo::Unknown),
			"nothing is known about its battery"
		);
	}

	#[test]
	fn rumble_is_refused_when_the_pad_cannot() -> ZResult<()> {
		let mut input = Input::default();
		let effect = RumbleEffect::new().strong(0.5).for_ms(100);
		assert!(
			matches!(input.rumble(0, effect), Err(ZenithError::Unsupported(_))),
			"no such pad"
		);
		input.apply(pad(false));
		assert!(
			matches!(input.rumble(0, effect), Err(ZenithError::Unsupported(_))),
			"a pad that cannot vibrate"
		);
		assert!(
			input.gamepad_mut(0).is_some_and(|mut pad| pad.rumble(effect).is_err()),
			"the object says the same"
		);
		assert!(input.take_pad_commands().is_empty(), "nothing was asked of the pad");

		input.apply(pad(true));
		assert!(
			matches!(
				input.rumble(0, RumbleEffect::new()),
				Err(ZenithError::InvalidSettings(_))
			),
			"nothing to play"
		);
		input.rumble(0, effect.strong(2.0))?;
		input.stop_rumble(0);
		input.stop_rumble(9);
		let commands = input.take_pad_commands();
		assert_eq!(
			commands.len(),
			2,
			"the rumble and the stop (not the one for a pad that is not there)"
		);
		assert!(
			matches!(commands.first(), Some(PadCommand::Rumble { id: 0, effect }) if (effect.strong - 1.0).abs() < f32::EPSILON),
			"the strength is limited: {commands:?}"
		);
		assert!(input.take_pad_commands().is_empty(), "taken once");
		Ok(())
	}

	#[test]
	fn the_cursor_request_is_taken_once_per_change() {
		let mut input = Input::default();
		assert_eq!(input.take_cursor_mode(), None, "nothing asked");
		input.set_cursor_mode(CursorMode::Locked);
		input.set_cursor_mode(CursorMode::Locked);
		assert_eq!(input.cursor_mode(), CursorMode::Locked, "remembered");
		assert_eq!(input.take_cursor_mode(), Some(CursorMode::Locked), "asked");
		assert_eq!(input.take_cursor_mode(), None, "only once");
		input.set_cursor_mode(CursorMode::Normal);
		assert_eq!(input.take_cursor_mode(), Some(CursorMode::Normal), "a change back");
	}

	#[test]
	fn sticks_have_a_round_dead_zone_and_a_stretched_range() {
		let mut input = Input::default();
		stick(&mut input, 0.1, 0.1);
		assert!(
			near(input.gamepad_axis(ZGamePadAxis::LeftStickX, 0), 0.0),
			"inside the dead zone"
		);
		stick(&mut input, 0.14, 0.14);
		assert!(
			input.gamepad_axis(ZGamePadAxis::LeftStickX, 0).abs() > 0.0,
			"0.14 on both axes is 0.198 long: outside the 0.15 zone, though each axis alone is inside"
		);
		stick(&mut input, 1.0, 0.0);
		assert!(
			near(input.gamepad_axis(ZGamePadAxis::LeftStickX, 0), 1.0),
			"full push is 1"
		);
		stick(&mut input, -0.575, 0.0);
		assert!(
			near(input.gamepad_axis(ZGamePadAxis::LeftStickX, 0), -0.5),
			"(0.575 - 0.15) / 0.85 = 0.5, with its sign"
		);
		stick(&mut input, 0.0, 0.0);
		input.apply(InputEvent::GamepadAxis {
			id: 0,
			axis: ZGamePadAxis::RightTrigger,
			value: 0.525,
		});
		assert!(
			near(input.gamepad_axis(ZGamePadAxis::RightTrigger, 0), 0.5),
			"(0.525 - 0.05) / 0.95 = 0.5"
		);
	}

	#[test]
	fn a_stick_is_a_vector_that_is_never_longer_than_one() {
		let mut input = Input::default();
		stick(&mut input, 0.0, -1.0);
		assert!(
			near(input.gamepad_stick(ZStick::Left, 0).distance(Vec2::new(0.0, -1.0)), 0.0),
			"down is (0, -1)"
		);
		stick(&mut input, 1.0, 1.0);
		let diagonal = input.gamepad_stick(ZStick::Left, 0);
		assert!(
			near(diagonal.length(), 1.0),
			"a corner of the square is on the circle: {diagonal}"
		);
		assert!(near(diagonal.x, diagonal.y), "and keeps its direction");
		assert_eq!(
			input.gamepad_stick(ZStick::Right, 0),
			Vec2::ZERO,
			"the other stick is at rest"
		);
		assert_eq!(input.gamepad_stick(ZStick::Left, 5), Vec2::ZERO, "no such pad");
	}

	#[test]
	fn the_response_curve_bends_the_push() -> ZResult<()> {
		let mut input = Input::default();
		input.load_settings("[gamepad]\ndead_zone = 0.0\ncurve = 2.0\n")?;
		stick(&mut input, 0.5, 0.0);
		assert!(
			near(input.gamepad_axis(ZGamePadAxis::LeftStickX, 0), 0.25),
			"0.5 squared"
		);
		stick(&mut input, 1.0, 0.0);
		assert!(
			near(input.gamepad_axis(ZGamePadAxis::LeftStickX, 0), 1.0),
			"the ends do not move"
		);
		Ok(())
	}

	#[test]
	fn an_action_is_pressed_when_any_binding_is() {
		let mut input = Input::default();
		input.add_action("Jump", [Binding::from(ZKeyCode::Space), Binding::from(ZGamePad::South)]);
		assert!(!input.is_action_pressed("Jump"), "nothing is down");
		key_down(&mut input, ZKeyCode::Space);
		assert!(input.is_action_pressed("Jump"), "by key");
		assert!(input.is_action_just_pressed("Jump"), "new");
		input.end_frame(FRAME);
		assert!(input.is_action_pressed("Jump"), "held");
		assert!(!input.is_action_just_pressed("Jump"), "not new");
		input.apply(InputEvent::KeyUp(ZKeyCode::Space));
		assert!(input.is_action_just_released("Jump"), "released");
		input.end_frame(FRAME);
		assert!(!input.is_action_just_released("Jump"), "for a frame");
		input.apply(InputEvent::GamepadButton {
			id: 1,
			button: ZGamePad::South,
			pressed: true,
		});
		assert!(input.is_action_pressed("Jump"), "by a button of any pad");
		assert!(
			!input.is_action_pressed("Nothing"),
			"an action that does not exist is not pressed"
		);
		assert!(near(input.action_value("Nothing"), 0.0), "and is 0");
	}

	#[test]
	fn four_keys_make_a_plain_yes_or_no() {
		let mut input = Input::default();
		input.add_action(
			"Cast",
			[
				Binding::from(ZKeyCode::W),
				Binding::from(ZKeyCode::S).negated(),
				Binding::from(ZKeyCode::T),
				Binding::from(ZKeyCode::B),
			],
		);
		for key in [ZKeyCode::W, ZKeyCode::S, ZKeyCode::T, ZKeyCode::B] {
			let mut input = input.clone();
			key_down(&mut input, key);
			assert!(input.is_action_pressed("Cast"), "{key} alone is enough");
		}
		key_down(&mut input, ZKeyCode::W);
		key_down(&mut input, ZKeyCode::S);
		assert!(
			input.is_action_pressed("Cast"),
			"W and a negated S cancel out as a value but the action is still pressed"
		);
		assert!(near(input.action_value("Cast"), 0.0), "as a value they cancel");
	}

	#[test]
	fn an_action_value_is_the_sum_of_keys_and_a_stick() {
		let mut input = Input::default();
		input.add_action(
			"MoveRight",
			[
				Binding::from(ZKeyCode::D),
				Binding::from(ZKeyCode::A).negated(),
				Binding::from(ZGamePadAxis::LeftStickX),
			],
		);
		key_down(&mut input, ZKeyCode::D);
		assert!(near(input.action_value("MoveRight"), 1.0), "D");
		key_down(&mut input, ZKeyCode::A);
		assert!(near(input.action_value("MoveRight"), 0.0), "D and A cancel out");
		input.apply(InputEvent::KeyUp(ZKeyCode::D));
		assert!(near(input.action_value("MoveRight"), -1.0), "A");
		assert!(input.is_action_pressed("MoveRight"), "a negative value is pressed too");
		input.apply(InputEvent::KeyUp(ZKeyCode::A));
		stick(&mut input, 0.4, 0.0);
		assert!(
			near(input.action_value("MoveRight"), 0.294_117_65),
			"stick: (0.4 - 0.15) / 0.85"
		);
		assert!(!input.is_action_pressed("MoveRight"), "below the press threshold");
	}

	#[test]
	fn a_vector_action_walks() {
		let mut input = Input::default();
		input.add_vector_action(
			"Move",
			VectorAction {
				x: vec![Binding::from(ZKeyCode::D), Binding::from(ZKeyCode::A).negated()],
				y: vec![Binding::from(ZKeyCode::W), Binding::from(ZKeyCode::S).negated()],
				normalize: true,
			},
		);
		assert_eq!(input.action_vector2("Move"), Vec2::ZERO, "standing");
		key_down(&mut input, ZKeyCode::D);
		assert_eq!(input.action_vector2("Move"), Vec2::new(1.0, 0.0), "right");
		key_down(&mut input, ZKeyCode::W);
		let diagonal = input.action_vector2("Move");
		assert!(
			near(diagonal.length(), 1.0),
			"a diagonal is no longer than a straight line: {diagonal}"
		);
		key_down(&mut input, ZKeyCode::A);
		assert_eq!(input.action_vector2("Move"), Vec2::new(0.0, 1.0), "D and A cancel");
		assert_eq!(input.action_vector2("Nothing"), Vec2::ZERO, "no such action");

		if let Some(action) = input.vector_action_mut("Move") {
			action.normalize = false;
		}
		input.apply(InputEvent::KeyUp(ZKeyCode::A));
		assert_eq!(
			input.action_vector2("Move"),
			Vec2::new(1.0, 1.0),
			"each axis on its own without normalize"
		);
	}

	#[test]
	fn a_chord_needs_its_modifier() {
		let mut input = Input::default();
		input.add_action("Save", [Binding::from(ZKeyCode::S).with(ZKeyCode::LeftControl)]);
		key_down(&mut input, ZKeyCode::S);
		assert!(!input.is_action_pressed("Save"), "S alone is not Ctrl+S");
		key_down(&mut input, ZKeyCode::LeftControl);
		assert!(input.is_action_pressed("Save"), "with Ctrl it is");
	}

	#[test]
	fn how_long_a_key_or_action_has_been_held() {
		let mut input = Input::default();
		input.add_action("Charge", [Binding::from(ZKeyCode::E)]);
		assert_eq!(input.key_held_for(ZKeyCode::E), None, "not held");
		assert_eq!(input.action_held_for("Charge"), None, "not held");
		key_down(&mut input, ZKeyCode::E);
		assert!(near(input.key_held_for(ZKeyCode::E).unwrap_or(-1.0), 0.0), "just now");
		input.end_frame(0.25);
		input.end_frame(0.25);
		assert!(near(input.key_held_for(ZKeyCode::E).unwrap_or(-1.0), 0.5), "two frames");
		assert!(
			near(input.action_held_for("Charge").unwrap_or(-1.0), 0.5),
			"the action too"
		);
		input.apply(InputEvent::KeyUp(ZKeyCode::E));
		assert_eq!(input.key_held_for(ZKeyCode::E), None, "let go");
		assert!((input.time() - 0.5).abs() < 1e-9, "the clock");
	}

	#[test]
	fn a_press_is_remembered_for_a_moment_and_can_be_used_up() {
		let mut input = Input::default();
		input.add_action("Jump", [Binding::from(ZKeyCode::Space)]);
		key_down(&mut input, ZKeyCode::Space);
		assert!(input.action_pressed_within("Jump", 0.15), "this frame");
		input.end_frame(0.1);
		input.apply(InputEvent::KeyUp(ZKeyCode::Space));
		assert!(input.action_pressed_within("Jump", 0.15), "a tenth of a second ago");
		input.end_frame(0.1);
		assert!(!input.action_pressed_within("Jump", 0.15), "a fifth is too long ago");

		key_down(&mut input, ZKeyCode::Space);
		input.end_frame(0.05);
		input.apply(InputEvent::KeyUp(ZKeyCode::Space));
		assert!(input.action_pressed_within("Jump", 0.15), "remembered");
		input.consume_action("Jump");
		assert!(!input.action_pressed_within("Jump", 0.15), "used up");
		input.end_frame(0.05);
		key_down(&mut input, ZKeyCode::Space);
		assert!(input.action_pressed_within("Jump", 0.15), "a new press counts again");
	}

	#[test]
	fn bindings_can_be_changed_and_put_back() -> ZResult<()> {
		let mut input = Input::default();
		input.add_action("Jump", [Binding::from(ZKeyCode::Space), Binding::from(ZGamePad::South)]);
		assert_eq!(input.overrides_toml(), "", "nothing changed yet");

		input.rebind("Jump", 0, Binding::from(ZKeyCode::J))?;
		input.add_binding("Jump", Binding::from(ZMouseButton::Right))?;
		assert_eq!(
			input.action_bindings("Jump").map(<[Binding]>::len),
			Some(3),
			"three now"
		);
		key_down(&mut input, ZKeyCode::J);
		assert!(input.is_action_pressed("Jump"), "the new key works");
		let removed = input.remove_binding("Jump", 1)?;
		assert_eq!(removed, Binding::from(ZGamePad::South), "the pad button went");

		assert!(
			input.rebind("Jump", 9, Binding::from(ZKeyCode::J)).is_err(),
			"no such binding"
		);
		assert!(
			input.rebind("Nothing", 0, Binding::from(ZKeyCode::J)).is_err(),
			"no such action"
		);
		assert!(input.set_bindings("Nothing", []).is_err(), "no such action");
		assert!(input.remove_binding("Jump", 9).is_err(), "no such binding");

		let saved = input.overrides_toml();
		assert!(saved.contains("Key:J"), "the change is in the text: {saved}");

		let mut fresh = Input::default();
		fresh.add_action("Jump", [Binding::from(ZKeyCode::Space), Binding::from(ZGamePad::South)]);
		fresh.load_overrides(&saved)?;
		assert_eq!(
			fresh.action_bindings("Jump"),
			input.action_bindings("Jump"),
			"loaded back"
		);
		assert!(
			fresh.load_overrides("[[action]]\nname = \"Gone\"\n").is_ok(),
			"unknown actions are skipped"
		);
		assert!(fresh.load_overrides("[[action]]\n").is_err(), "a bad file is an error");

		assert!(fresh.reset_action("Jump"), "reset");
		assert_eq!(fresh.overrides_toml(), "", "back to the defaults");
		assert!(!fresh.reset_action("Nothing"), "no such action");
		Ok(())
	}

	#[test]
	fn a_tap_between_two_questions_is_not_lost_by_an_action() {
		let mut input = Input::default();
		input.add_action("Jump", [Binding::from(ZKeyCode::Space)]);
		// Down and up before anybody asks, at a low frame rate.
		key_down(&mut input, ZKeyCode::Space);
		input.apply(InputEvent::KeyUp(ZKeyCode::Space));
		assert!(!input.is_action_pressed("Jump"), "it is up again");
		assert!(input.is_action_just_pressed("Jump"), "but it was pressed");
		assert!(input.is_action_just_released("Jump"), "and released");
		assert!(input.action_pressed_within("Jump", 0.15), "and the buffer saw it");
		input.end_frame(FRAME);
		assert!(!input.is_action_just_pressed("Jump"), "for one frame only");
		assert!(!input.is_action_just_released("Jump"), "so is the release");
	}

	#[test]
	fn an_action_defined_while_its_key_is_held_is_not_a_new_press() {
		let mut input = Input::default();
		key_down(&mut input, ZKeyCode::Space);
		input.add_action("Jump", [Binding::from(ZKeyCode::Space)]);
		assert!(input.is_action_pressed("Jump"), "held");
		assert!(!input.is_action_just_pressed("Jump"), "not new: it was already down");
		input.rebind("Jump", 0, Binding::from(ZKeyCode::J)).ok();
		assert!(!input.is_action_pressed("Jump"), "bound to another key now");
		assert!(
			!input.is_action_just_released("Jump"),
			"changing bindings is not a release"
		);
	}

	#[test]
	fn a_long_session_keeps_frame_precision() {
		let mut input = Input::default();
		input.add_action("Charge", [Binding::from(ZKeyCode::E)]);
		// 40 hours at 60 frames per second would round every frame away in f32.
		for _ in 0..8_640_000 {
			input.end_frame(1.0 / 60.0);
		}
		key_down(&mut input, ZKeyCode::E);
		input.end_frame(1.0 / 60.0);
		let held = input.action_held_for("Charge").unwrap_or(-1.0);
		assert!((held - 1.0 / 60.0).abs() < 1e-4, "one frame is still one frame: {held}");
		let key = input.key_held_for(ZKeyCode::E).unwrap_or(-1.0);
		assert!((key - 1.0 / 60.0).abs() < 1e-4, "also for a key: {key}");
	}

	#[test]
	fn a_pad_that_is_unplugged_while_held_releases_its_buttons() {
		let mut input = Input::default();
		input.add_action("Jump", [Binding::from(ZGamePad::South)]);
		input.apply(pad(false));
		input.apply(InputEvent::GamepadButton {
			id: 0,
			button: ZGamePad::South,
			pressed: true,
		});
		input.end_frame(FRAME);
		input.apply(InputEvent::GamepadDisconnected { id: 0 });
		assert!(
			input.is_gamepad_just_released(ZGamePad::South, 0),
			"the button came up with the pad"
		);
		assert!(
			!input.is_gamepad_just_released(ZGamePad::Start, 0),
			"only the ones that were held"
		);
		assert!(input.is_action_just_released("Jump"), "the action noticed too");
		input.end_frame(FRAME);
		assert!(!input.is_gamepad_just_released(ZGamePad::South, 0), "for a frame");
	}

	#[test]
	fn bindings_can_belong_to_one_gamepad() {
		let mut input = Input::default();
		input.add_action("Player1Jump", [Binding::from(ZGamePad::South).on_pad(0)]);
		input.add_action("Player2Jump", [Binding::from(ZGamePad::South).on_pad(1)]);
		input.add_action("AnyJump", [Binding::from(ZGamePad::South)]);
		input.apply(InputEvent::GamepadButton {
			id: 1,
			button: ZGamePad::South,
			pressed: true,
		});
		assert!(!input.is_action_pressed("Player1Jump"), "not the first pad");
		assert!(input.is_action_pressed("Player2Jump"), "the second pad");
		assert!(input.is_action_pressed("AnyJump"), "any pad");

		input.add_action("Player2Move", [Binding::from(ZGamePadAxis::LeftStickX).on_pad(1)]);
		input.apply(InputEvent::GamepadAxis {
			id: 1,
			axis: ZGamePadAxis::LeftStickX,
			value: 1.0,
		});
		input.apply(InputEvent::GamepadAxis {
			id: 0,
			axis: ZGamePadAxis::LeftStickX,
			value: -1.0,
		});
		assert!(
			near(input.action_value("Player2Move"), 1.0),
			"only the second pad's stick"
		);
	}

	#[test]
	fn removing_an_action_leaves_nothing_behind() {
		let mut input = Input::default();
		input.add_action("Jump", [Binding::from(ZKeyCode::Space)]);
		input.add_vector_action("Move", VectorAction::default());
		key_down(&mut input, ZKeyCode::Space);
		assert!(input.remove_action("Jump"), "removed");
		assert!(!input.action_pressed_within("Jump", 10.0), "its buffer went with it");
		assert!(!input.is_action_just_pressed("Jump"), "and its edges");
		assert!(input.remove_vector_action("Move"), "a vector action can be removed too");
		assert!(!input.remove_vector_action("Move"), "only once");
		assert_eq!(input.overrides_toml(), "", "nothing is left to save");
	}

	#[test]
	fn a_chord_can_be_taken_as_a_binding() {
		let mut input = Input::default();
		input.listen_for_binding();
		key_down(&mut input, ZKeyCode::LeftControl);
		assert!(input.take_captured().is_none(), "a modifier alone waits");
		key_down(&mut input, ZKeyCode::S);
		assert_eq!(
			input.take_captured(),
			Some(Binding::from(ZKeyCode::S).with(ZKeyCode::LeftControl)),
			"Ctrl+S"
		);

		let mut input = Input::default();
		input.listen_for_binding();
		key_down(&mut input, ZKeyCode::LeftShift);
		input.apply(InputEvent::KeyUp(ZKeyCode::LeftShift));
		assert_eq!(
			input.take_captured(),
			Some(Binding::from(ZKeyCode::LeftShift)),
			"a modifier pressed and let go alone is the binding"
		);

		let mut input = Input::default();
		input.listen_for_binding();
		key_down(&mut input, ZKeyCode::LeftControl);
		key_down(&mut input, ZKeyCode::LeftShift);
		input.apply(InputEvent::MouseDown(ZMouseButton::Left));
		assert_eq!(
			input.take_captured(),
			Some(
				Binding::from(ZMouseButton::Left)
					.with(ZKeyCode::LeftControl)
					.with(ZKeyCode::LeftShift)
			),
			"Ctrl+Shift+click"
		);
	}

	#[test]
	fn the_next_press_can_be_taken_as_a_binding() {
		let mut input = Input::default();
		assert!(!input.is_listening(), "not listening");
		input.listen_for_binding();
		assert!(input.is_listening(), "listening");
		input.apply(InputEvent::MouseMoved(Vec2::new(1.0, 1.0)));
		input.apply(InputEvent::GamepadAxis {
			id: 0,
			axis: ZGamePadAxis::LeftStickX,
			value: 0.3,
		});
		assert!(
			input.take_captured().is_none(),
			"a move or a small push is not a choice"
		);
		input.apply(InputEvent::GamepadAxis {
			id: 0,
			axis: ZGamePadAxis::LeftStickX,
			value: -0.9,
		});
		assert_eq!(
			input.take_captured(),
			Some(Binding::from(ZGamePadAxis::LeftStickX).negated()),
			"a far push to the left"
		);
		assert!(!input.is_listening(), "it stops by itself");
		assert!(input.take_captured().is_none(), "taken once");

		input.listen_for_binding();
		key_down(&mut input, ZKeyCode::K);
		key_down(&mut input, ZKeyCode::L);
		assert_eq!(
			input.take_captured(),
			Some(Binding::from(ZKeyCode::K)),
			"the first press only"
		);
		input.listen_for_binding();
		input.cancel_listening();
		key_down(&mut input, ZKeyCode::M);
		assert!(input.take_captured().is_none(), "cancelled");
	}

	#[test]
	fn settings_and_actions_come_from_a_file() -> ZResult<()> {
		let mut input = Input::default();
		input.add_action("Old", [Binding::from(ZKeyCode::Q)]);
		input.load_settings(
			"[[action]]\nname = \"Fire\"\nbindings = [\"Mouse:Left\"]\n[[vector]]\nname = \"Look\"\nx = [\"Key:D\"]\n",
		)?;
		assert!(input.action_bindings("Fire").is_some(), "from the file");
		assert!(input.action_bindings("Old").is_some(), "others stay");
		assert_eq!(input.actions().collect::<Vec<_>>(), ["Fire", "Old"], "sorted");
		assert_eq!(input.vector_actions().collect::<Vec<_>>(), ["Look"], "vectors too");
		assert!(input.vector_action("Look").is_some(), "readable");
		input.apply(InputEvent::MouseDown(ZMouseButton::Left));
		assert!(input.is_action_pressed("Fire"), "bound to the mouse");

		assert!(
			input.load_settings("[mouse]\nsensitivity = -1\n").is_err(),
			"a bad file"
		);
		assert!(
			near(input.settings().mouse.sensitivity, 1.0),
			"is refused as a whole and changes nothing"
		);
		assert!(input.remove_action("Fire"), "removed");
		assert!(!input.remove_action("Fire"), "only once");
		Ok(())
	}
	#[test]
	fn a_consumed_press_stays_consumed_while_held() {
		let mut input = Input::default();
		input.add_action("Jump", [Binding::from(ZKeyCode::Space)]);
		key_down(&mut input, ZKeyCode::Space);
		assert!(input.action_pressed_within("Jump", 0.15), "new");
		input.consume_action("Jump");
		input.end_frame(0.016);
		assert!(!input.action_pressed_within("Jump", 0.15), "held is not a new press");
	}
}
