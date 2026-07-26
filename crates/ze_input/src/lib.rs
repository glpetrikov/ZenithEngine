#[repr(usize)]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ZKeyCode {
	Escape = 0,
	Space,
	Q,
	W,
	E,
	R,
	T,
	Y,
	U,
	I,
	O,
	P,
	A,
	S,
	D,
	F,
	G,
	H,
	J,
	K,
	L,
	Z,
	X,
	C,
	V,
	B,
	N,
	M,
	Enter,
	LCtrl,
	LShift,
	RCtrl,
	RShift,
	ArrowUp,
	ArrowDown,
	ArrowLeft,
	ArrowRight,
	K1,
	K2,
	K3,
	K4,
	K5,
	K6,
	K7,
	K8,
	K9,
	K0,
	KF1,
	KF2,
	KF3,
	KF4,
	KF5,
	KF6,
	KF7,
	KF8,
	KF9,
	KF10,
	KF11,
	KF12,
	LAlt,
	RAlt,
	LSuper,
	RSuper,
	CapsLock,
	Tab,
	Backspace,
	ContextMenu,
	Unknown = 511,
}

#[repr(usize)]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ZMouseCode {
	Left = 0,
	Right = 1,
	Middle = 2,
	Back = 3,
	Forward = 4,
	Other = 7,
}

pub struct Input {
	current_keys: [bool; 512],
	previous_keys: [bool; 512],

	current_mouse: [bool; 8],
	previous_mouse: [bool; 8],

	pub mouse_pos: Vec2,
	pub mouse_delta: Vec2,
	pub mouse_wheel_delta: f32,
}

impl Input {
	pub fn new() -> Self { Self::default() }

	// --- Updaters ---

	pub const fn set_key(&mut self, code: ZKeyCode, state: bool) { self.current_keys[code as usize] = state; }

	pub const fn set_mouse_button(&mut self, button: ZMouseCode, state: bool) {
		self.current_mouse[button as usize] = state;
	}

	pub const fn set_mouse_pos(&mut self, x: f32, y: f32) { self.mouse_pos = Vec2::new(x, y); }

	pub fn add_mouse_delta(&mut self, dx: f32, dy: f32) {
		self.mouse_delta.x += dx;
		self.mouse_delta.y += dy;
	}

	pub fn add_mouse_wheel_delta(&mut self, dy: f32) { self.mouse_wheel_delta += dy; }

	pub const fn late_update(&mut self) {
		self.previous_keys = self.current_keys;
		self.previous_mouse = self.current_mouse;
		self.mouse_delta = Vec2::new(0.0, 0.0);
		self.mouse_wheel_delta = 0.0;
	}

	pub const fn reset(&mut self) {
		self.current_keys = [false; 512];
		self.previous_keys = [false; 512];
		self.current_mouse = [false; 8];
		self.previous_mouse = [false; 8];
		self.mouse_wheel_delta = 0.0;
	}

	// --- Main Methods ---

	// Keyboard

	const fn key_pressed(&self, code: ZKeyCode) -> bool { self.current_keys[code as usize] }

	const fn key_just_pressed(&self, code: ZKeyCode) -> bool {
		self.current_keys[code as usize] && !self.previous_keys[code as usize]
	}

	const fn key_released(&self, key_code: ZKeyCode) -> bool { !self.current_keys[key_code as usize] }

	const fn key_just_released(&self, key_code: ZKeyCode) -> bool {
		!self.key_pressed(key_code) && self.previous_keys[key_code as usize]
	}

	// Mouse

	const fn is_button_pressed(&self, code: ZMouseCode) -> bool { self.current_mouse[code as usize] }

	const fn is_button_just_pressed(&self, code: ZMouseCode) -> bool {
		self.current_mouse[code as usize] && !self.previous_mouse[code as usize]
	}

	const fn is_button_released(&self, mouse_code: ZMouseCode) -> bool { !self.current_mouse[mouse_code as usize] }

	const fn is_button_just_released(&self, mouse_code: ZMouseCode) -> bool {
		!self.is_button_pressed(mouse_code) && self.previous_mouse[mouse_code as usize]
	}
}

// --- Global Access ---

// FIXME: change unwrap to handle

impl Input {
	// Keyboard

