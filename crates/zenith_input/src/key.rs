//! Physical keys.

use std::{fmt, str::FromStr};

use serde::{Deserialize, Deserializer, Serialize, Serializer};
use zenith_error::ZenithError;

use crate::names::normalize;

/// Other names that are accepted when a key is read from text.
const ALIASES: &[(&str, ZKeyCode)] = &[
	("return", ZKeyCode::Enter),
	("esc", ZKeyCode::Escape),
	("ctrl", ZKeyCode::LeftControl),
	("control", ZKeyCode::LeftControl),
	("shift", ZKeyCode::LeftShift),
	("alt", ZKeyCode::LeftAlt),
	("win", ZKeyCode::LeftSuper),
	("cmd", ZKeyCode::LeftSuper),
	("command", ZKeyCode::LeftSuper),
	("del", ZKeyCode::Delete),
	("pgup", ZKeyCode::PageUp),
	("pgdn", ZKeyCode::PageDown),
	("arrowup", ZKeyCode::Up),
	("arrowdown", ZKeyCode::Down),
	("arrowleft", ZKeyCode::Left),
	("arrowright", ZKeyCode::Right),
];

macro_rules! keys {
	($($name:ident => $winit:ident,)*) => {
		/// A physical key: the place on the keyboard, not the letter it types.
		///
		/// `ZKeyCode::W` is the key in the `W` position of a US keyboard, which is
		/// the one under the finger of a player on any layout. The list is the one
		/// of `winit` (`KeyCode`), with some names made friendlier (`LeftShift`,
		/// `Up`, `A`).
		#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
		pub enum ZKeyCode {
			$($name,)*
		}

		impl ZKeyCode {
			/// Every key, in the order of the declaration.
			pub const ALL: &'static [Self] = &[$(Self::$name,)*];

			/// The name of the key: `W`, `Space`, `LeftShift`.
			#[must_use]
			pub const fn name(self) -> &'static str {
				match self {
					$(Self::$name => stringify!($name),)*
				}
			}

			/// The name `winit` gives the key (`KeyW`, `ShiftLeft`); accepted when
			/// reading text too.
			const fn winit_name(self) -> &'static str {
				match self {
					$(Self::$name => stringify!($winit),)*
				}
			}

			/// The key for a `winit` key code; `None` for codes this list does not
			/// have (`winit` may add some).
			#[cfg(feature = "winit")]
			#[must_use]
			pub const fn from_winit(code: winit::keyboard::KeyCode) -> Option<Self> {
				use winit::keyboard::KeyCode as K;
				match code {
					$(K::$winit => Some(Self::$name),)*
					_ => None,
				}
			}

			/// The `winit` key code of the key.
			#[cfg(feature = "winit")]
			#[must_use]
			pub const fn to_winit(self) -> winit::keyboard::KeyCode {
				use winit::keyboard::KeyCode as K;
				match self {
					$(Self::$name => K::$winit,)*
				}
			}
		}
	};
}