	pub fn is_key_pressed(key: ZKeyCode) -> bool {
		Self::global()
			.lock()
			.expect("Failed to get Global input")
			.key_pressed(key)
	}

	pub fn is_key_just_pressed(key: ZKeyCode) -> bool {
		Self::global()
			.lock()
			.expect("Failed to get Global input")
			.key_just_pressed(key)
	}

	pub fn is_key_released(key: ZKeyCode) -> bool {
		Self::global()
			.lock()
			.expect("Failed to get Global input")
			.key_released(key)
	}

	pub fn is_key_just_released(key: ZKeyCode) -> bool {
		Self::global()
			.lock()
			.expect("Failed to get Global input")
			.key_just_released(key)
	}

	// Mouse

	pub fn get_mouse_pos() -> Vec2 { Self::global().lock().expect("Failed to get Global input").mouse_pos }

	pub fn get_mouse_delta() -> Vec2 { Self::global().lock().expect("Failed to get Global input").mouse_delta }

	pub fn get_mouse_wheel_delta() -> f32 {
		Self::global()
			.lock()
			.expect("Failed to get Global input")
			.mouse_wheel_delta
	}

	pub fn is_mouse_button_pressed(button: ZMouseCode) -> bool {
		Self::global()
			.lock()
			.expect("Failed to get Global input")
			.is_button_pressed(button)
	}

	pub fn is_mouse_button_just_pressed(button: ZMouseCode) -> bool {
		Self::global()
			.lock()
			.expect("Failed to get Global input")
			.is_button_just_pressed(button)
	}

	pub fn is_mouse_button_released(button: ZMouseCode) -> bool {
		Self::global()
			.lock()
			.expect("Failed to get Global input")
			.is_button_released(button)
	}

	pub fn is_mouse_button_just_released(button: ZMouseCode) -> bool {
		Self::global()
			.lock()
			.expect("Failed to get Global input")
			.is_button_just_released(button)
	}
}

impl Default for Input {
	fn default() -> Self {
		Self {
			current_keys: [false; 512],
			previous_keys: [false; 512],
			current_mouse: [false; 8],
			previous_mouse: [false; 8],
			mouse_pos: Vec2::ZERO,
			mouse_delta: Vec2::ZERO,
			mouse_wheel_delta: 0.0,
		}
	}
}

use std::sync::{Mutex, OnceLock};

use ze_core::Vec2;

static INPUT_INSTANCE: OnceLock<Mutex<Input>> = OnceLock::new();

impl Input {
	pub fn global() -> &'static Mutex<Self> { INPUT_INSTANCE.get_or_init(|| Mutex::new(Self::new())) }

	pub fn update_globally<F>(f: F)
	where
		F: FnOnce(&mut Self),
	{
		let mut input = Self::global().lock().expect("Failed to get Global input");
		f(&mut input);
	}
}

#[cfg(feature = "winit")]
macro_rules! impl_from_winit_keycode {
    ($($winit:ident => $zero:ident),* $(,)?) => {
        #[cfg(feature = "winit")]
        impl From<winit::keyboard::KeyCode> for ZKeyCode {
            fn from(key: winit::keyboard::KeyCode) -> Self {
                match key {
                    $(winit::keyboard::KeyCode::$winit => ZKeyCode::$zero,)*
                    _ => ZKeyCode::Unknown,
                }
            }
        }
    };
}
#[cfg(feature = "winit")]
impl_from_winit_keycode! {
	Escape => Escape,
	Space => Space,
	KeyQ => Q,
	KeyW => W,
	KeyE => E,
	KeyR => R,
	KeyT => T,
	KeyY => Y,
	KeyU => U,
	KeyI => I,
	KeyO => O,
	KeyP => P,
	KeyA => A,
	KeyS => S,
	KeyD => D,
	KeyF => F,
	KeyG => G,
	KeyH => H,
	KeyJ => J,
	KeyK => K,
	KeyL => L,
	KeyZ => Z,
	KeyX => X,
	KeyC => C,
	KeyV => V,
	KeyB => B,
	KeyN => N,
	KeyM => M,
	Enter => Enter,
	ControlLeft => LCtrl,
	ShiftLeft => LShift,
	ControlRight => RCtrl,
	ShiftRight => RShift,
	AltLeft => LAlt,
	AltRight => RAlt,
	SuperLeft => LSuper,
	SuperRight => RSuper,
	CapsLock => CapsLock,
	Tab => Tab,
	Backspace => Backspace,
	ContextMenu => ContextMenu,
	ArrowUp => ArrowUp,
	ArrowDown => ArrowDown,
	ArrowLeft => ArrowLeft,
	ArrowRight => ArrowRight,
	Digit1 => K1,
	Digit2 => K2,
	Digit3 => K3,
	Digit4 => K4,
	Digit5 => K5,
	Digit6 => K6,
	Digit7 => K7,
	Digit8 => K8,
	Digit9 => K9,
	Digit0 => K0,
	F1 => KF1,
	F2 => KF2,
	F3 => KF3,
	F4 => KF4,
	F5 => KF5,
	F6 => KF6,
	F7 => KF7,
	F8 => KF8,
	F9 => KF9,
	F10 => KF10,
	F11 => KF11,
	F12 => KF12,
}

#[cfg(feature = "winit")]
impl From<winit::event::MouseButton> for ZMouseCode {
	fn from(button: winit::event::MouseButton) -> Self {
		match button {
			winit::event::MouseButton::Left => Self::Left,
			winit::event::MouseButton::Right => Self::Right,
			winit::event::MouseButton::Middle => Self::Middle,
			winit::event::MouseButton::Back => Self::Back,
			winit::event::MouseButton::Forward => Self::Forward,
			winit::event::MouseButton::Other(_) => Self::Other,
		}
	}
}

#[cfg(test)]
mod tests {
	use super::*;

	fn make_input() -> Input { Input::new() }

	#[test]
	fn test_key_just_pressed() {
		let mut input = make_input();
		input.set_key(ZKeyCode::Space, true);
		assert!(input.key_just_pressed(ZKeyCode::Space));
		assert!(input.key_pressed(ZKeyCode::Space));
	}

	#[test]
	fn test_key_just_released() {
		let mut input = make_input();
		input.set_key(ZKeyCode::Space, true);
		input.late_update();
		input.set_key(ZKeyCode::Space, false);
		assert!(input.key_just_released(ZKeyCode::Space));
		assert!(!input.key_pressed(ZKeyCode::Space));
	}

	#[test]
	fn test_key_held() {
		let mut input = make_input();
		input.set_key(ZKeyCode::W, true);
		input.late_update();
		// Held - pressed but not just pressed
		assert!(input.key_pressed(ZKeyCode::W));
		assert!(!input.key_just_pressed(ZKeyCode::W));
	}

	#[test]
	fn test_mouse_just_pressed() {
		let mut input = make_input();
		input.set_mouse_button(ZMouseCode::Left, true);
		assert!(input.is_button_just_pressed(ZMouseCode::Left));
	}

	#[test]
	fn test_late_update_clears_just_pressed() {
		let mut input = make_input();
		input.set_key(ZKeyCode::Enter, true);
		input.late_update();
		assert!(!input.key_just_pressed(ZKeyCode::Enter));
	}

	#[cfg(feature = "winit")]
	#[test]
	fn winit_modifier_keys_map_to_engine_keys() {
		assert_eq!(ZKeyCode::from(winit::keyboard::KeyCode::ShiftLeft), ZKeyCode::LShift);
		assert_eq!(ZKeyCode::from(winit::keyboard::KeyCode::ShiftRight), ZKeyCode::RShift);
		assert_eq!(ZKeyCode::from(winit::keyboard::KeyCode::ControlLeft), ZKeyCode::LCtrl);
		assert_eq!(ZKeyCode::from(winit::keyboard::KeyCode::ControlRight), ZKeyCode::RCtrl);
		assert_eq!(ZKeyCode::from(winit::keyboard::KeyCode::AltLeft), ZKeyCode::LAlt);
		assert_eq!(ZKeyCode::from(winit::keyboard::KeyCode::AltRight), ZKeyCode::RAlt);
		assert_eq!(ZKeyCode::from(winit::keyboard::KeyCode::SuperLeft), ZKeyCode::LSuper);
		assert_eq!(ZKeyCode::from(winit::keyboard::KeyCode::SuperRight), ZKeyCode::RSuper);
		assert_eq!(
			ZKeyCode::from(winit::keyboard::KeyCode::ContextMenu),
			ZKeyCode::ContextMenu
		);
	}
}