keys! {
	Backquote => Backquote,
	Backslash => Backslash,
	LeftBracket => BracketLeft,
	RightBracket => BracketRight,
	Comma => Comma,
	Digit0 => Digit0,
	Digit1 => Digit1,
	Digit2 => Digit2,
	Digit3 => Digit3,
	Digit4 => Digit4,
	Digit5 => Digit5,
	Digit6 => Digit6,
	Digit7 => Digit7,
	Digit8 => Digit8,
	Digit9 => Digit9,
	Equal => Equal,
	IntlBackslash => IntlBackslash,
	IntlRo => IntlRo,
	IntlYen => IntlYen,
	A => KeyA,
	B => KeyB,
	C => KeyC,
	D => KeyD,
	E => KeyE,
	F => KeyF,
	G => KeyG,
	H => KeyH,
	I => KeyI,
	J => KeyJ,
	K => KeyK,
	L => KeyL,
	M => KeyM,
	N => KeyN,
	O => KeyO,
	P => KeyP,
	Q => KeyQ,
	R => KeyR,
	S => KeyS,
	T => KeyT,
	U => KeyU,
	V => KeyV,
	W => KeyW,
	X => KeyX,
	Y => KeyY,
	Z => KeyZ,
	Minus => Minus,
	Period => Period,
	Quote => Quote,
	Semicolon => Semicolon,
	Slash => Slash,
	LeftAlt => AltLeft,
	RightAlt => AltRight,
	Backspace => Backspace,
	CapsLock => CapsLock,
	ContextMenu => ContextMenu,
	LeftControl => ControlLeft,
	RightControl => ControlRight,
	Enter => Enter,
	LeftSuper => SuperLeft,
	RightSuper => SuperRight,
	LeftShift => ShiftLeft,
	RightShift => ShiftRight,
	Space => Space,
	Tab => Tab,
	Convert => Convert,
	KanaMode => KanaMode,
	Lang1 => Lang1,
	Lang2 => Lang2,
	Lang3 => Lang3,
	Lang4 => Lang4,
	Lang5 => Lang5,
	NonConvert => NonConvert,
	Delete => Delete,
	End => End,
	Help => Help,
	Home => Home,
	Insert => Insert,
	PageDown => PageDown,
	PageUp => PageUp,
	Down => ArrowDown,
	Left => ArrowLeft,
	Right => ArrowRight,
	Up => ArrowUp,
	NumLock => NumLock,
	Numpad0 => Numpad0,
	Numpad1 => Numpad1,
	Numpad2 => Numpad2,
	Numpad3 => Numpad3,
	Numpad4 => Numpad4,
	Numpad5 => Numpad5,
	Numpad6 => Numpad6,
	Numpad7 => Numpad7,
	Numpad8 => Numpad8,
	Numpad9 => Numpad9,
	NumpadAdd => NumpadAdd,
	NumpadBackspace => NumpadBackspace,
	NumpadClear => NumpadClear,
	NumpadClearEntry => NumpadClearEntry,
	NumpadComma => NumpadComma,
	NumpadDecimal => NumpadDecimal,
	NumpadDivide => NumpadDivide,
	NumpadEnter => NumpadEnter,
	NumpadEqual => NumpadEqual,
	NumpadHash => NumpadHash,
	NumpadMemoryAdd => NumpadMemoryAdd,
	NumpadMemoryClear => NumpadMemoryClear,
	NumpadMemoryRecall => NumpadMemoryRecall,
	NumpadMemoryStore => NumpadMemoryStore,
	NumpadMemorySubtract => NumpadMemorySubtract,
	NumpadMultiply => NumpadMultiply,
	NumpadLeftParen => NumpadParenLeft,
	NumpadRightParen => NumpadParenRight,
	NumpadStar => NumpadStar,
	NumpadSubtract => NumpadSubtract,
	Escape => Escape,
	Fn => Fn,
	FnLock => FnLock,
	PrintScreen => PrintScreen,
	ScrollLock => ScrollLock,
	Pause => Pause,
	BrowserBack => BrowserBack,
	BrowserFavorites => BrowserFavorites,
	BrowserForward => BrowserForward,
	BrowserHome => BrowserHome,
	BrowserRefresh => BrowserRefresh,
	BrowserSearch => BrowserSearch,
	BrowserStop => BrowserStop,
	Eject => Eject,
	LaunchApp1 => LaunchApp1,
	LaunchApp2 => LaunchApp2,
	LaunchMail => LaunchMail,
	MediaPlayPause => MediaPlayPause,
	MediaSelect => MediaSelect,
	MediaStop => MediaStop,
	MediaTrackNext => MediaTrackNext,
	MediaTrackPrevious => MediaTrackPrevious,
	Power => Power,
	Sleep => Sleep,
	AudioVolumeDown => AudioVolumeDown,
	AudioVolumeMute => AudioVolumeMute,
	AudioVolumeUp => AudioVolumeUp,
	WakeUp => WakeUp,
	Meta => Meta,
	Hyper => Hyper,
	Turbo => Turbo,
	Abort => Abort,
	Resume => Resume,
	Suspend => Suspend,
	Again => Again,
	Copy => Copy,
	Cut => Cut,
	Find => Find,
	Open => Open,
	Paste => Paste,
	Props => Props,
	Select => Select,
	Undo => Undo,
	Hiragana => Hiragana,
	Katakana => Katakana,
	F1 => F1,
	F2 => F2,
	F3 => F3,
	F4 => F4,
	F5 => F5,
	F6 => F6,
	F7 => F7,
	F8 => F8,
	F9 => F9,
	F10 => F10,
	F11 => F11,
	F12 => F12,
	F13 => F13,
	F14 => F14,
	F15 => F15,
	F16 => F16,
	F17 => F17,
	F18 => F18,
	F19 => F19,
	F20 => F20,
	F21 => F21,
	F22 => F22,
	F23 => F23,
	F24 => F24,
	F25 => F25,
	F26 => F26,
	F27 => F27,
	F28 => F28,
	F29 => F29,
	F30 => F30,
	F31 => F31,
	F32 => F32,
	F33 => F33,
	F34 => F34,
	F35 => F35,
}

impl fmt::Display for ZKeyCode {
	fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
		f.write_str(self.name())
	}
}

impl FromStr for ZKeyCode {
	type Err = ZenithError;

	/// Reads a key by its name, in any case and with or without `_`, `-` and
	/// spaces; `winit` names (`KeyW`, `ShiftLeft`) and a few short names
	/// (`Esc`, `Ctrl`, `Shift`) work too.
	fn from_str(text: &str) -> Result<Self, Self::Err> {
		let wanted = normalize(text);
		Self::ALL
			.iter()
			.copied()
			.find(|key| normalize(key.name()) == wanted)
			.or_else(|| ALIASES.iter().find(|(alias, _)| *alias == wanted).map(|(_, key)| *key))
			.or_else(|| {
				Self::ALL
					.iter()
					.copied()
					.find(|key| normalize(key.winit_name()) == wanted)
			})
			.ok_or_else(|| ZenithError::InvalidSettings(format!("unknown key {text:?}")))
	}
}

impl Serialize for ZKeyCode {
	fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
		serializer.serialize_str(self.name())
	}
}

impl<'de> Deserialize<'de> for ZKeyCode {
	fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
		let text = String::deserialize(deserializer)?;
		text.parse().map_err(serde::de::Error::custom)
	}
}

#[cfg(test)]
mod tests {
	use super::*;

	#[test]
	fn the_list_is_the_one_of_winit() {
		// winit 0.30.13 has 194 key codes; the macro maps each of them once.
		assert_eq!(ZKeyCode::ALL.len(), 194, "one key for every winit key code");
	}

	#[test]
	fn names_read_back() {
		for key in ZKeyCode::ALL {
			assert_eq!(key.name().parse::<ZKeyCode>().ok(), Some(*key), "{key}");
			assert_eq!(
				key.winit_name().parse::<ZKeyCode>().ok(),
				Some(*key),
				"{key} by its winit name"
			);
		}
	}

	#[test]
	fn names_are_forgiving() {
		for (text, key) in [
			("w", ZKeyCode::W),
			("LeftShift", ZKeyCode::LeftShift),
			("left_shift", ZKeyCode::LeftShift),
			("LEFT-CONTROL", ZKeyCode::LeftControl),
			("ctrl", ZKeyCode::LeftControl),
			("Esc", ZKeyCode::Escape),
			("ArrowUp", ZKeyCode::Up),
			("KeyW", ZKeyCode::W),
			("ShiftLeft", ZKeyCode::LeftShift),
			("space", ZKeyCode::Space),
		] {
			assert_eq!(text.parse::<ZKeyCode>().ok(), Some(key), "{text}");
		}
		assert!("".parse::<ZKeyCode>().is_err(), "empty is not a key");
		assert!("Banana".parse::<ZKeyCode>().is_err(), "unknown is not a key");
	}

	#[test]
	fn names_are_unique() {
		let mut names: Vec<String> = ZKeyCode::ALL.iter().map(|k| normalize(k.name())).collect();
		names.sort();
		names.dedup();
		assert_eq!(names.len(), ZKeyCode::ALL.len(), "no two keys share a name");
	}
}
